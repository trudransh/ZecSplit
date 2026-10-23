//! Write-ahead SQLite store for swaps, events and transactions (PROTOCOL_SPEC §6.3, INTERFACES §4).

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use zecsplit_crypto::Role;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
CREATE TABLE IF NOT EXISTS swaps (id BLOB PRIMARY KEY, role INTEGER NOT NULL, network TEXT NOT NULL,
  state TEXT NOT NULL, quote_json TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  deadline_t1_height INTEGER NOT NULL, deadline_t2_height INTEGER NOT NULL, done INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS swaps_active ON swaps (done) WHERE done = 0;
CREATE TABLE IF NOT EXISTS events (swap_id BLOB NOT NULL, seq INTEGER NOT NULL, at INTEGER NOT NULL,
  kind TEXT NOT NULL, data_json TEXT NOT NULL, PRIMARY KEY (swap_id, seq));
CREATE TABLE IF NOT EXISTS txs (swap_id BLOB NOT NULL, name TEXT NOT NULL, raw BLOB NOT NULL, txid TEXT NOT NULL,
  broadcast_at INTEGER, conf_height INTEGER, PRIMARY KEY (swap_id, name));
CREATE TABLE IF NOT EXISTS secrets (swap_id BLOB PRIMARY KEY, ciphertext BLOB NOT NULL);
";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("unknown swap")]
    UnknownSwap,
    #[error("swap already exists")]
    DuplicateSwap,
    #[error("swap is terminal")]
    Terminal,
    #[error("tx {0} already stored with different bytes")]
    TxMismatch(String),
}

/// A new swap's fixed fields plus its first state.
#[derive(Debug, Clone)]
pub struct NewSwap<'a> {
    pub id: [u8; 32],
    pub role: Role,
    pub network: &'a str,
    pub quote_json: &'a str,
    pub t1_height: u32,
    pub t2_height: u32,
    pub state_json: &'a str,
}

/// A stored swap as loaded on resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapRow {
    pub id: [u8; 32],
    pub role: Role,
    pub network: String,
    pub quote_json: String,
    pub state_json: String,
    pub t1_height: u32,
    pub t2_height: u32,
}

/// A stored transaction and what is known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxRow {
    pub name: String,
    pub raw: Vec<u8>,
    pub txid: String,
    pub broadcast_at: Option<i64>,
}

/// The engine's or maker's durable swap state.
pub struct Store {
    conn: Connection,
}

fn role_code(role: Role) -> i64 {
    match role {
        Role::Alice => 0,
        Role::Bob => 1,
    }
}

fn role_from(code: i64) -> rusqlite::Result<Role> {
    match code {
        0 => Ok(Role::Alice),
        1 => Ok(Role::Bob),
        other => Err(rusqlite::Error::IntegralValueOutOfRange(1, other)),
    }
}

fn id_from(blob: Vec<u8>) -> rusqlite::Result<[u8; 32]> {
    let len = blob.len();
    blob.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("swap id of {len} bytes").into(),
        )
    })
}

impl Store {
    /// Opens or creates the database and applies the schema.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    /// Records a new swap and its first state in one transaction.
    pub fn create_swap(&mut self, swap: &NewSwap<'_>, now: i64) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO swaps (id, role, network, state, quote_json, created_at, updated_at,
               deadline_t1_height, deadline_t2_height) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
            params![
                &swap.id[..],
                role_code(swap.role),
                swap.network,
                swap.state_json,
                swap.quote_json,
                now,
                swap.t1_height,
                swap.t2_height
            ],
        )?;
        if inserted == 0 {
            return Err(StoreError::DuplicateSwap);
        }
        tx.execute(
            "INSERT INTO events (swap_id, seq, at, kind, data_json) VALUES (?1, 1, ?2, 'created', ?3)",
            params![&swap.id[..], now, swap.state_json],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// §6.3-R1: durably commits a transition before the side effects it permits.
    pub fn commit_transition(
        &mut self,
        id: &[u8; 32],
        state_json: &str,
        terminal: bool,
        kind: &str,
        data_json: &str,
        now: i64,
    ) -> Result<u64, StoreError> {
        let tx = self.conn.transaction()?;
        let done: Option<bool> = tx
            .query_row("SELECT done FROM swaps WHERE id = ?1", [&id[..]], |r| {
                r.get(0)
            })
            .optional()?;
        match done {
            None => return Err(StoreError::UnknownSwap),
            Some(true) => return Err(StoreError::Terminal),
            Some(false) => {}
        }
        tx.execute(
            "UPDATE swaps SET state = ?2, done = ?3, updated_at = ?4 WHERE id = ?1",
            params![&id[..], state_json, terminal, now],
        )?;
        let seq: u64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE swap_id = ?1",
            [&id[..]],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO events (swap_id, seq, at, kind, data_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id[..], seq, now, kind, data_json],
        )?;
        tx.commit()?;
        Ok(seq)
    }

    /// The last committed state of a swap.
    pub fn state(&self, id: &[u8; 32]) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT state FROM swaps WHERE id = ?1", [&id[..]], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// §6.3-R2: every non-terminal swap, oldest first, for resume on start.
    pub fn active_swaps(&self) -> Result<Vec<SwapRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, network, quote_json, state, deadline_t1_height, deadline_t2_height
             FROM swaps WHERE done = 0 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SwapRow {
                id: id_from(r.get(0)?)?,
                role: role_from(r.get(1)?)?,
                network: r.get(2)?,
                quote_json: r.get(3)?,
                state_json: r.get(4)?,
                t1_height: r.get(5)?,
                t2_height: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// §6.3-R1: stores a signed tx once; storing the same bytes again is a no-op.
    pub fn put_tx(
        &mut self,
        id: &[u8; 32],
        name: &str,
        raw: &[u8],
        txid: &str,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let known: Option<Vec<u8>> = tx
            .query_row(
                "SELECT raw FROM txs WHERE swap_id = ?1 AND name = ?2",
                params![&id[..], name],
                |r| r.get(0),
            )
            .optional()?;
        match known {
            Some(existing) if existing == raw => {}
            Some(_) => return Err(StoreError::TxMismatch(name.to_string())),
            None => {
                tx.execute(
                    "INSERT INTO txs (swap_id, name, raw, txid) VALUES (?1, ?2, ?3, ?4)",
                    params![&id[..], name, raw, txid],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Records the first broadcast of a stored tx; later calls keep the first time.
    pub fn mark_broadcast(
        &mut self,
        id: &[u8; 32],
        name: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let n = self.conn.execute(
            "UPDATE txs SET broadcast_at = COALESCE(broadcast_at, ?3) WHERE swap_id = ?1 AND name = ?2",
            params![&id[..], name, now],
        )?;
        if n == 0 {
            return Err(StoreError::UnknownSwap);
        }
        Ok(())
    }

    /// Every stored tx of a swap, by name.
    pub fn txs(&self, id: &[u8; 32]) -> Result<Vec<TxRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, raw, txid, broadcast_at FROM txs WHERE swap_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([&id[..]], |r| {
            Ok(TxRow {
                name: r.get(0)?,
                raw: r.get(1)?,
                txid: r.get(2)?,
                broadcast_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swap(id: u8) -> NewSwap<'static> {
        NewSwap {
            id: [id; 32],
            role: Role::Alice,
            network: "regtest",
            quote_json: "{}",
            t1_height: 120,
            t2_height: 140,
            state_json: "\"Quoted\"",
        }
    }

    fn open(dir: &tempfile::TempDir) -> Store {
        Store::open(&dir.path().join("swaps.sqlite")).unwrap()
    }

    #[test]
    fn spec_6_3_r1_committed_state_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(&dir);
        store.create_swap(&swap(1), 10).unwrap();
        assert_eq!(
            store
                .commit_transition(&[1; 32], "\"Committed\"", false, "step", "{}", 11)
                .unwrap(),
            2
        );
        drop(store);

        let store = open(&dir);
        assert_eq!(
            store.state(&[1; 32]).unwrap().as_deref(),
            Some("\"Committed\"")
        );
    }

    #[test]
    fn spec_6_3_r2_resume_loads_only_active_swaps() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(&dir);
        store.create_swap(&swap(1), 10).unwrap();
        store.create_swap(&swap(2), 11).unwrap();
        store
            .commit_transition(&[2; 32], "\"Done\"", true, "step", "{}", 12)
            .unwrap();

        let active = open(&dir).active_swaps().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, [1; 32]);
        assert_eq!(active[0].role, Role::Alice);
        assert_eq!(active[0].t2_height, 140);
    }

    #[test]
    fn terminal_swaps_and_unknown_ids_reject_transitions() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(&dir);
        store.create_swap(&swap(1), 10).unwrap();
        store
            .commit_transition(&[1; 32], "\"Done\"", true, "step", "{}", 11)
            .unwrap();
        assert!(matches!(
            store.commit_transition(&[1; 32], "\"Quoted\"", false, "step", "{}", 12),
            Err(StoreError::Terminal)
        ));
        assert!(matches!(
            store.commit_transition(&[9; 32], "\"Quoted\"", false, "step", "{}", 12),
            Err(StoreError::UnknownSwap)
        ));
        assert!(matches!(
            store.create_swap(&swap(1), 13),
            Err(StoreError::DuplicateSwap)
        ));
    }

    #[test]
    fn spec_6_3_r1_tx_store_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(&dir);
        store.create_swap(&swap(1), 10).unwrap();
        store.put_tx(&[1; 32], "redeem", b"raw", "ab").unwrap();
        store.put_tx(&[1; 32], "redeem", b"raw", "ab").unwrap();
        assert!(matches!(
            store.put_tx(&[1; 32], "redeem", b"other", "cd"),
            Err(StoreError::TxMismatch(_))
        ));
        store.mark_broadcast(&[1; 32], "redeem", 20).unwrap();
        store.mark_broadcast(&[1; 32], "redeem", 30).unwrap();

        let txs = store.txs(&[1; 32]).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].broadcast_at, Some(20));
    }
}

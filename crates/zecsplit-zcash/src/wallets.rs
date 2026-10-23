//! One scanning wallet per swap lock, keyed by `ak`, on a light server (§4.3, §4.5-R3, §5.3-R3).

use std::path::PathBuf;

use zcash_client_backend::data_api::WalletRead;
use zcash_client_backend::proto::service::ChainSpec;
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_protocol::TxId;
use zcash_protocol::consensus::Parameters;
use zecsplit_crypto::Ask;

use crate::lock::lock_keys;
use crate::sweep::{SweepError, sweep};
use crate::watch::{Client, LockStatus, WatchError, lock_status, synced_wallet, wallet_err};

/// Blocks before the first scan a fresh lock wallet starts from; covers a lock paid before its wallet existed.
const BIRTHDAY_LOOKBACK: u32 = 1000;

/// The lock's `ak ‖ nk ‖ rivk`.
pub struct LockKey<'a> {
    pub ak: &'a [u8; 32],
    pub nk: &'a [u8; 32],
    pub rivk: &'a [u8; 32],
}

/// Lock wallets under `dir`, scanned through `server`, sweeping to `payout`.
pub struct LockWallets<P> {
    params: P,
    dir: PathBuf,
    server: String,
    payout: String,
}

impl<P: Parameters + Clone + Send + Sync + 'static> LockWallets<P> {
    pub fn new(params: P, dir: PathBuf, server: &str, payout: &str) -> Self {
        LockWallets {
            params,
            dir,
            server: server.to_string(),
            payout: payout.to_string(),
        }
    }

    async fn client(&self) -> Result<Client, WatchError> {
        CompactTxStreamerClient::connect(self.server.clone())
            .await
            .map_err(|e| WatchError::Server(e.to_string()))
    }

    async fn tip(&self) -> Result<u32, WatchError> {
        let block = self
            .client()
            .await?
            .get_latest_block(ChainSpec {})
            .await
            .map_err(|e| WatchError::Server(e.to_string()))?
            .into_inner();
        u32::try_from(block.height).map_err(|e| WatchError::Server(e.to_string()))
    }

    /// The wallet path, UFVK and birthday for `key`; the birthday only matters on first use.
    async fn open(&self, key: &LockKey<'_>) -> Result<(PathBuf, String, u32), WatchError> {
        let keys = lock_keys(&self.params, key.ak, key.nk, key.rivk)
            .map_err(|e| WatchError::Wallet(e.to_string()))?;
        let db = self.dir.join(format!("{}.sqlite", hex::encode(key.ak)));
        let birthday = if db.exists() {
            1
        } else {
            self.tip().await?.saturating_sub(BIRTHDAY_LOOKBACK).max(2)
        };
        Ok((db, keys.ufvk, birthday))
    }

    /// §4.3-R2: what the lock address holds, counting notes with at least `min_conf` confirmations.
    pub async fn status(&self, key: &LockKey<'_>, min_conf: u32) -> Result<LockStatus, WatchError> {
        let (db, ufvk, birthday) = self.open(key).await?;
        lock_status(&self.params, &db, &ufvk, birthday, &self.server, min_conf).await
    }

    /// §4.5-R3, §5.3-R3: sweeps every lock note to the payout address; returns the txid.
    pub async fn sweep(&self, key: &LockKey<'_>, ask: &Ask) -> Result<String, SweepError> {
        let (db, ufvk, birthday) = self.open(key).await?;
        let txid = sweep(
            &self.params,
            &db,
            &ufvk,
            birthday,
            &self.server,
            ask,
            &self.payout,
        )
        .await?;
        Ok(txid.to_string())
    }

    /// Confirmations of a sweep of `key`'s lock, from a local scan, never `GetTransaction` (PRIV-05).
    pub async fn confirmations(
        &self,
        key: &LockKey<'_>,
        txid: &str,
    ) -> Result<Option<u32>, WatchError> {
        let id = TxId::from_hex(txid).ok_or_else(|| WatchError::Wallet("bad txid".into()))?;
        let (db, ufvk, birthday) = self.open(key).await?;
        let (_, db) = synced_wallet(&self.params, &db, &ufvk, birthday, &self.server).await?;
        let mined = db.get_tx_height(id).map_err(wallet_err)?;
        let tip = db.chain_height().map_err(wallet_err)?;
        Ok(mined
            .zip(tip)
            .map(|(h, t)| u32::from(t).saturating_sub(u32::from(h)) + 1))
    }
}

//! Lock detection (PROTOCOL_SPEC §4.3, T-011).

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use zcash_client_backend::data_api::chain::{BlockCache, BlockSource, error as chain_error};
use zcash_client_backend::data_api::scanning::ScanRange;
use zcash_client_backend::data_api::wallet::ConfirmationsPolicy;
use zcash_client_backend::data_api::{AccountBirthday, AccountPurpose, WalletRead, WalletWrite};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::BlockId;
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_client_backend::sync;
use zcash_client_backend::util::SystemClock;
use zcash_client_sqlite::WalletDb;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::{BlockHeight, Parameters};

/// The lock address as of the latest scan (INTERFACES §1.2 `LockStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockStatus {
    pub total: u64,
    pub confirmed: u64,
    pub tip: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("light server: {0}")]
    Server(String),
    #[error("wallet database: {0}")]
    Wallet(String),
    #[error("lock UFVK does not decode for this network")]
    BadUfvk,
    #[error("min_conf must be at least 1")]
    ZeroConfirmations,
}

fn wallet_err(e: impl std::fmt::Display) -> WatchError {
    WatchError::Wallet(e.to_string())
}

/// §4.3-R1/R2: scan with the lock UFVK and report what the lock address holds.
pub async fn lock_status<P: Parameters + Clone + Send + 'static>(
    params: &P,
    db_path: &Path,
    ufvk: &str,
    birthday_height: u32,
    server: &str,
    min_conf: u32,
) -> Result<LockStatus, WatchError> {
    let min_conf = NonZeroU32::new(min_conf).ok_or(WatchError::ZeroConfirmations)?;
    let mut client = CompactTxStreamerClient::connect(server.to_string())
        .await
        .map_err(|e| WatchError::Server(e.to_string()))?;
    let rng = getrandom::rand_core::UnwrapErr(getrandom::SysRng);
    let mut db =
        WalletDb::for_path(db_path, params.clone(), SystemClock, rng).map_err(wallet_err)?;
    zcash_client_sqlite::wallet::init::init_wallet_db(&mut db, None).map_err(wallet_err)?;

    if db.get_account_ids().map_err(wallet_err)?.is_empty() {
        let ufvk = UnifiedFullViewingKey::decode(params, ufvk).map_err(|_| WatchError::BadUfvk)?;
        let tree_state = client
            .get_tree_state(BlockId {
                height: u64::from(birthday_height.saturating_sub(1)),
                hash: vec![],
            })
            .await
            .map_err(|e| WatchError::Server(e.to_string()))?
            .into_inner();
        let birthday = AccountBirthday::from_treestate(tree_state, None)
            .map_err(|_| WatchError::Server("bad tree state".into()))?;
        db.import_account_ufvk(
            "lock",
            &ufvk,
            &birthday,
            AccountPurpose::Spending { derivation: None },
            None,
        )
        .map_err(wallet_err)?;
    }

    let cache = MemBlockCache::default();
    sync::run(&mut client, params, &cache, &mut db, 100)
        .await
        .map_err(|e| WatchError::Server(e.to_string()))?;

    let policy = ConfirmationsPolicy::new_symmetrical(min_conf, false);
    let Some(summary) = db.get_wallet_summary(policy).map_err(wallet_err)? else {
        return Ok(LockStatus {
            total: 0,
            confirmed: 0,
            tip: 0,
        });
    };
    let (mut total, mut confirmed) = (0u64, 0u64);
    for balance in summary.account_balances().values() {
        for pool in [balance.ironwood_balance(), balance.orchard_balance()] {
            total += u64::from(pool.total());
            confirmed += u64::from(pool.spendable_value());
        }
    }
    Ok(LockStatus {
        total,
        confirmed,
        tip: u32::from(summary.chain_tip_height()),
    })
}

/// The smallest `BlockCache`: blocks live in memory only for the duration of one scan.
#[derive(Default)]
struct MemBlockCache {
    blocks: Mutex<Vec<CompactBlock>>,
}

#[derive(Debug, thiserror::Error)]
#[error("block cache lock poisoned")]
struct CacheError;

fn in_range(block: &CompactBlock, range: &ScanRange) -> bool {
    u32::try_from(block.height)
        .map(|h| range.block_range().contains(&BlockHeight::from_u32(h)))
        .unwrap_or(false)
}

impl BlockSource for MemBlockCache {
    type Error = CacheError;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), chain_error::Error<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), chain_error::Error<WalletErrT, Self::Error>>,
    {
        let mut blocks = self
            .blocks
            .lock()
            .map_err(|_| chain_error::Error::BlockSource(CacheError))?
            .clone();
        blocks.sort_by_key(|b| b.height);
        let from = from_height.map_or(0, u64::from);
        for block in blocks
            .into_iter()
            .filter(|b| b.height >= from)
            .take(limit.unwrap_or(usize::MAX))
        {
            with_block(block)?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemBlockCache {
    fn get_tip_height(
        &self,
        range: Option<&ScanRange>,
    ) -> Result<Option<BlockHeight>, Self::Error> {
        let blocks = self.blocks.lock().map_err(|_| CacheError)?;
        Ok(blocks
            .iter()
            .filter(|b| range.is_none_or(|r| in_range(b, r)))
            .filter_map(|b| u32::try_from(b.height).ok())
            .max()
            .map(BlockHeight::from_u32))
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, Self::Error> {
        let blocks = self.blocks.lock().map_err(|_| CacheError)?;
        Ok(blocks
            .iter()
            .filter(|b| in_range(b, range))
            .cloned()
            .collect())
    }

    async fn insert(&self, mut compact_blocks: Vec<CompactBlock>) -> Result<(), Self::Error> {
        self.blocks
            .lock()
            .map_err(|_| CacheError)?
            .append(&mut compact_blocks);
        Ok(())
    }

    async fn delete(&self, range: ScanRange) -> Result<(), Self::Error> {
        self.blocks
            .lock()
            .map_err(|_| CacheError)?
            .retain(|b| !in_range(b, &range));
        Ok(())
    }
}

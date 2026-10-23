//! Real ports: the Zcash lock wallets behind `ZecSweep`.

use async_trait::async_trait;
use zcash_protocol::consensus::Parameters;
use zecsplit_crypto::Ask;
use zecsplit_protocol::session::Shared;
use zecsplit_zcash::LockWallets;

use crate::runner::ZecSweep;

#[async_trait]
impl<P: Parameters + Clone + Send + Sync + 'static> ZecSweep for LockWallets<P> {
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String> {
        LockWallets::sweep(self, &shared.lock_key(), ask)
            .await
            .map_err(|e| e.to_string())
    }

    async fn confirmations(&self, shared: &Shared, txid: &str) -> Result<Option<u32>, String> {
        LockWallets::confirmations(self, &shared.lock_key(), txid)
            .await
            .map_err(|e| e.to_string())
    }
}

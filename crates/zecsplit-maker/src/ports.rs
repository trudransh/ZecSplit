//! Real ports: Bob's bitcoind wallet and the Zcash lock wallets on his own node.

use async_trait::async_trait;
use bitcoin::consensus::encode::deserialize_hex;
use bitcoin::{Address, Amount, Psbt, ScriptBuf, Transaction};
use serde_json::{Value, json};
use zcash_protocol::consensus::Parameters;
use zecsplit_btc::sources::BitcoindRpc;
use zecsplit_crypto::Ask;
use zecsplit_protocol::machine::ZecLock;
use zecsplit_protocol::session::Shared;
use zecsplit_zcash::{LockStatus, LockWallets};

use crate::runner::{Wallet, ZecNode};

/// Bob's wallet in bitcoind, reached through its `/wallet/<name>` RPC endpoint.
pub struct BitcoindWallet {
    rpc: BitcoindRpc,
    network: bitcoin::Network,
}

impl BitcoindWallet {
    pub fn new(rpc: BitcoindRpc, network: bitcoin::Network) -> Self {
        BitcoindWallet { rpc, network }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.rpc
            .call(method, params)
            .await
            .map_err(|e| format!("{method}: {e}"))
    }
}

#[async_trait]
impl Wallet for BitcoindWallet {
    async fn refund_address(&self) -> Result<String, String> {
        let addr = self.call("getnewaddress", json!(["", "bech32"])).await?;
        addr.as_str()
            .map(str::to_string)
            .ok_or_else(|| "getnewaddress: not a string".into())
    }

    async fn fund(&self, spk: &ScriptBuf, value: Amount) -> Result<Psbt, String> {
        let addr = Address::from_script(spk, self.network).map_err(|e| e.to_string())?;
        let funded = self
            .call(
                "walletcreatefundedpsbt",
                json!([[], [{ addr.to_string(): value.to_btc() }], 0, { "change_type": "bech32" }]),
            )
            .await?;
        funded["psbt"]
            .as_str()
            .ok_or("walletcreatefundedpsbt: no psbt")?
            .parse()
            .map_err(|e| format!("walletcreatefundedpsbt: {e}"))
    }

    async fn sign(&self, psbt: &Psbt) -> Result<Transaction, String> {
        let signed = self
            .call("walletprocesspsbt", json!([psbt.to_string()]))
            .await?;
        if signed["complete"] != true {
            return Err("walletprocesspsbt: incomplete".into());
        }
        let hex = signed["hex"].as_str().ok_or("walletprocesspsbt: no hex")?;
        deserialize_hex(hex).map_err(|e| e.to_string())
    }
}

/// §4.3-R2, §5.5: the lock counts only when notes sum to exactly `zatoshis`, all with `min_conf` confirmations.
pub fn zec_lock(seen: LockStatus, zatoshis: u64, min_conf: u32) -> ZecLock {
    if seen.total == 0 {
        ZecLock::Unseen
    } else if seen.total != zatoshis {
        ZecLock::WrongAmount
    } else if seen.confirmed == zatoshis {
        ZecLock::Correct {
            confirmations: min_conf,
        }
    } else {
        ZecLock::Correct { confirmations: 0 }
    }
}

#[async_trait]
impl<P: Parameters + Clone + Send + Sync + 'static> ZecNode for LockWallets<P> {
    async fn lock(&self, shared: &Shared, zatoshis: u64, min_conf: u32) -> Result<ZecLock, String> {
        let seen = self
            .status(&shared.lock_key(), min_conf)
            .await
            .map_err(|e| e.to_string())?;
        Ok(zec_lock(seen, zatoshis, min_conf))
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(total: u64, confirmed: u64) -> LockStatus {
        LockStatus {
            total,
            confirmed,
            tip: 100,
        }
    }

    #[test]
    fn spec_4_3_r2_lock_counts_only_when_enough_is_confirmed() {
        assert_eq!(zec_lock(seen(0, 0), 10, 5), ZecLock::Unseen);
        assert_eq!(
            zec_lock(seen(10, 4), 10, 5),
            ZecLock::Correct { confirmations: 0 }
        );
        assert_eq!(
            zec_lock(seen(10, 10), 10, 5),
            ZecLock::Correct { confirmations: 5 }
        );
    }

    #[test]
    fn spec_5_5_underpay_and_overpay_are_wrong_amounts_adv_05() {
        assert_eq!(zec_lock(seen(4, 4), 10, 5), ZecLock::WrongAmount);
        assert_eq!(zec_lock(seen(12, 12), 10, 5), ZecLock::WrongAmount);
        assert_eq!(zec_lock(seen(11, 10), 10, 5), ZecLock::WrongAmount);
    }
}

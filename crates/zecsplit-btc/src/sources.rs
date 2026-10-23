//! Bitcoin data sources and the two-source agreement check (PROTOCOL_SPEC §4.2-R1, T-020).

use std::time::Duration;

use async_trait::async_trait;
use bitcoin::consensus::encode::{deserialize_hex, serialize_hex};
use bitcoin::{ScriptBuf, Transaction, TxOut, Txid};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::txs::SwapTxs;

const TIMEOUT: Duration = Duration::from_secs(30);
const RPC_NOT_FOUND: i64 = -5;

/// A transaction as one source reports it; `height` is `None` while unconfirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTx {
    pub tx: Transaction,
    pub height: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("rpc {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("§4.2-R1 needs at least two sources")]
    TooFewSources,
}

fn bad(e: impl std::fmt::Display) -> SourceError {
    SourceError::BadResponse(e.to_string())
}

/// One view of the Bitcoin chain (INTERFACES §1.3).
#[async_trait]
pub trait BtcSource: Send + Sync {
    /// Height of the best block.
    async fn tip(&self) -> Result<u32, SourceError>;
    /// The transaction with this txid, or `None` if the source has never seen it.
    async fn tx(&self, txid: &Txid) -> Result<Option<SourceTx>, SourceError>;
}

/// What all sources agree on about one transaction; mirrors the protocol's `TxStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    Unseen,
    Mempool,
    Confirmed {
        height: u32,
    },
    /// `BTC_SOURCES_DISAGREE` (THREAT_MODEL T3, ADV-03).
    Disagree,
}

/// §4.2-R1: the lowest tip across all sources.
pub async fn tip_on_all(sources: &[&dyn BtcSource]) -> Result<u32, SourceError> {
    if sources.len() < 2 {
        return Err(SourceError::TooFewSources);
    }
    let mut tip = u32::MAX;
    for s in sources {
        tip = tip.min(s.tip().await?);
    }
    Ok(tip)
}

/// §4.2-R1: every source must report `txid` with an output equal to `expect`.
pub async fn confirmed_on_all(
    sources: &[&dyn BtcSource],
    txid: &Txid,
    expect: &TxOut,
) -> Result<Agreement, SourceError> {
    if sources.len() < 2 {
        return Err(SourceError::TooFewSources);
    }
    let mut views = Vec::with_capacity(sources.len());
    for s in sources {
        let view = s.tx(txid).await?;
        if let Some(seen) = &view
            && (seen.tx.compute_txid() != *txid || !seen.tx.output.contains(expect))
        {
            return Ok(Agreement::Disagree);
        }
        views.push(view.map(|v| v.height));
    }
    Ok(agree(&views))
}

/// §6.3-R1: idempotent broadcast to every source (T-023).
#[async_trait]
pub trait Broadcast: Send + Sync {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError>;
}

/// §6.3-R1: a rejected broadcast is a no-op when the source already has the transaction
/// (in its mempool or a block); any other rejection, such as a conflicting spend, stands.
async fn known_or(
    src: &dyn BtcSource,
    tx: &Transaction,
    sent: Result<(), SourceError>,
) -> Result<(), SourceError> {
    let Err(e) = sent else { return Ok(()) };
    match src.tx(&tx.compute_txid()).await {
        Ok(Some(_)) => Ok(()),
        _ => Err(e),
    }
}

/// What all sources agree on about the tip and a swap's five transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwapStatus {
    pub tip: u32,
    pub lock: Agreement,
    pub redeem: Agreement,
    pub cancel: Agreement,
    pub refund: Agreement,
    pub punish: Agreement,
}

/// §4.2-R1: the lowest tip and each swap transaction as every source sees it.
pub async fn swap_status(
    sources: &[&dyn BtcSource],
    txs: &SwapTxs,
) -> Result<SwapStatus, SourceError> {
    let lock_out = TxOut {
        value: txs.v_lock,
        script_pubkey: ScriptBuf::new_p2wsh(&txs.w_script.wscript_hash()),
    };
    let spend = |tx: &Transaction| (tx.compute_txid(), tx.output[0].clone());
    let watched = [
        (txs.lock.unsigned_tx.compute_txid(), lock_out),
        spend(&txs.redeem),
        spend(&txs.cancel),
        spend(&txs.refund),
        spend(&txs.punish),
    ];
    let mut seen = [Agreement::Unseen; 5];
    for (slot, (txid, out)) in seen.iter_mut().zip(&watched) {
        *slot = confirmed_on_all(sources, txid, out).await?;
    }
    let [lock, redeem, cancel, refund, punish] = seen;
    Ok(SwapStatus {
        tip: tip_on_all(sources).await?,
        lock,
        redeem,
        cancel,
        refund,
        punish,
    })
}

/// The transaction as the first source that has it reports it.
pub async fn fetch_any(
    sources: &[&dyn BtcSource],
    txid: &Txid,
) -> Result<Option<Transaction>, SourceError> {
    for s in sources {
        if let Some(seen) = s.tx(txid).await? {
            return Ok(Some(seen.tx));
        }
    }
    Ok(None)
}

/// Combines per-source views, always taking the weakest claim; only a confirmation one
/// source has never heard of is a disagreement.
fn agree(views: &[Option<Option<u32>>]) -> Agreement {
    let confirmed = views.iter().filter(|v| matches!(v, Some(Some(_)))).count();
    if views.contains(&None) {
        return if confirmed > 0 {
            Agreement::Disagree
        } else {
            Agreement::Unseen
        };
    }
    if confirmed < views.len() {
        return Agreement::Mempool;
    }
    let height = views.iter().flatten().flatten().copied().max();
    height.map_or(Agreement::Unseen, |height| Agreement::Confirmed { height })
}

/// bitcoind JSON-RPC; needs `-txindex` to see transactions outside its wallet.
pub struct BitcoindRpc {
    http: reqwest::Client,
    url: String,
    user: String,
    pass: String,
}

#[derive(Deserialize)]
struct RpcReply {
    result: Value,
    error: Option<RpcErr>,
}

#[derive(Deserialize)]
struct RpcErr {
    code: i64,
    message: String,
}

impl BitcoindRpc {
    pub fn new(url: &str, user: &str, pass: &str) -> Result<Self, SourceError> {
        Ok(Self {
            http: reqwest::Client::builder().timeout(TIMEOUT).build()?,
            url: url.to_string(),
            user: user.to_string(),
            pass: pass.to_string(),
        })
    }

    /// One JSON-RPC call.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, SourceError> {
        let res = self
            .http
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.pass))
            .json(&json!({"jsonrpc": "1.0", "id": 0, "method": method, "params": params}))
            .send()
            .await?;
        let status = res.status();
        let reply: RpcReply = res
            .json()
            .await
            .map_err(|_| bad(format!("{method}: http {status}")))?;
        match reply.error {
            Some(e) => Err(SourceError::Rpc {
                code: e.code,
                message: e.message,
            }),
            None => Ok(reply.result),
        }
    }
}

#[async_trait]
impl BtcSource for BitcoindRpc {
    async fn tip(&self) -> Result<u32, SourceError> {
        let n = self.call("getblockcount", json!([])).await?;
        n.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| bad(n))
    }

    async fn tx(&self, txid: &Txid) -> Result<Option<SourceTx>, SourceError> {
        let v = match self
            .call("getrawtransaction", json!([txid.to_string(), 1]))
            .await
        {
            Err(SourceError::Rpc { code, .. }) if code == RPC_NOT_FOUND => return Ok(None),
            r => r?,
        };
        let hex = v["hex"]
            .as_str()
            .ok_or_else(|| bad("getrawtransaction: no hex"))?;
        let tx = deserialize_hex(hex).map_err(bad)?;
        let height = match v["blockhash"].as_str() {
            None => None,
            Some(hash) => {
                let h = self.call("getblockheader", json!([hash])).await?;
                // A block off the active chain reports -1 confirmations.
                match h["confirmations"].as_i64() {
                    Some(c) if c > 0 => h["height"].as_u64().and_then(|n| u32::try_from(n).ok()),
                    _ => None,
                }
            }
        };
        Ok(Some(SourceTx { tx, height }))
    }
}

#[async_trait]
impl Broadcast for BitcoindRpc {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        let sent = self
            .call("sendrawtransaction", json!([serialize_hex(tx)]))
            .await
            .map(drop);
        known_or(self, tx, sent).await
    }
}

/// An Esplora REST endpoint (plain HTTP until T-025 adds TLS).
pub struct Esplora {
    http: reqwest::Client,
    base: String,
}

#[derive(Deserialize)]
struct EsploraStatus {
    confirmed: bool,
    block_height: Option<u32>,
}

impl Esplora {
    pub fn new(base: &str) -> Result<Self, SourceError> {
        Ok(Self {
            http: reqwest::Client::builder().timeout(TIMEOUT).build()?,
            base: base.trim_end_matches('/').to_string(),
        })
    }

    async fn get(&self, path: &str) -> Result<Option<reqwest::Response>, SourceError> {
        let res = self.http.get(format!("{}{path}", self.base)).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(res.error_for_status()?))
    }
}

#[async_trait]
impl BtcSource for Esplora {
    async fn tip(&self) -> Result<u32, SourceError> {
        let res = self.get("/blocks/tip/height").await?;
        let body = res.ok_or_else(|| bad("no tip"))?.text().await?;
        body.trim().parse().map_err(bad)
    }

    async fn tx(&self, txid: &Txid) -> Result<Option<SourceTx>, SourceError> {
        let Some(res) = self.get(&format!("/tx/{txid}/hex")).await? else {
            return Ok(None);
        };
        let tx = deserialize_hex(res.text().await?.trim()).map_err(bad)?;
        let res = self.get(&format!("/tx/{txid}/status")).await?;
        let status: EsploraStatus = res.ok_or_else(|| bad("no status"))?.json().await?;
        let height = status.block_height.filter(|_| status.confirmed);
        Ok(Some(SourceTx { tx, height }))
    }
}

#[async_trait]
impl Broadcast for Esplora {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        let sent = async {
            let res = self
                .http
                .post(format!("{}/tx", self.base))
                .body(serialize_hex(tx))
                .send()
                .await?;
            if res.status().is_success() {
                Ok(())
            } else {
                Err(bad(res.text().await?))
            }
        }
        .await;
        known_or(self, tx, sent).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{Amount, ScriptBuf};

    struct Fake {
        tip: u32,
        tx: Option<SourceTx>,
    }

    #[async_trait]
    impl BtcSource for Fake {
        async fn tip(&self) -> Result<u32, SourceError> {
            Ok(self.tip)
        }
        async fn tx(&self, _: &Txid) -> Result<Option<SourceTx>, SourceError> {
            Ok(self.tx.clone())
        }
    }

    fn lock_out() -> TxOut {
        TxOut {
            value: Amount::from_sat(100_000),
            script_pubkey: ScriptBuf::new_p2wsh(&ScriptBuf::new().wscript_hash()),
        }
    }

    fn lock_tx(out: TxOut) -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![out],
        }
    }

    fn fake(tx: Option<Transaction>, height: Option<u32>) -> Fake {
        Fake {
            tip: 200,
            tx: tx.map(|tx| SourceTx { tx, height }),
        }
    }

    async fn check(a: Fake, b: Fake) -> Agreement {
        let txid = lock_tx(lock_out()).compute_txid();
        confirmed_on_all(&[&a, &b], &txid, &lock_out())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn spec_4_2_r1_agrees_when_both_confirm() {
        let tx = Some(lock_tx(lock_out()));
        let got = check(fake(tx.clone(), Some(150)), fake(tx, Some(150))).await;
        assert_eq!(got, Agreement::Confirmed { height: 150 });
    }

    #[tokio::test]
    async fn adv_03_confirmed_on_one_unseen_on_other_disagrees() {
        let got = check(fake(Some(lock_tx(lock_out())), Some(150)), fake(None, None)).await;
        assert_eq!(got, Agreement::Disagree);
    }

    #[tokio::test]
    async fn adv_03_different_output_disagrees() {
        let mut wrong = lock_out();
        wrong.value = Amount::from_sat(1);
        let got = check(
            fake(Some(lock_tx(lock_out())), Some(150)),
            fake(Some(lock_tx(wrong)), Some(150)),
        )
        .await;
        assert_eq!(got, Agreement::Disagree);
    }

    #[tokio::test]
    async fn adv_03_tx_with_other_txid_disagrees() {
        let mut other = lock_tx(lock_out());
        other.lock_time = LockTime::from_height(1).unwrap();
        let got = check(
            fake(Some(lock_tx(lock_out())), Some(150)),
            fake(Some(other), Some(150)),
        )
        .await;
        assert_eq!(got, Agreement::Disagree);
    }

    #[tokio::test]
    async fn spec_4_2_r1_needs_two_sources() {
        let a = fake(None, None);
        let txid = lock_tx(lock_out()).compute_txid();
        assert!(matches!(
            confirmed_on_all(&[&a], &txid, &lock_out()).await,
            Err(SourceError::TooFewSources)
        ));
        assert!(matches!(
            tip_on_all(&[&a]).await,
            Err(SourceError::TooFewSources)
        ));
    }

    #[tokio::test]
    async fn tip_on_all_takes_the_lowest() {
        let (a, mut b) = (fake(None, None), fake(None, None));
        b.tip = 190;
        assert_eq!(tip_on_all(&[&a, &b]).await.unwrap(), 190);
    }

    #[test]
    fn agree_takes_the_weakest_claim() {
        assert_eq!(agree(&[None, None]), Agreement::Unseen);
        assert_eq!(agree(&[Some(None), None]), Agreement::Unseen);
        assert_eq!(agree(&[Some(None), Some(None)]), Agreement::Mempool);
        assert_eq!(agree(&[Some(Some(150)), Some(None)]), Agreement::Mempool);
        assert_eq!(
            agree(&[Some(Some(150)), Some(Some(151))]),
            Agreement::Confirmed { height: 151 }
        );
    }
}

//! A finished swap's public record, and the checks anyone can run on it against the chain.

use bitcoin::Transaction;
use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use zecsplit_btc::adaptor::encsig_from_bytes;
use zecsplit_crypto::{Commitment, Role, commit, verify_reveal};

use crate::messages::{CommitBody, Quote, RedeemEncSig, RevealBody};
use crate::recovery::RecoveryBundle;
use crate::session::{recover_share, script_for};

/// Everything public about a swap that ended in `tx_redeem`: no share, key or passphrase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicRecord {
    pub quote: Quote,
    pub alice_commit: CommitBody,
    pub bob_commit: CommitBody,
    pub alice_reveal: RevealBody,
    pub bob_reveal: RevealBody,
    pub esig_redeem: RedeemEncSig,
    pub redeem_txid: String,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("not Alice's session")]
    NotAlice,
    #[error("the session has no {0}")]
    Missing(&'static str),
    #[error("{0}")]
    Parse(String),
    #[error("{0}'s reveal fails §3.2-R3: {1}")]
    Reveal(&'static str, String),
    #[error("tx_redeem is not {0}")]
    WrongTx(String),
    #[error("tx_redeem does not pay Alice's payout address")]
    Payout,
    #[error("tx_redeem does not yield a share matching Alice's committed S^P")]
    Recover,
}

fn blob<T: serde::de::DeserializeOwned>(
    b: &RecoveryBundle,
    name: &'static str,
) -> Result<T, VerifyError> {
    serde_json::from_slice(b.blob(name).ok_or(VerifyError::Missing(name))?)
        .map_err(|e| VerifyError::Parse(format!("{name}: {e}")))
}

/// The public record of Alice's sealed session once it holds `esig_B(tx_redeem)`.
pub fn record_from_alice(
    b: &RecoveryBundle,
    redeem_txid: &str,
) -> Result<PublicRecord, VerifyError> {
    if b.role != Role::Alice {
        return Err(VerifyError::NotAlice);
    }
    let quote: Quote =
        serde_json::from_str(&b.quote_json).map_err(|e| VerifyError::Parse(e.to_string()))?;
    let alice_reveal: RevealBody = blob(b, "own_reveal")?;
    let r = &alice_reveal.0;
    let c_a = commit(&b.swap_id, Role::Alice, &r.share_pub, &r.btc_key, &r.nonce);
    Ok(PublicRecord {
        quote,
        alice_commit: CommitBody { commitment: c_a.0 },
        bob_commit: blob(b, "peer_commit")?,
        bob_reveal: blob(b, "peer_reveal")?,
        esig_redeem: blob(b, "encsig")?,
        alice_reveal,
        redeem_txid: redeem_txid.to_string(),
    })
}

fn key(bytes: &[u8]) -> Result<PublicKey, VerifyError> {
    PublicKey::from_slice(bytes).map_err(|e| VerifyError::Parse(e.to_string()))
}

/// §3.2-R3 on both reveals (commitments, cross-group DLEq, proofs of knowledge), then §4.5-R1:
/// `redeem` pays Alice and Bob's signature in it yields the share behind Alice's `S^P`.
pub fn verify(r: &PublicRecord, redeem: &Transaction) -> Result<(), VerifyError> {
    let id = &r.quote.swap_id;
    let (a, b) = (&r.alice_reveal.0, &r.bob_reveal.0);
    let (c_a, c_b) = (
        Commitment(r.alice_commit.commitment),
        Commitment(r.bob_commit.commitment),
    );
    verify_reveal(id, Role::Alice, &c_a, a, &c_a, &c_b, &b.share_pub)
        .map_err(|e| VerifyError::Reveal("Alice", e.to_string()))?;
    verify_reveal(id, Role::Bob, &c_b, b, &c_a, &c_b, &a.share_pub)
        .map_err(|e| VerifyError::Reveal("Bob", e.to_string()))?;

    if redeem.compute_txid().to_string() != r.redeem_txid {
        return Err(VerifyError::WrongTx(r.redeem_txid.clone()));
    }
    let payout = script_for(&a.payout_addr, r.quote.network)
        .map_err(|e| VerifyError::Parse(e.to_string()))?;
    if redeem.output.first().map(|o| &o.script_pubkey) != Some(&payout) {
        return Err(VerifyError::Payout);
    }
    let esig = encsig_from_bytes(&r.esig_redeem.esig_redeem)
        .ok_or(VerifyError::Parse("esig_redeem".into()))?;
    recover_share(
        redeem,
        &key(&b.btc_key)?,
        &key(&a.btc_key)?,
        &key(&a.share_pub.secp)?,
        &esig,
        &a.share_pub,
        "tx_redeem",
    )
    .map(drop)
    .map_err(|_| VerifyError::Recover)
}

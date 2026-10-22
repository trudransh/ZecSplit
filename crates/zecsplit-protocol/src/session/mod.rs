//! Per-swap cryptographic sessions that turn wire messages into keys, transactions and signatures.

pub mod alice;
pub mod bob;

use std::str::FromStr;

use bitcoin::secp256k1::{PublicKey, SecretKey};
use bitcoin::{Address, Amount, ScriptBuf, Transaction};
use zecsplit_btc::adaptor::EncSig;
use zecsplit_btc::sign::{sig_in_witness, sighash};
use zecsplit_btc::txs::{BtcParams, SwapTxs};
use zecsplit_crypto::rand_core::{CryptoRng, RngCore};
use zecsplit_crypto::{
    Ask, Commitment, KeyMaterial, Reveal, RevealError, Role, Share, SharePub, Sigma,
    TranscriptInput, check_share_matches, derive_nk_rivk, generate, quote_digest, shared_ak,
    shared_ask, transcript, verify_reveal,
};
use zeroize::Zeroizing;

use crate::messages::{CommitBody, Network, PROTOCOL_VERSION, Quote, RevealBody};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("message out of order: {0}")]
    OutOfOrder(&'static str),
    #[error("peer reveal rejected: {0}")]
    Reveal(#[from] RevealError),
    #[error("bad bitcoin address: {0}")]
    Address(String),
    #[error("transaction template rejected: {0}")]
    Template(String),
    #[error("signature from peer does not verify: {0}")]
    BadSignature(&'static str),
    #[error("could not recover the peer's share from {0}")]
    Recover(&'static str),
    #[error("key error: {0}")]
    Key(String),
}

/// The Zcash side every party derives after both reveals (§3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shared {
    pub ak: [u8; 32],
    pub sigma: Sigma,
    pub transcript: [u8; 32],
    pub nk: [u8; 32],
    pub rivk: [u8; 32],
}

/// Commit-reveal and key state common to Alice and Bob.
pub struct Core {
    pub quote: Quote,
    pub role: Role,
    keys: KeyMaterial,
    payout_addr: String,
    commitment: Commitment,
    peer_commitment: Option<Commitment>,
    own_reveal: Option<Reveal>,
    peer: Option<Reveal>,
    shared: Option<Shared>,
}

fn crypto_network(n: Network) -> zecsplit_crypto::Network {
    match n {
        Network::Regtest => zecsplit_crypto::Network::Regtest,
        Network::Test => zecsplit_crypto::Network::Test,
        Network::MainBeta => zecsplit_crypto::Network::MainBeta,
    }
}

/// The Bitcoin network a quote's `network` runs on (§2).
pub fn btc_network(n: Network) -> bitcoin::Network {
    match n {
        Network::Regtest => bitcoin::Network::Regtest,
        Network::Test => bitcoin::Network::Signet,
        Network::MainBeta => bitcoin::Network::Bitcoin,
    }
}

/// A payout or refund address checked against the swap's Bitcoin network.
pub fn script_for(addr: &str, network: Network) -> Result<ScriptBuf, SessionError> {
    Address::from_str(addr)
        .map_err(|e| SessionError::Address(e.to_string()))?
        .require_network(btc_network(network))
        .map(|a| a.script_pubkey())
        .map_err(|e| SessionError::Address(e.to_string()))
}

/// The secp256k1 view of a share: the same integer, big-endian.
fn share_secp(share: &Share) -> Result<SecretKey, SessionError> {
    let mut be = Zeroizing::new(*share.expose_secret());
    be.reverse();
    SecretKey::from_slice(&be[..])
        .map_err(|_| SessionError::Key("share is not a secp256k1 scalar".into()))
}

/// The share whose secp256k1 view is `sk`.
fn share_from_secp(sk: &SecretKey) -> Share {
    let mut le = Zeroizing::new(sk.secret_bytes());
    le.reverse();
    Share::from_bytes(*le)
}

fn pubkey(bytes: &[u8]) -> Result<PublicKey, SessionError> {
    PublicKey::from_slice(bytes).map_err(|_| SessionError::Key("bad public key".into()))
}

/// §3.5: the digests of the four pre-signed spends.
pub struct Digests {
    pub redeem: [u8; 32],
    pub cancel: [u8; 32],
    pub refund: [u8; 32],
    pub punish: [u8; 32],
}

fn digests(txs: &SwapTxs) -> Result<Digests, SessionError> {
    let v_lock = txs
        .lock
        .unsigned_tx
        .output
        .iter()
        .find(|o| o.script_pubkey == ScriptBuf::new_p2wsh(&txs.w_script.wscript_hash()))
        .map(|o| o.value)
        .ok_or(SessionError::Template("no W output".into()))?;
    let v_cancel = txs.cancel.output[0].value;
    let d = |tx: &Transaction, v: Amount| {
        sighash(tx, &txs.w_script, v).ok_or(SessionError::Template("sighash".into()))
    };
    Ok(Digests {
        redeem: d(&txs.redeem, v_lock)?,
        cancel: d(&txs.cancel, v_lock)?,
        refund: d(&txs.refund, v_cancel)?,
        punish: d(&txs.punish, v_cancel)?,
    })
}

/// §4.5-R1, §5.3-R3: recovers the peer's share from the signature `signer` published in `tx`.
fn recover_share(
    tx: &Transaction,
    signer: &PublicKey,
    other: &PublicKey,
    peer_y: &PublicKey,
    esig: &EncSig,
    peer: &SharePub,
    what: &'static str,
) -> Result<Share, SessionError> {
    let witness = &tx.input.first().ok_or(SessionError::Recover(what))?.witness;
    let sig = sig_in_witness(witness, signer, other).ok_or(SessionError::Recover(what))?;
    let sk =
        zecsplit_btc::adaptor::recover(peer_y, &sig, esig).ok_or(SessionError::Recover(what))?;
    let share = share_from_secp(&sk);
    if !check_share_matches(&share, peer) {
        return Err(SessionError::Recover(what));
    }
    Ok(share)
}

impl Core {
    /// §3.2-R2: fresh key material for `role` in this quote's swap.
    pub fn new<R: RngCore + CryptoRng>(
        rng: &mut R,
        quote: Quote,
        role: Role,
        payout_addr: String,
    ) -> Result<Self, SessionError> {
        let keys = generate(rng, &quote.swap_id, role)?;
        let commitment = keys.commitment(&quote.swap_id, role);
        Ok(Core {
            quote,
            role,
            keys,
            payout_addr,
            commitment,
            peer_commitment: None,
            own_reveal: None,
            peer: None,
            shared: None,
        })
    }

    fn peer_role(&self) -> Role {
        match self.role {
            Role::Alice => Role::Bob,
            Role::Bob => Role::Alice,
        }
    }

    fn ordered(&self) -> Result<(Commitment, Commitment), SessionError> {
        let peer = self
            .peer_commitment
            .ok_or(SessionError::OutOfOrder("no peer commit"))?;
        Ok(match self.role {
            Role::Alice => (self.commitment, peer),
            Role::Bob => (peer, self.commitment),
        })
    }

    /// §3.1: this party's commitment.
    pub fn commit_msg(&self) -> CommitBody {
        CommitBody {
            commitment: self.commitment.0,
        }
    }

    /// §3.1: stores the peer's commitment; a second, different one is refused.
    pub fn on_commit(&mut self, body: &CommitBody) -> Result<(), SessionError> {
        match self.peer_commitment {
            Some(c) if c.0 != body.commitment => Err(SessionError::OutOfOrder("second commit")),
            _ => {
                self.peer_commitment = Some(Commitment(body.commitment));
                Ok(())
            }
        }
    }

    /// §3.2-R1: this party's reveal, only after both commitments are known.
    pub fn reveal_msg(&mut self) -> Result<RevealBody, SessionError> {
        let (c_a, c_b) = self.ordered()?;
        let reveal = self.keys.reveal(
            &self.quote.swap_id,
            self.role,
            &c_a,
            &c_b,
            self.payout_addr.clone(),
        )?;
        self.own_reveal = Some(reveal.clone());
        Ok(RevealBody(reveal))
    }

    /// §3.2-R3, §3.3: verifies the peer's reveal and derives `ak`, `σ`, `T`, `nk` and `rivk`.
    pub fn on_reveal(&mut self, body: &RevealBody) -> Result<Shared, SessionError> {
        let (c_a, c_b) = self.ordered()?;
        let peer_c = self
            .peer_commitment
            .ok_or(SessionError::OutOfOrder("no peer commit"))?;
        let own = self
            .own_reveal
            .as_ref()
            .ok_or(SessionError::OutOfOrder("reveal first"))?;
        verify_reveal(
            &self.quote.swap_id,
            self.peer_role(),
            &peer_c,
            &body.0,
            &c_a,
            &c_b,
            &self.keys.share_pub,
        )?;
        let (reveal_a, reveal_b) = match self.role {
            Role::Alice => (own, &body.0),
            Role::Bob => (&body.0, own),
        };
        let key = shared_ak(&reveal_a.share_pub, &reveal_b.share_pub)
            .map_err(|e| SessionError::Key(e.to_string()))?;
        let quote_hash = quote_digest(
            &self
                .quote
                .canonical_json()
                .map_err(|e| SessionError::Key(e.to_string()))?,
        );
        let t = transcript(&TranscriptInput {
            protocol_version: PROTOCOL_VERSION,
            swap_id: self.quote.swap_id,
            network: crypto_network(self.quote.network),
            quote_hash,
            c_a,
            c_b,
            reveal_a,
            reveal_b,
        })
        .map_err(|e| SessionError::Key(e.to_string()))?;
        let (nk, rivk) = derive_nk_rivk(&t);
        let shared = Shared {
            ak: key.ak,
            sigma: key.sigma,
            transcript: t,
            nk,
            rivk,
        };
        self.peer = Some(body.0.clone());
        self.shared = Some(shared);
        Ok(shared)
    }

    fn peer(&self) -> Result<&Reveal, SessionError> {
        self.peer
            .as_ref()
            .ok_or(SessionError::OutOfOrder("no peer reveal"))
    }

    /// The derived Zcash setup, once both reveals are in.
    pub fn shared(&self) -> Option<Shared> {
        self.shared
    }

    fn own_btc_sk(&self) -> Result<SecretKey, SessionError> {
        SecretKey::from_slice(self.keys.btc_secret.expose_secret())
            .map_err(|_| SessionError::Key("own BTC key".into()))
    }

    fn own_btc_pk(&self) -> Result<PublicKey, SessionError> {
        pubkey(&self.keys.btc_key)
    }

    fn peer_btc_pk(&self) -> Result<PublicKey, SessionError> {
        pubkey(&self.peer()?.btc_key)
    }

    fn own_share_secp(&self) -> Result<SecretKey, SessionError> {
        share_secp(&self.keys.share)
    }

    fn own_y(&self) -> Result<PublicKey, SessionError> {
        pubkey(&self.keys.share_pub.secp)
    }

    fn peer_y(&self) -> Result<PublicKey, SessionError> {
        pubkey(&self.peer()?.share_pub.secp)
    }

    /// §3.4: the template parameters, with Alice's payout from her reveal and Bob's refund address.
    fn btc_params(&self, bob_refund_addr: &str) -> Result<BtcParams, SessionError> {
        let (k_a, k_b, alice_addr) = match self.role {
            Role::Alice => (
                self.own_btc_pk()?,
                self.peer_btc_pk()?,
                self.payout_addr.clone(),
            ),
            Role::Bob => (
                self.peer_btc_pk()?,
                self.own_btc_pk()?,
                self.peer()?.payout_addr.clone(),
            ),
        };
        Ok(BtcParams {
            btc_amount: Amount::from_sat(self.quote.btc_amount),
            fee_rate_sat_vb: self.quote.btc_fee_rate,
            t1: self.quote.t1,
            t2: self.quote.t2,
            k_a,
            k_b,
            alice_payout: script_for(&alice_addr, self.quote.network)?,
            bob_refund: script_for(bob_refund_addr, self.quote.network)?,
        })
    }

    /// §3.3-R1: `ask = σ·(s_a + s_b)` from this party's share and the recovered one.
    fn ask_with(&self, peer_share: &Share) -> Result<Ask, SessionError> {
        let sigma = self
            .shared
            .ok_or(SessionError::OutOfOrder("no shared key"))?
            .sigma;
        let (a, b) = match self.role {
            Role::Alice => (&self.keys.share, peer_share),
            Role::Bob => (peer_share, &self.keys.share),
        };
        shared_ask(a, b, sigma).map_err(|e| SessionError::Key(e.to_string()))
    }
}

#[cfg(test)]
mod tests;

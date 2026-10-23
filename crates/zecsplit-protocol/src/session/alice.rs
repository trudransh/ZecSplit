//! Alice's session: verifies Bob's templates and signatures, signs her side and redeems (§3.5, §4.4, §5.3).

use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::{Psbt, Transaction};
use zecsplit_btc::adaptor::{
    EncSig, decrypt, enc_sign, encsig_from_bytes, encsig_to_bytes, verify_enc,
};
use zecsplit_btc::sign::{sign, verify, witness};
use zecsplit_btc::txs::{SwapTxs, build_txs, lock_value};
use zecsplit_crypto::Ask;

use super::persist::parse;
use super::{Core, Digests, SessionError, digests, recover_share};
use crate::messages::{LockTemplate, RedeemEncSig, SetupSigsAlice, SetupSigsBob};
use crate::recovery::{
    ESIG_A_REFUND, RecoveryBundle, SIG_B_CANCEL, SIG_B_PUNISH, TX_CANCEL, TX_LOCK, TX_PUNISH,
    TX_REDEEM, TX_REFUND,
};
use bitcoin::consensus::encode::serialize;

/// Alice's per-swap state beyond commit-reveal.
pub struct AliceSession {
    pub core: Core,
    txs: Option<(SwapTxs, Digests)>,
    esig_a_refund: Option<EncSig>,
    sig_b_cancel: Option<Signature>,
    sig_b_punish: Option<Signature>,
    esig_b_redeem: Option<EncSig>,
}

fn sig_from(bytes: &[u8], what: &'static str) -> Result<Signature, SessionError> {
    Signature::from_compact(bytes).map_err(|_| SessionError::BadSignature(what))
}

impl AliceSession {
    pub fn new(core: Core) -> Self {
        AliceSession {
            core,
            txs: None,
            esig_a_refund: None,
            sig_b_cancel: None,
            sig_b_punish: None,
            esig_b_redeem: None,
        }
    }

    fn txs(&self) -> Result<&(SwapTxs, Digests), SessionError> {
        self.txs
            .as_ref()
            .ok_or(SessionError::OutOfOrder("no lock template"))
    }

    /// §3.5-R1, §3.5-R2: rebuilds the four spends from Bob's `tx_lock`, then signs the cancel and adaptor-signs the refund.
    pub fn on_lock_template(&mut self, tpl: &LockTemplate) -> Result<SetupSigsAlice, SessionError> {
        let params = self.core.btc_params(&tpl.refund_addr)?;
        let psbt: Psbt = tpl
            .psbt_base64
            .parse()
            .map_err(|_| SessionError::Template("bad PSBT".into()))?;
        let v_lock = lock_value(&params).map_err(|e| SessionError::Template(e.to_string()))?;
        if v_lock.to_sat() != tpl.v_lock_sats {
            return Err(SessionError::Template(
                "v_lock does not match the quote".into(),
            ));
        }
        let txs = build_txs(&params, psbt).map_err(|e| SessionError::Template(e.to_string()))?;
        let d = digests(&txs)?;
        let sk = self.core.own_btc_sk()?;
        let sig_cancel = sign(&sk, &d.cancel);
        let esig = enc_sign(&sk, &self.core.peer_y()?, &d.refund);
        let msg = SetupSigsAlice {
            sig_cancel: sig_cancel.serialize_compact().to_vec(),
            esig_refund: encsig_to_bytes(&esig),
        };
        self.esig_a_refund = Some(esig);
        self.txs = Some((txs, d));
        self.core.record("lock_template", tpl)?;
        Ok(msg)
    }

    /// §3.5-R3, §3.5-R4: verifies Bob's cancel and punish signatures.
    pub fn on_setup_sigs_bob(&mut self, msg: &SetupSigsBob) -> Result<(), SessionError> {
        let k_b = self.core.peer_btc_pk()?;
        let (_, d) = self.txs()?;
        let cancel = sig_from(&msg.sig_cancel, "sig_B(tx_cancel)")?;
        let punish = sig_from(&msg.sig_punish, "sig_B(tx_punish)")?;
        if !verify(&k_b, &d.cancel, &cancel) {
            return Err(SessionError::BadSignature("sig_B(tx_cancel)"));
        }
        if !verify(&k_b, &d.punish, &punish) {
            return Err(SessionError::BadSignature("sig_B(tx_punish)"));
        }
        self.sig_b_cancel = Some(cancel);
        self.sig_b_punish = Some(punish);
        self.core.record("setup_sigs_bob", msg)
    }

    /// §4.4-R2: verifies `esig_B(tx_redeem; Y_a)`.
    pub fn on_encsig(&mut self, msg: &RedeemEncSig) -> Result<(), SessionError> {
        let esig = encsig_from_bytes(&msg.esig_redeem)
            .ok_or(SessionError::BadSignature("esig_B(tx_redeem)"))?;
        let (_, d) = self.txs()?;
        if !verify_enc(
            &self.core.peer_btc_pk()?,
            &self.core.own_y()?,
            &d.redeem,
            &esig,
        ) {
            return Err(SessionError::BadSignature("esig_B(tx_redeem)"));
        }
        self.esig_b_redeem = Some(esig);
        self.core.record("encsig", msg)
    }

    /// The four pre-signed spends, once the lock template is in.
    pub fn templates(&self) -> Option<&SwapTxs> {
        self.txs.as_ref().map(|(t, _)| t)
    }

    /// §4.4-R2: `tx_redeem` with `sig_B` decrypted by `s_a` and Alice's own signature (reveals `s_a`).
    pub fn signed_redeem(&self) -> Result<Transaction, SessionError> {
        let (txs, d) = self.txs()?;
        let esig = self
            .esig_b_redeem
            .as_ref()
            .ok_or(SessionError::OutOfOrder("no esig_B"))?;
        let sig_b = decrypt(&self.core.own_share_secp()?, esig);
        let sig_a = sign(&self.core.own_btc_sk()?, &d.redeem);
        let mut tx = txs.redeem.clone();
        tx.input[0].witness = witness(
            &txs.w_script,
            &self.core.own_btc_pk()?,
            &sig_a,
            &self.core.peer_btc_pk()?,
            &sig_b,
        );
        Ok(tx)
    }

    /// §5.3-R1: the fully signed `tx_cancel`.
    pub fn signed_cancel(&self) -> Result<Transaction, SessionError> {
        let (txs, d) = self.txs()?;
        let sig_b = self
            .sig_b_cancel
            .ok_or(SessionError::OutOfOrder("no sig_B(tx_cancel)"))?;
        let mut tx = txs.cancel.clone();
        tx.input[0].witness = witness(
            &txs.w_script,
            &self.core.own_btc_pk()?,
            &sign(&self.core.own_btc_sk()?, &d.cancel),
            &self.core.peer_btc_pk()?,
            &sig_b,
        );
        Ok(tx)
    }

    /// §5.3-R4: the fully signed `tx_punish`.
    pub fn signed_punish(&self) -> Result<Transaction, SessionError> {
        let (txs, d) = self.txs()?;
        let sig_b = self
            .sig_b_punish
            .ok_or(SessionError::OutOfOrder("no sig_B(tx_punish)"))?;
        let mut tx = txs.punish.clone();
        tx.input[0].witness = witness(
            &txs.w_script,
            &self.core.own_btc_pk()?,
            &sign(&self.core.own_btc_sk()?, &d.punish),
            &self.core.peer_btc_pk()?,
            &sig_b,
        );
        Ok(tx)
    }

    /// §5.3-R3: recovers `s_b` from Bob's published `tx_refund` and returns `ask` to sweep the ZEC back.
    pub fn ask_from_refund(&self, refund: &Transaction) -> Result<Ask, SessionError> {
        let esig = self
            .esig_a_refund
            .as_ref()
            .ok_or(SessionError::OutOfOrder("no esig_A"))?;
        let s_b = recover_share(
            refund,
            &self.core.own_btc_pk()?,
            &self.core.peer_btc_pk()?,
            &self.core.peer_y()?,
            esig,
            &self.core.peer()?.share_pub,
            "tx_refund",
        )?;
        self.core.ask_with(&s_b)
    }
}

impl AliceSession {
    /// §6.4-R1: the recovery bundle, with the templates and Bob's signatures the spec requires.
    pub fn save(&self, maker_endpoint: &str) -> Result<RecoveryBundle, SessionError> {
        let mut extra = Vec::new();
        if let Some((txs, _)) = &self.txs {
            extra.push((TX_LOCK.to_string(), txs.lock.serialize()));
            for (name, tx) in [
                (TX_REDEEM, &txs.redeem),
                (TX_CANCEL, &txs.cancel),
                (TX_REFUND, &txs.refund),
                (TX_PUNISH, &txs.punish),
            ] {
                extra.push((name.to_string(), serialize(tx)));
            }
        }
        for (name, sig) in [
            (SIG_B_CANCEL, self.sig_b_cancel),
            (SIG_B_PUNISH, self.sig_b_punish),
        ] {
            if let Some(s) = sig {
                extra.push((name.to_string(), s.serialize_compact().to_vec()));
            }
        }
        if let Some(e) = &self.esig_a_refund {
            extra.push((ESIG_A_REFUND.to_string(), encsig_to_bytes(e)));
        }
        self.core.bundle(maker_endpoint, extra)
    }

    /// §6.3-R2, §6.4: rebuilds the session by replaying every saved input through the same checks.
    pub fn restore(bundle: RecoveryBundle) -> Result<Self, SessionError> {
        let (core, blobs) = Core::restored(bundle)?;
        let mut s = AliceSession::new(core);
        for (name, bytes) in &blobs {
            if s.core.replay(name, bytes)? {
                continue;
            }
            match name.as_str() {
                "lock_template" => {
                    s.on_lock_template(&parse(bytes)?)?;
                }
                "setup_sigs_bob" => s.on_setup_sigs_bob(&parse(bytes)?)?,
                "encsig" => s.on_encsig(&parse(bytes)?)?,
                _ => {}
            }
        }
        Ok(s)
    }
}

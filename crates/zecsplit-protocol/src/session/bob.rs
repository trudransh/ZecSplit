//! Bob's session: builds the lock template, checks Alice's signatures, adaptor-signs the redeem and recovers `s_a` (§3.5, §4.3, §4.5, §5.3).

use bitcoin::secp256k1::ecdsa::Signature;
use bitcoin::{Amount, Psbt, ScriptBuf, Transaction};
use zecsplit_btc::adaptor::{
    EncSig, decrypt, enc_sign, encsig_from_bytes, encsig_to_bytes, verify_enc,
};
use zecsplit_btc::sign::{sign, verify, witness};
use zecsplit_btc::txs::{SwapTxs, build_txs, lock_script_pubkey, lock_value};
use zecsplit_crypto::Ask;

use super::persist::parse;
use super::{Core, Digests, SessionError, digests, recover_share};
use crate::messages::{LockTemplate, RedeemEncSig, SetupSigsAlice, SetupSigsBob};
use crate::recovery::RecoveryBundle;

/// Bob's per-swap state beyond commit-reveal.
pub struct BobSession {
    pub core: Core,
    txs: Option<(SwapTxs, Digests)>,
    sig_a_cancel: Option<Signature>,
    esig_a_refund: Option<EncSig>,
    esig_b_redeem: Option<EncSig>,
}

impl BobSession {
    pub fn new(core: Core) -> Self {
        BobSession {
            core,
            txs: None,
            sig_a_cancel: None,
            esig_a_refund: None,
            esig_b_redeem: None,
        }
    }

    fn txs(&self) -> Result<&(SwapTxs, Digests), SessionError> {
        self.txs
            .as_ref()
            .ok_or(SessionError::OutOfOrder("no lock template"))
    }

    /// §3.4: the output Bob's wallet must fund in `tx_lock`, as `(P2WSH(W), V_lock)`.
    pub fn lock_output(&self, refund_addr: &str) -> Result<(ScriptBuf, Amount), SessionError> {
        let p = self.core.btc_params(refund_addr)?;
        let v = lock_value(&p).map_err(|e| SessionError::Template(e.to_string()))?;
        Ok((lock_script_pubkey(&p), v))
    }

    /// §3.5-R1: builds the four spends from Bob's unsigned `tx_lock` and returns the template message.
    pub fn lock_template(
        &mut self,
        psbt: Psbt,
        refund_addr: &str,
    ) -> Result<LockTemplate, SessionError> {
        if self.txs.is_some() {
            return Err(SessionError::OutOfOrder("lock template already built"));
        }
        let p = self.core.btc_params(refund_addr)?;
        let v_lock = lock_value(&p).map_err(|e| SessionError::Template(e.to_string()))?;
        let psbt_base64 = psbt.to_string();
        let txs = build_txs(&p, psbt).map_err(|e| SessionError::Template(e.to_string()))?;
        let lock_spk = lock_script_pubkey(&p);
        let lock_vout = txs
            .lock
            .unsigned_tx
            .output
            .iter()
            .position(|o| o.script_pubkey == lock_spk)
            .and_then(|i| u32::try_from(i).ok())
            .ok_or(SessionError::Template("no W output".into()))?;
        let d = digests(&txs)?;
        self.txs = Some((txs, d));
        let tpl = LockTemplate {
            psbt_base64,
            lock_vout,
            v_lock_sats: v_lock.to_sat(),
            refund_addr: refund_addr.to_string(),
        };
        self.core.record("lock_template", &tpl)?;
        Ok(tpl)
    }

    /// §3.5-R1: the template already sent, for resending after a restart.
    pub fn lock_template_msg(&self) -> Result<Option<LockTemplate>, SessionError> {
        self.core.logged("lock_template")
    }

    /// §3.5-R4, §3.5-R5: verifies Alice's cancel sig and refund adaptor sig, then returns Bob's cancel and punish sigs.
    pub fn on_setup_sigs_alice(
        &mut self,
        msg: &SetupSigsAlice,
    ) -> Result<SetupSigsBob, SessionError> {
        if self.core.seen("setup_sigs_alice", msg)? {
            return self.setup_sigs();
        }
        let k_a = self.core.peer_btc_pk()?;
        let (_, d) = self.txs()?;
        let cancel = Signature::from_compact(&msg.sig_cancel)
            .map_err(|_| SessionError::BadSignature("sig_A(tx_cancel)"))?;
        if !verify(&k_a, &d.cancel, &cancel) {
            return Err(SessionError::BadSignature("sig_A(tx_cancel)"));
        }
        let esig = encsig_from_bytes(&msg.esig_refund)
            .ok_or(SessionError::BadSignature("esig_A(tx_refund)"))?;
        if !verify_enc(&k_a, &self.core.own_y()?, &d.refund, &esig) {
            return Err(SessionError::BadSignature("esig_A(tx_refund)"));
        }
        self.sig_a_cancel = Some(cancel);
        self.esig_a_refund = Some(esig);
        self.core.record("setup_sigs_alice", msg)?;
        self.setup_sigs()
    }

    /// §3.5-R3: Bob's cancel and punish signatures, re-derivable at any time (RFC6979).
    pub fn setup_sigs(&self) -> Result<SetupSigsBob, SessionError> {
        let (_, d) = self.txs()?;
        let sk = self.core.own_btc_sk()?;
        Ok(SetupSigsBob {
            sig_cancel: sign(&sk, &d.cancel).serialize_compact().to_vec(),
            sig_punish: sign(&sk, &d.punish).serialize_compact().to_vec(),
        })
    }

    /// §3.5-R5: true only once Alice's cancel sig and refund adaptor sig have been verified.
    pub fn may_broadcast_lock(&self) -> bool {
        self.sig_a_cancel.is_some() && self.esig_a_refund.is_some()
    }

    /// The four pre-signed spends, once the lock template is built.
    pub fn templates(&self) -> Option<&SwapTxs> {
        self.txs.as_ref().map(|(t, _)| t)
    }

    /// §4.3-R3: `esig_B(tx_redeem; Y_a)`, kept so `s_a` can be recovered later (§6.4).
    pub fn encsig(&mut self) -> Result<RedeemEncSig, SessionError> {
        let (_, d) = self.txs()?;
        let esig = enc_sign(&self.core.own_btc_sk()?, &self.core.peer_y()?, &d.redeem);
        let msg = RedeemEncSig {
            esig_redeem: encsig_to_bytes(&esig),
        };
        self.esig_b_redeem = Some(esig);
        if !self.core.seen("encsig", &msg)? {
            self.core.record("encsig", &msg)?;
        }
        Ok(msg)
    }

    /// §4.5-R1–R3: recovers `s_a` from Alice's published `tx_redeem`, checks it, and returns `ask`.
    pub fn ask_from_redeem(&self, redeem: &Transaction) -> Result<Ask, SessionError> {
        let esig = self
            .esig_b_redeem
            .as_ref()
            .ok_or(SessionError::OutOfOrder("no esig_B"))?;
        let s_a = recover_share(
            redeem,
            &self.core.own_btc_pk()?,
            &self.core.peer_btc_pk()?,
            &self.core.peer_y()?,
            esig,
            &self.core.peer()?.share_pub,
            "tx_redeem",
        )?;
        self.core.ask_with(&s_a)
    }

    /// §5.2: the fully signed `tx_cancel`.
    pub fn signed_cancel(&self) -> Result<Transaction, SessionError> {
        let (txs, d) = self.txs()?;
        let sig_a = self
            .sig_a_cancel
            .ok_or(SessionError::OutOfOrder("no sig_A(tx_cancel)"))?;
        let mut tx = txs.cancel.clone();
        tx.input[0].witness = witness(
            &txs.w_script,
            &self.core.peer_btc_pk()?,
            &sig_a,
            &self.core.own_btc_pk()?,
            &sign(&self.core.own_btc_sk()?, &d.cancel),
        );
        Ok(tx)
    }

    /// §5.3-R2: `tx_refund` with `sig_A` decrypted by `s_b` (reveals `s_b`) and Bob's own signature.
    pub fn signed_refund(&self) -> Result<Transaction, SessionError> {
        let (txs, d) = self.txs()?;
        let esig = self
            .esig_a_refund
            .as_ref()
            .ok_or(SessionError::OutOfOrder("no esig_A"))?;
        let sig_a = decrypt(&self.core.own_share_secp()?, esig);
        let mut tx = txs.refund.clone();
        tx.input[0].witness = witness(
            &txs.w_script,
            &self.core.peer_btc_pk()?,
            &sig_a,
            &self.core.own_btc_pk()?,
            &sign(&self.core.own_btc_sk()?, &d.refund),
        );
        Ok(tx)
    }
}

impl BobSession {
    /// §6.4: Bob's saved session; his DB keeps `esig_B(tx_redeem)` through the replay log.
    pub fn save(&self, maker_endpoint: &str) -> Result<RecoveryBundle, SessionError> {
        self.core.bundle(maker_endpoint, Vec::new())
    }

    /// §6.3-R2: rebuilds the session by replaying every saved input through the same checks.
    pub fn restore(bundle: RecoveryBundle) -> Result<Self, SessionError> {
        let (core, blobs) = Core::restored(bundle)?;
        let mut s = BobSession::new(core);
        for (name, bytes) in &blobs {
            if s.core.replay(name, bytes)? {
                continue;
            }
            match name.as_str() {
                "lock_template" => {
                    let tpl: LockTemplate = parse(bytes)?;
                    let psbt: Psbt = tpl
                        .psbt_base64
                        .parse()
                        .map_err(|_| SessionError::Template("saved PSBT".into()))?;
                    s.lock_template(psbt, &tpl.refund_addr)?;
                }
                "setup_sigs_alice" => {
                    s.on_setup_sigs_alice(&parse(bytes)?)?;
                }
                "encsig" => {
                    s.encsig()?;
                }
                _ => {}
            }
        }
        Ok(s)
    }
}

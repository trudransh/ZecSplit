use std::fmt;

use bitcoin::opcodes::all::OP_CHECKMULTISIG;
use bitcoin::secp256k1::PublicKey;
use bitcoin::{
    Amount, OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute,
    transaction,
};

const MAX_SIG_LEN: usize = 73;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcParams {
    pub btc_amount: Amount,
    pub fee_rate_sat_vb: u64,
    pub t1: u16,
    pub t2: u16,
    pub k_a: PublicKey,
    pub k_b: PublicKey,
    pub alice_payout: ScriptBuf,
    pub bob_refund: ScriptBuf,
}

#[derive(Debug, Clone)]
pub struct SwapTxs {
    pub w_script: ScriptBuf,
    pub lock: Psbt,
    pub redeem: Transaction,
    pub cancel: Transaction,
    pub refund: Transaction,
    pub punish: Transaction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxError {
    ZeroTimelock,
    NonSegwitLockInput(usize),
    LockOutputNotUnique,
    LockValueMismatch { expected: Amount, got: Amount },
    BelowDust(&'static str),
    Overflow,
}

impl fmt::Display for TxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTimelock => write!(f, "t1 and t2 must be at least one block"),
            Self::NonSegwitLockInput(i) => write!(f, "tx_lock input {i} is not P2WPKH or P2TR"),
            Self::LockOutputNotUnique => write!(f, "tx_lock must pay P2WSH(W) exactly once"),
            Self::LockValueMismatch { expected, got } => {
                write!(f, "tx_lock pays {got} to P2WSH(W), expected {expected}")
            }
            Self::BelowDust(tx) => write!(f, "{tx} output would be dust"),
            Self::Overflow => write!(f, "amount overflow"),
        }
    }
}

impl std::error::Error for TxError {}

/// `OP_2 <K_1> <K_2> OP_2 OP_CHECKMULTISIG` with the keys in lexicographic order (§3.4).
pub fn w_script(k_a: &PublicKey, k_b: &PublicKey) -> ScriptBuf {
    let (k1, k2) = if k_a.serialize() <= k_b.serialize() {
        (k_a, k_b)
    } else {
        (k_b, k_a)
    };
    ScriptBuf::builder()
        .push_int(2)
        .push_key(&bitcoin::PublicKey::new(*k1))
        .push_key(&bitcoin::PublicKey::new(*k2))
        .push_int(2)
        .push_opcode(OP_CHECKMULTISIG)
        .into_script()
}

pub fn lock_script_pubkey(p: &BtcParams) -> ScriptBuf {
    ScriptBuf::new_p2wsh(&w_script(&p.k_a, &p.k_b).wscript_hash())
}

/// `V_lock = btc_amount + fee(tx_redeem)`.
pub fn lock_value(p: &BtcParams) -> Result<Amount, TxError> {
    if p.btc_amount < p.alice_payout.minimal_non_dust() {
        return Err(TxError::BelowDust("tx_redeem"));
    }
    let w = w_script(&p.k_a, &p.k_b);
    let redeem = spend(
        OutPoint::null(),
        Sequence::ENABLE_RBF_NO_LOCKTIME,
        &p.alice_payout,
    );
    let fee = fee(p.fee_rate_sat_vb, redeem, &w)?;
    p.btc_amount.checked_add(fee).ok_or(TxError::Overflow)
}

/// Builds the four pre-signed transactions from the quote, keys, addresses and Bob's unsigned
/// `tx_lock` (§3.4-R1).
pub fn build_txs(p: &BtcParams, lock: Psbt) -> Result<SwapTxs, TxError> {
    if p.t1 == 0 || p.t2 == 0 {
        return Err(TxError::ZeroTimelock);
    }

    for i in 0..lock.unsigned_tx.input.len() {
        let segwit = lock
            .inputs
            .get(i)
            .and_then(|input| input.witness_utxo.as_ref())
            .is_some_and(|utxo| utxo.script_pubkey.is_p2wpkh() || utxo.script_pubkey.is_p2tr());
        if !segwit {
            return Err(TxError::NonSegwitLockInput(i));
        }
    }

    let w = w_script(&p.k_a, &p.k_b);
    let lock_spk = lock_script_pubkey(p);
    let mut matches = lock
        .unsigned_tx
        .output
        .iter()
        .enumerate()
        .filter(|(_, out)| out.script_pubkey == lock_spk);
    let (vout, lock_out) = match (matches.next(), matches.next()) {
        (Some(m), None) => m,
        _ => return Err(TxError::LockOutputNotUnique),
    };
    let v_lock = lock_value(p)?;
    if lock_out.value != v_lock {
        return Err(TxError::LockValueMismatch {
            expected: v_lock,
            got: lock_out.value,
        });
    }
    let lock_outpoint = OutPoint::new(lock.unsigned_tx.compute_txid(), vout as u32);

    let mut redeem = spend(
        lock_outpoint,
        Sequence::ENABLE_RBF_NO_LOCKTIME,
        &p.alice_payout,
    );
    redeem.output[0].value = p.btc_amount;

    let cancel = spend_paying_fee(
        "tx_cancel",
        lock_outpoint,
        v_lock,
        Sequence::from_height(p.t1),
        &lock_spk,
        p.fee_rate_sat_vb,
        &w,
    )?;
    let cancel_outpoint = OutPoint::new(cancel.compute_txid(), 0);
    let cancel_value = cancel.output[0].value;

    let refund = spend_paying_fee(
        "tx_refund",
        cancel_outpoint,
        cancel_value,
        Sequence::ENABLE_RBF_NO_LOCKTIME,
        &p.bob_refund,
        p.fee_rate_sat_vb,
        &w,
    )?;
    let punish = spend_paying_fee(
        "tx_punish",
        cancel_outpoint,
        cancel_value,
        Sequence::from_height(p.t2),
        &p.alice_payout,
        p.fee_rate_sat_vb,
        &w,
    )?;

    Ok(SwapTxs {
        w_script: w,
        lock,
        redeem,
        cancel,
        refund,
        punish,
    })
}

fn spend(prev: OutPoint, sequence: Sequence, to: &ScriptBuf) -> Transaction {
    Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: prev,
            script_sig: ScriptBuf::new(),
            sequence,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: to.clone(),
        }],
    }
}

fn spend_paying_fee(
    name: &'static str,
    prev: OutPoint,
    prev_value: Amount,
    sequence: Sequence,
    to: &ScriptBuf,
    fee_rate_sat_vb: u64,
    w: &ScriptBuf,
) -> Result<Transaction, TxError> {
    let mut tx = spend(prev, sequence, to);
    let fee = fee(fee_rate_sat_vb, tx.clone(), w)?;
    let value = prev_value
        .checked_sub(fee)
        .ok_or(TxError::BelowDust(name))?;
    if value < to.minimal_non_dust() {
        return Err(TxError::BelowDust(name));
    }
    tx.output[0].value = value;
    Ok(tx)
}

/// `fee_rate × vsize`, with vsize measured on the tx carrying a max-size 2-of-2 witness.
fn fee(fee_rate_sat_vb: u64, mut tx: Transaction, w: &ScriptBuf) -> Result<Amount, TxError> {
    tx.input[0].witness = Witness::from_slice(&[
        vec![],
        vec![0; MAX_SIG_LEN],
        vec![0; MAX_SIG_LEN],
        w.to_bytes(),
    ]);
    fee_rate_sat_vb
        .checked_mul(tx.vsize() as u64)
        .map(Amount::from_sat)
        .ok_or(TxError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;
    use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
    use bitcoin::sighash::{EcdsaSighashType, SighashCache};
    use bitcoin::{PubkeyHash, Txid, WPubkeyHash};

    fn keys() -> (SecretKey, SecretKey) {
        (
            SecretKey::from_slice(&[0x11; 32]).unwrap(),
            SecretKey::from_slice(&[0x22; 32]).unwrap(),
        )
    }

    fn params() -> BtcParams {
        let secp = Secp256k1::new();
        let (sk_a, sk_b) = keys();
        BtcParams {
            btc_amount: Amount::from_sat(100_000),
            fee_rate_sat_vb: 2,
            t1: 36,
            t2: 36,
            k_a: sk_a.public_key(&secp),
            k_b: sk_b.public_key(&secp),
            alice_payout: ScriptBuf::new_p2wpkh(&WPubkeyHash::from_byte_array([0xaa; 20])),
            bob_refund: ScriptBuf::new_p2wpkh(&WPubkeyHash::from_byte_array([0xbb; 20])),
        }
    }

    fn lock_psbt(p: &BtcParams, input_spk: ScriptBuf, lock_amount: Amount) -> Psbt {
        let tx = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::from_byte_array([0x01; 32]), 0),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![
                TxOut {
                    value: Amount::from_sat(50_000),
                    script_pubkey: p.bob_refund.clone(),
                },
                TxOut {
                    value: lock_amount,
                    script_pubkey: lock_script_pubkey(p),
                },
            ],
        };
        let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(1_000_000),
            script_pubkey: input_spk,
        });
        psbt
    }

    fn segwit_input() -> ScriptBuf {
        ScriptBuf::new_p2wpkh(&WPubkeyHash::from_byte_array([0xcc; 20]))
    }

    fn built() -> SwapTxs {
        let p = params();
        build_txs(&p, lock_psbt(&p, segwit_input(), lock_value(&p).unwrap())).unwrap()
    }

    #[test]
    fn spec_3_4_w_script_is_sorted_2_of_2() {
        let p = params();
        let w = w_script(&p.k_a, &p.k_b);
        assert_eq!(w, w_script(&p.k_b, &p.k_a));
        assert_eq!(w.len(), 71);
        assert!(w.is_multisig());
    }

    #[test]
    fn spec_3_4_template_shapes() {
        let p = params();
        let txs = built();
        let lock_vout = 1;
        let lock_outpoint = OutPoint::new(txs.lock.unsigned_tx.compute_txid(), lock_vout);
        let v_lock = txs.lock.unsigned_tx.output[lock_vout as usize].value;
        let rbf = Sequence::ENABLE_RBF_NO_LOCKTIME;

        for tx in [&txs.redeem, &txs.cancel, &txs.refund, &txs.punish] {
            assert_eq!(tx.version, transaction::Version::TWO);
            assert_eq!(tx.lock_time, absolute::LockTime::ZERO);
            assert_eq!((tx.input.len(), tx.output.len()), (1, 1));
        }

        assert_eq!(txs.redeem.input[0].previous_output, lock_outpoint);
        assert_eq!(txs.redeem.input[0].sequence, rbf);
        assert_eq!(txs.redeem.output[0].script_pubkey, p.alice_payout);
        assert_eq!(txs.redeem.output[0].value, p.btc_amount);

        assert_eq!(txs.cancel.input[0].previous_output, lock_outpoint);
        assert_eq!(txs.cancel.input[0].sequence, Sequence::from_height(p.t1));
        assert_eq!(txs.cancel.output[0].script_pubkey, lock_script_pubkey(&p));
        assert!(txs.cancel.output[0].value < v_lock);

        let cancel_outpoint = OutPoint::new(txs.cancel.compute_txid(), 0);
        let cancel_value = txs.cancel.output[0].value;

        assert_eq!(txs.refund.input[0].previous_output, cancel_outpoint);
        assert_eq!(txs.refund.input[0].sequence, rbf);
        assert_eq!(txs.refund.output[0].script_pubkey, p.bob_refund);
        assert!(txs.refund.output[0].value < cancel_value);

        assert_eq!(txs.punish.input[0].previous_output, cancel_outpoint);
        assert_eq!(txs.punish.input[0].sequence, Sequence::from_height(p.t2));
        assert_eq!(txs.punish.output[0].script_pubkey, p.alice_payout);
        assert!(txs.punish.output[0].value < cancel_value);
    }

    #[test]
    fn spec_3_4_r1_fee_matches_signed_vsize() {
        let secp = Secp256k1::new();
        let p = params();
        let (sk_a, sk_b) = keys();
        let txs = built();
        let v_lock = txs.lock.unsigned_tx.output[1].value;
        let v_cancel = txs.cancel.output[0].value;

        for (tx, spent) in [
            (&txs.redeem, v_lock),
            (&txs.cancel, v_lock),
            (&txs.refund, v_cancel),
            (&txs.punish, v_cancel),
        ] {
            let sighash = SighashCache::new(tx)
                .p2wsh_signature_hash(0, &txs.w_script, spent, EcdsaSighashType::All)
                .unwrap();
            let msg = Message::from_digest(sighash.to_byte_array());
            let sig = |sk: &SecretKey| {
                bitcoin::ecdsa::Signature::sighash_all(secp.sign_ecdsa(&msg, sk)).to_vec()
            };
            let mut signed = tx.clone();
            signed.input[0].witness =
                Witness::from_slice(&[vec![], sig(&sk_a), sig(&sk_b), txs.w_script.to_bytes()]);

            let paid = spent - tx.output[0].value;
            let estimated_vsize = paid.to_sat() / p.fee_rate_sat_vb;
            let real_vsize = signed.vsize() as u64;
            assert!(
                estimated_vsize.abs_diff(real_vsize) <= 2,
                "{estimated_vsize} vs {real_vsize}"
            );
        }
    }

    #[test]
    fn spec_3_4_r2_rejects_non_segwit_lock_inputs() {
        let p = params();
        let v_lock = lock_value(&p).unwrap();

        let p2pkh = ScriptBuf::new_p2pkh(&PubkeyHash::from_byte_array([0xdd; 20]));
        let err = build_txs(&p, lock_psbt(&p, p2pkh, v_lock)).unwrap_err();
        assert_eq!(err, TxError::NonSegwitLockInput(0));

        let mut no_utxo = lock_psbt(&p, segwit_input(), v_lock);
        no_utxo.inputs[0].witness_utxo = None;
        assert_eq!(
            build_txs(&p, no_utxo).unwrap_err(),
            TxError::NonSegwitLockInput(0)
        );
    }

    #[test]
    fn rejects_wrong_lock_value() {
        let p = params();
        let v_lock = lock_value(&p).unwrap();
        let short = v_lock - Amount::from_sat(1);
        assert_eq!(
            build_txs(&p, lock_psbt(&p, segwit_input(), short)).unwrap_err(),
            TxError::LockValueMismatch {
                expected: v_lock,
                got: short
            }
        );
    }

    #[test]
    fn rejects_lock_without_w_output() {
        let p = params();
        let mut psbt = lock_psbt(&p, segwit_input(), lock_value(&p).unwrap());
        psbt.unsigned_tx.output.pop();
        assert_eq!(
            build_txs(&p, psbt).unwrap_err(),
            TxError::LockOutputNotUnique
        );
    }

    #[test]
    fn rejects_zero_timelock() {
        let mut p = params();
        let psbt = lock_psbt(&p, segwit_input(), lock_value(&p).unwrap());
        p.t1 = 0;
        assert_eq!(build_txs(&p, psbt).unwrap_err(), TxError::ZeroTimelock);
    }

    #[test]
    fn rejects_amount_too_small_for_fees() {
        let mut p = params();
        p.btc_amount = Amount::from_sat(293);
        assert_eq!(lock_value(&p).unwrap_err(), TxError::BelowDust("tx_redeem"));

        p.btc_amount = Amount::from_sat(400);
        p.fee_rate_sat_vb = 50;
        let psbt = lock_psbt(&p, segwit_input(), lock_value(&p).unwrap());
        assert_eq!(
            build_txs(&p, psbt).unwrap_err(),
            TxError::BelowDust("tx_cancel")
        );
    }
}

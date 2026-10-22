//! BIP143 sighashes, plain signatures and 2-of-2 witnesses for the spends of `P2WSH(W)` (PROTOCOL_SPEC §3.4).

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, SecretKey, ecdsa::Signature};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{Amount, Script, Transaction, Witness};

/// §3.4: the `SIGHASH_ALL` BIP143 digest of input 0 spending `P2WSH(w)` worth `prev_value`.
pub fn sighash(tx: &Transaction, w: &Script, prev_value: Amount) -> Option<[u8; 32]> {
    SighashCache::new(tx)
        .p2wsh_signature_hash(0, w, prev_value, EcdsaSighashType::All)
        .ok()
        .map(|h| h.to_byte_array())
}

/// A low-S ECDSA signature over `digest`.
pub fn sign(sk: &SecretKey, digest: &[u8; 32]) -> Signature {
    Secp256k1::signing_only().sign_ecdsa(&Message::from_digest(*digest), sk)
}

/// §3.5-R4: true iff `sig` is a valid low-S signature on `digest` under `pk`.
pub fn verify(pk: &PublicKey, digest: &[u8; 32], sig: &Signature) -> bool {
    let mut low = *sig;
    low.normalize_s();
    low == *sig
        && Secp256k1::verification_only()
            .verify_ecdsa(&Message::from_digest(*digest), sig, pk)
            .is_ok()
}

fn ordered<'a, T>(k_a: &PublicKey, k_b: &PublicKey, a: &'a T, b: &'a T) -> [&'a T; 2] {
    if k_a.serialize() <= k_b.serialize() {
        [a, b]
    } else {
        [b, a]
    }
}

/// §3.4: `<> <sig_1> <sig_2> <W>`, signatures in the key order of `W`.
pub fn witness(
    w: &Script,
    k_a: &PublicKey,
    sig_a: &Signature,
    k_b: &PublicKey,
    sig_b: &Signature,
) -> Witness {
    let [s1, s2] = ordered(k_a, k_b, sig_a, sig_b);
    let with_type = |s: &Signature| bitcoin::ecdsa::Signature::sighash_all(*s).to_vec();
    Witness::from_slice(&[vec![], with_type(s1), with_type(s2), w.to_bytes()])
}

/// §4.5-R1, §5.3-R3: the signature `k` placed in a published 2-of-2 witness.
pub fn sig_in_witness(witness: &Witness, k: &PublicKey, other: &PublicKey) -> Option<Signature> {
    let slot = if k.serialize() <= other.serialize() {
        1
    } else {
        2
    };
    if witness.len() != 4 {
        return None;
    }
    bitcoin::ecdsa::Signature::from_slice(witness.nth(slot)?)
        .ok()
        .filter(|s| s.sighash_type == EcdsaSighashType::All)
        .map(|s| s.signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::{decrypt, enc_sign, recover};
    use crate::txs::w_script;
    use bitcoin::absolute::LockTime;
    use bitcoin::transaction::Version;
    use bitcoin::{OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Txid};
    use sha2::{Digest, Sha256};

    fn key(seed: &str) -> SecretKey {
        SecretKey::from_slice(&Sha256::digest(seed)).unwrap()
    }

    fn spend() -> Transaction {
        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::all_zeros(), 0),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(90_000),
                script_pubkey: ScriptBuf::new_op_return([1u8; 4]),
            }],
        }
    }

    #[test]
    fn spec_3_4_witness_orders_signatures_like_w() {
        let secp = Secp256k1::new();
        for (a, b) in [("a", "b"), ("b", "a"), ("x", "y"), ("y", "x")] {
            let (sk_a, sk_b) = (key(a), key(b));
            let (k_a, k_b) = (sk_a.public_key(&secp), sk_b.public_key(&secp));
            let w = w_script(&k_a, &k_b);
            let digest = sighash(&spend(), &w, Amount::from_sat(100_000)).unwrap();
            let (sig_a, sig_b) = (sign(&sk_a, &digest), sign(&sk_b, &digest));
            let wit = witness(&w, &k_a, &sig_a, &k_b, &sig_b);

            assert_eq!(wit.len(), 4);
            assert!(wit.nth(0).unwrap().is_empty());
            assert_eq!(wit.nth(3).unwrap(), w.as_bytes());
            assert_eq!(sig_in_witness(&wit, &k_a, &k_b), Some(sig_a));
            assert_eq!(sig_in_witness(&wit, &k_b, &k_a), Some(sig_b));
            assert!(verify(&k_a, &digest, &sig_a) && verify(&k_b, &digest, &sig_b));
        }
    }

    #[test]
    fn spec_3_4_sighash_commits_to_value_and_sequence() {
        let w = w_script(
            &key("a").public_key(&Secp256k1::new()),
            &key("b").public_key(&Secp256k1::new()),
        );
        let tx = spend();
        let base = sighash(&tx, &w, Amount::from_sat(100_000)).unwrap();
        assert_ne!(base, sighash(&tx, &w, Amount::from_sat(100_001)).unwrap());
        let mut later = tx;
        later.input[0].sequence = Sequence::from_height(20);
        assert_ne!(
            base,
            sighash(&later, &w, Amount::from_sat(100_000)).unwrap()
        );
    }

    #[test]
    fn spec_3_5_r4_verify_rejects_wrong_key_digest_and_high_s() {
        let secp = Secp256k1::new();
        let sk = key("a");
        let digest = [9; 32];
        let sig = sign(&sk, &digest);
        assert!(verify(&sk.public_key(&secp), &digest, &sig));
        assert!(!verify(&key("b").public_key(&secp), &digest, &sig));
        assert!(!verify(&sk.public_key(&secp), &[8; 32], &sig));

        let mut compact = sig.serialize_compact();
        let n = bitcoin::secp256k1::constants::CURVE_ORDER;
        let mut s = [0u8; 32];
        s.copy_from_slice(&compact[32..]);
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let d = i16::from(n[i]) - i16::from(s[i]) - borrow;
            borrow = i16::from(d < 0);
            s[i] = d.rem_euclid(256) as u8;
        }
        compact[32..].copy_from_slice(&s);
        let high = Signature::from_compact(&compact).unwrap();
        assert!(!verify(&sk.public_key(&secp), &digest, &high));
    }

    #[test]
    fn spec_4_5_r1_share_recovered_from_published_redeem_witness() {
        let secp = Secp256k1::new();
        let (sk_a, sk_b, s_a) = (key("k_a"), key("k_b"), key("s_a"));
        let (k_a, k_b, y_a) = (
            sk_a.public_key(&secp),
            sk_b.public_key(&secp),
            s_a.public_key(&secp),
        );
        let w = w_script(&k_a, &k_b);
        let digest = sighash(&spend(), &w, Amount::from_sat(100_000)).unwrap();

        let esig_b = enc_sign(&sk_b, &y_a, &digest);
        let sig_b = decrypt(&s_a, &esig_b);
        let sig_a = sign(&sk_a, &digest);
        let published = witness(&w, &k_a, &sig_a, &k_b, &sig_b);

        let seen = sig_in_witness(&published, &k_b, &k_a).unwrap();
        assert!(verify(&k_b, &digest, &seen));
        assert_eq!(recover(&y_a, &seen, &esig_b), Some(s_a));
    }

    #[test]
    fn sig_in_witness_rejects_malformed_witnesses() {
        let secp = Secp256k1::new();
        let (k_a, k_b) = (key("a").public_key(&secp), key("b").public_key(&secp));
        assert_eq!(sig_in_witness(&Witness::new(), &k_a, &k_b), None);
        let junk = Witness::from_slice(&[vec![], vec![1, 2], vec![3], vec![4]]);
        assert_eq!(sig_in_witness(&junk, &k_a, &k_b), None);
    }
}

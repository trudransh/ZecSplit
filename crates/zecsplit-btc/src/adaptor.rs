//! ECDSA adaptor signatures (PROTOCOL_SPEC §3.5, §4.5-R1, §5.3-R3), wrapping `ecdsa_fun` so
//! callers only see `secp256k1` types. `msg` is always a BIP143 sighash.

use bitcoin::secp256k1::{PublicKey, SecretKey, ecdsa::Signature};
use ecdsa_fun::adaptor::{Adaptor, HashTranscript};
use ecdsa_fun::fun::{Point, Scalar, nonce};
use rand_chacha::ChaCha20Rng;
use sha2::Sha256;

pub use ecdsa_fun::adaptor::EncryptedSignature as EncSig;

// Deterministic nonces: the same key, adaptor point and sighash always give the same `EncSig`,
// so test vectors (T-014) are reproducible and signing needs no RNG.
type Engine = Adaptor<HashTranscript<Sha256, ChaCha20Rng>, nonce::Deterministic<Sha256>>;

pub fn enc_sign(sk: &SecretKey, y: &PublicKey, msg: &[u8; 32]) -> EncSig {
    Engine::default().encrypted_sign(&Scalar::from(*sk), &Point::from(*y), msg)
}

/// §3.5-R4: true iff `es` decrypts to a valid signature on `msg` under `pk`.
pub fn verify_enc(pk: &PublicKey, y: &PublicKey, msg: &[u8; 32], es: &EncSig) -> bool {
    Engine::default().verify_encrypted_signature(&Point::from(*pk), &Point::from(*y), msg, es)
}

/// Decrypts `es` with the adaptor secret. The result is always low-S, so it is standard on
/// Bitcoin. Publishing it reveals `y_secret` to whoever holds `es`.
pub fn decrypt(y_secret: &SecretKey, es: &EncSig) -> Signature {
    Engine::default()
        .decrypt_signature(&Scalar::from(*y_secret), es.clone())
        .into()
}

/// §4.5-R1, §5.3-R3: recovers the adaptor secret for `y` from the signature published in a
/// witness. `None` if `sig` is not the decryption of `es`.
pub fn recover(y: &PublicKey, sig: &Signature, es: &EncSig) -> Option<SecretKey> {
    Engine::default()
        .recover_decryption_key(&Point::from(*y), &(*sig).into(), es)
        .map(SecretKey::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::Witness;
    use bitcoin::secp256k1::{Message, Secp256k1};
    use sha2::Digest;

    fn key(seed: &str) -> SecretKey {
        SecretKey::from_slice(&Sha256::digest(seed)).unwrap()
    }

    fn msg(seed: &str) -> [u8; 32] {
        Sha256::digest(seed).into()
    }

    #[test]
    fn adaptor_round_trip_1000() {
        let secp = Secp256k1::new();
        for i in 0..1000 {
            let sk = key(&format!("sk{i}"));
            let y_secret = key(&format!("y{i}"));
            let (pk, y) = (sk.public_key(&secp), y_secret.public_key(&secp));
            let m = msg(&format!("m{i}"));

            let es = enc_sign(&sk, &y, &m);
            assert!(verify_enc(&pk, &y, &m, &es), "round {i}");

            let sig = decrypt(&y_secret, &es);
            secp.verify_ecdsa(&Message::from_digest(m), &sig, &pk)
                .unwrap();
            let mut low = sig;
            low.normalize_s();
            assert_eq!(low, sig, "round {i}: decrypted signature is high-S");

            assert_eq!(recover(&y, &sig, &es), Some(y_secret), "round {i}");
        }
    }

    #[test]
    fn recovers_from_the_witness_signature() {
        let secp = Secp256k1::new();
        let (sk, y_secret) = (key("sk"), key("y"));
        let y = y_secret.public_key(&secp);
        let m = msg("m");
        let es = enc_sign(&sk, &y, &m);

        let other = bitcoin::ecdsa::Signature::sighash_all(
            secp.sign_ecdsa(&Message::from_digest(m), &key("other")),
        );
        let ours = bitcoin::ecdsa::Signature::sighash_all(decrypt(&y_secret, &es));
        let witness = Witness::from_slice(&[vec![], other.to_vec(), ours.to_vec(), vec![0x52]]);

        let published = bitcoin::ecdsa::Signature::from_slice(witness.nth(2).unwrap()).unwrap();
        assert_eq!(recover(&y, &published.signature, &es), Some(y_secret));
        let unrelated = bitcoin::ecdsa::Signature::from_slice(witness.nth(1).unwrap()).unwrap();
        assert_eq!(recover(&y, &unrelated.signature, &es), None);
    }

    #[test]
    fn spec_3_5_r4_verify_enc_rejects_mismatches() {
        let secp = Secp256k1::new();
        let (sk, y_secret) = (key("sk"), key("y"));
        let (pk, y) = (sk.public_key(&secp), y_secret.public_key(&secp));
        let wrong = key("wrong").public_key(&secp);
        let m = msg("m");
        let es = enc_sign(&sk, &y, &m);

        assert!(verify_enc(&pk, &y, &m, &es));
        assert!(!verify_enc(&wrong, &y, &m, &es), "wrong signer");
        assert!(!verify_enc(&pk, &wrong, &m, &es), "wrong adaptor point");
        assert!(!verify_enc(&pk, &y, &msg("other"), &es), "wrong sighash");
    }

    #[test]
    fn recover_rejects_the_wrong_adaptor_point() {
        let secp = Secp256k1::new();
        let (sk, y_secret) = (key("sk"), key("y"));
        let y = y_secret.public_key(&secp);
        let es = enc_sign(&sk, &y, &msg("m"));
        let sig = decrypt(&y_secret, &es);

        assert_eq!(recover(&key("wrong").public_key(&secp), &sig, &es), None);
    }

    #[test]
    fn enc_sign_is_deterministic() {
        let secp = Secp256k1::new();
        let y = key("y").public_key(&secp);
        let a = enc_sign(&key("sk"), &y, &msg("m"));
        let b = enc_sign(&key("sk"), &y, &msg("m"));
        assert_eq!(decrypt(&key("y"), &a), decrypt(&key("y"), &b));
    }
}

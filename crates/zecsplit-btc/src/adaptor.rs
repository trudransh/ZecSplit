//! ECDSA adaptor signatures (PROTOCOL_SPEC §3.5, §4.5-R1, §5.3-R3), wrapping `ecdsa_fun` so
//! callers only see `secp256k1` types.

use bitcoin::secp256k1::{PublicKey, SecretKey, ecdsa::Signature};
use ecdsa_fun::adaptor::{Adaptor, HashTranscript};
use ecdsa_fun::fun::{Point, Scalar, nonce};
use rand_chacha::ChaCha20Rng;
use sha2::Sha256;

pub use ecdsa_fun::adaptor::EncryptedSignature as EncSig;

type Engine = Adaptor<HashTranscript<Sha256, ChaCha20Rng>, nonce::Deterministic<Sha256>>;

/// §3.5-R2: encrypts an ECDSA signature on `msg` under the adaptor point `y`.
pub fn enc_sign(sk: &SecretKey, y: &PublicKey, msg: &[u8; 32]) -> EncSig {
    Engine::default().encrypted_sign(&Scalar::from(*sk), &Point::from(*y), msg)
}

/// §3.5-R4: true iff `es` decrypts to a valid signature on `msg` under `pk`.
pub fn verify_enc(pk: &PublicKey, y: &PublicKey, msg: &[u8; 32], es: &EncSig) -> bool {
    Engine::default().verify_encrypted_signature(&Point::from(*pk), &Point::from(*y), msg, es)
}

/// Decrypts `es` with the adaptor secret.
pub fn decrypt(y_secret: &SecretKey, es: &EncSig) -> Signature {
    Engine::default()
        .decrypt_signature(&Scalar::from(*y_secret), es.clone())
        .into()
}

/// §4.5-R1, §5.3-R3: recovers the adaptor secret for `y` from the signature published in a
/// witness.
pub fn recover(y: &PublicKey, sig: &Signature, es: &EncSig) -> Option<SecretKey> {
    Engine::default()
        .recover_decryption_key(&Point::from(*y), &(*sig).into(), es)
        .map(SecretKey::from)
}

/// The 162-byte wire form of an adaptor signature (INTERFACES §2).
pub fn encsig_to_bytes(es: &EncSig) -> Vec<u8> {
    hex::decode(es.to_string()).unwrap_or_default()
}

/// Parses the 162-byte wire form; `None` if malformed.
pub fn encsig_from_bytes(bytes: &[u8]) -> Option<EncSig> {
    if bytes.len() != 162 {
        return None;
    }
    hex::encode(bytes).parse().ok()
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
    fn encsig_bytes_round_trip_and_reject_garbage() {
        let es = enc_sign(
            &key("sk"),
            &key("y").public_key(&Secp256k1::new()),
            &msg("m"),
        );
        let bytes = encsig_to_bytes(&es);
        assert_eq!(bytes.len(), 162);
        assert_eq!(encsig_from_bytes(&bytes), Some(es));
        assert_eq!(encsig_from_bytes(&bytes[..161]), None);
        assert_eq!(encsig_from_bytes(&[0xff; 162]), None);
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

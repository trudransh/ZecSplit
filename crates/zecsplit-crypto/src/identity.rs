//! The maker identity key (PROTOCOL_SPEC §2-R1, §2-R2): a long-term BIP340 key that signs
//! every quote. The engine pins the maker's public key, so a quote board or a man in the
//! middle can relay quotes but never forge one (THREAT_MODEL T23, ADV-14).

use secp256k1::{Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey, schnorr};

use crate::MakerSecret;
use crate::hash::{HK_QUOTE_V1, h256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuoteSigError {
    #[error("not a valid secp256k1 secret key")]
    InvalidSecret,
    #[error("not a valid x-only public key")]
    InvalidKey,
    #[error("quote signature does not verify under the pinned maker key")]
    BadSignature,
    #[error("OS random number generator failed")]
    Rng,
}

/// `H256(HK_quote_v1, canonical_json(quote))`, the message the maker signs (§2-R2).
pub fn quote_digest(canonical_json: &[u8]) -> [u8; 32] {
    h256(HK_QUOTE_V1, canonical_json)
}

/// The x-only public key a taker pins for this maker.
pub fn maker_public_key(secret: &MakerSecret) -> Result<[u8; 32], QuoteSigError> {
    Ok(keypair(secret)?.x_only_public_key().0.serialize())
}

pub fn sign_quote(secret: &MakerSecret, canonical_json: &[u8]) -> Result<[u8; 64], QuoteSigError> {
    let mut aux = [0; 32];
    getrandom::fill(&mut aux).map_err(|_| QuoteSigError::Rng)?;
    let msg = Message::from_digest(quote_digest(canonical_json));
    Ok(Secp256k1::signing_only()
        .sign_schnorr_with_aux_rand(&msg, &keypair(secret)?, &aux)
        .serialize())
}

pub fn verify_quote(
    maker_key: &[u8; 32],
    canonical_json: &[u8],
    sig: &[u8; 64],
) -> Result<(), QuoteSigError> {
    let key = XOnlyPublicKey::from_slice(maker_key).map_err(|_| QuoteSigError::InvalidKey)?;
    let sig = schnorr::Signature::from_slice(sig).map_err(|_| QuoteSigError::BadSignature)?;
    let msg = Message::from_digest(quote_digest(canonical_json));
    Secp256k1::verification_only()
        .verify_schnorr(&sig, &msg, &key)
        .map_err(|_| QuoteSigError::BadSignature)
}

fn keypair(secret: &MakerSecret) -> Result<Keypair, QuoteSigError> {
    let sk =
        SecretKey::from_slice(secret.expose_secret()).map_err(|_| QuoteSigError::InvalidSecret)?;
    Ok(Keypair::from_secret_key(&Secp256k1::signing_only(), &sk))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_2_r2_quote_signature_round_trip() {
        let maker = MakerSecret::from_bytes([5; 32]);
        let key = maker_public_key(&maker).unwrap();
        let sig = sign_quote(&maker, b"{\"q\":1}").unwrap();
        assert_eq!(verify_quote(&key, b"{\"q\":1}", &sig), Ok(()));
    }

    #[test]
    fn adv_14_quote_from_another_key_or_edited_is_rejected() {
        let maker = MakerSecret::from_bytes([5; 32]);
        let impostor = MakerSecret::from_bytes([6; 32]);
        let pinned = maker_public_key(&maker).unwrap();
        let forged = sign_quote(&impostor, b"{\"q\":1}").unwrap();
        assert_eq!(
            verify_quote(&pinned, b"{\"q\":1}", &forged),
            Err(QuoteSigError::BadSignature)
        );
        let sig = sign_quote(&maker, b"{\"q\":1}").unwrap();
        assert_eq!(
            verify_quote(&pinned, b"{\"q\":2}", &sig),
            Err(QuoteSigError::BadSignature)
        );
    }
}

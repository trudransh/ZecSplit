//! Cross-group DLEq between Pallas and secp256k1. PROTOCOL_SPEC §3.2-R2 and §3.2-R3.2.
//!
//! A thin wrapper over the vendored Serai `dleq` (see `vendor/dleq/REVIEW.md`). `prove` draws
//! a share `s` valid in both groups and proves `S^P = [s]·SpendAuthG` and `S^S = [s]·G`;
//! `verify` checks such a proof against a given `(S^P, S^S)`.
//!
//! The transcript is flexible-transcript's `RecommendedTranscript` (BLAKE2b-512, every item
//! length-prefixed): name `HK_dleq_v1`, then message `swap_id` (32 bytes), then message
//! `role_byte` (1 byte). `dleq` then appends its generators and both points.

use dleq::cross_group::{CompromiseLinearDLEq, Generators};
use flexible_transcript::{RecommendedTranscript, Transcript};
use k256::elliptic_curve::hash2curve::{ExpandMsgXmd, GroupDigest};
use k256::{ProjectivePoint, Secp256k1};
use pasta_curves_05::arithmetic::CurveExt;
use pasta_curves_05::group::{GroupEncoding, ff::PrimeField};
use pasta_curves_05::pallas;
use rand_core::{CryptoRng, OsRng, RngCore};
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;

/// Upstream's recommended variant: two bits per ring, batch-verified.
type Proof = CompromiseLinearDLEq<pallas::Point, ProjectivePoint>;

/// Hash-to-curve domain for both alternate generators (§3.2-R2), with an empty message.
const ALT_DOMAIN: &str = "HK_dleq_alt_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DleqError {
    #[error("malformed DLEq proof: the bytes aren't exactly one proof")]
    Malformed,
    #[error("invalid DLEq proof, or it proves a different pair of points")]
    Invalid,
    /// Generator inputs are constants, so this can't happen.
    #[error("DLEq generator derivation failed")]
    Generators,
}

/// The proven share `s` as its Pallas scalar encoding (little-endian). dleq also returns it
/// as a secp256k1 scalar; `prove` checks that both are the same integer, so the big-endian
/// secp256k1 form is just these bytes reversed. No `Debug`, `Display`, `Clone` or
/// `Serialize`; zeroized on drop.
pub struct DleqSecret {
    pallas: Zeroizing<[u8; 32]>,
}

impl DleqSecret {
    /// `s` as a Pallas scalar: `F_q` canonical encoding, little-endian.
    pub fn pallas(&self) -> &[u8; 32] {
        &self.pallas
    }
}

/// The share, `S^P` (32-byte Pallas encoding), `S^S` (SEC1 compressed) and the proof bytes.
pub type DleqOutput = (DleqSecret, [u8; 32], [u8; 33], Vec<u8>);

/// Primaries are `SpendAuthG` (Orchard, §1) and secp256k1's `G`. Pallas uses the vendored
/// pasta 0.5, so `SpendAuthG` is rebuilt here; a test pins it to reddsa's basepoint.
fn generators() -> Result<(Generators<pallas::Point>, Generators<ProjectivePoint>), DleqError> {
    let spend_auth_g = pallas::Point::hash_to_curve("z.cash:Orchard")(b"G");
    let pallas_alt = pallas::Point::hash_to_curve(ALT_DOMAIN)(b"");
    let secp_alt =
        Secp256k1::hash_from_bytes::<ExpandMsgXmd<Sha256>>(&[b""], &[ALT_DOMAIN.as_bytes()])
            .map_err(|_| DleqError::Generators)?;
    Ok((
        Generators::new(spend_auth_g, pallas_alt).ok_or(DleqError::Generators)?,
        Generators::new(ProjectivePoint::GENERATOR, secp_alt).ok_or(DleqError::Generators)?,
    ))
}

fn transcript(swap_id: &[u8; 32], role_byte: u8) -> RecommendedTranscript {
    let mut t = RecommendedTranscript::new(b"HK_dleq_v1");
    t.append_message(b"swap_id", swap_id);
    t.append_message(b"role_byte", [role_byte]);
    t
}

/// §3.2-R2: draw a share valid in both groups and prove its two public points.
pub fn prove<R: RngCore + CryptoRng>(
    rng: &mut R,
    swap_id: &[u8; 32],
    role_byte: u8,
) -> Result<DleqOutput, DleqError> {
    let generators = generators()?;
    // dleq derives the scalar from a digest's output, truncated to the shared capacity (254 bits).
    let mut seed = Zeroizing::new([0u8; 32]);
    rng.fill_bytes(seed.as_mut());
    let (proof, (s_p, s_s)) = Proof::prove(
        rng,
        &mut transcript(swap_id, role_byte),
        generators,
        Sha512::new_with_prefix(seed.as_ref()),
    );

    let point_p = (generators.0.primary * *s_p).to_bytes();
    let mut point_s = [0u8; 33];
    point_s.copy_from_slice(&(generators.1.primary * *s_s).to_bytes());
    let pallas = Zeroizing::new(s_p.to_repr());
    let mut secp = Zeroizing::new([0u8; 32]);
    secp.copy_from_slice(&s_s.to_repr());
    // One integer in both groups (§1), so reversing the bytes converts between encodings.
    if !pallas.iter().rev().eq(secp.iter()) {
        return Err(DleqError::Invalid);
    }
    let secret = DleqSecret { pallas };

    let mut bytes = Vec::new();
    // Writing into a Vec can't fail.
    proof.write(&mut bytes).map_err(|_| DleqError::Malformed)?;
    Ok((secret, point_p, point_s, bytes))
}

/// §3.2-R3.2: `proof` shows `pallas` and `secp` share one discrete log, under the prover's
/// `swap_id` and `role_byte`.
pub fn verify(
    swap_id: &[u8; 32],
    role_byte: u8,
    pallas: &[u8; 32],
    secp: &[u8; 33],
    proof: &[u8],
) -> Result<(), DleqError> {
    let mut reader = proof;
    let proof = Proof::read(&mut reader).map_err(|_| DleqError::Malformed)?;
    if !reader.is_empty() {
        return Err(DleqError::Malformed);
    }
    let (point_p, point_s) = proof
        .verify(
            &mut OsRng,
            &mut transcript(swap_id, role_byte),
            generators()?,
        )
        .map_err(|_| DleqError::Invalid)?;
    if point_p.to_bytes() != *pallas || point_s.to_bytes().as_slice() != secp.as_slice() {
        return Err(DleqError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use reddsa::orchard::SpendAuth;
    use reddsa::{SigningKey, VerificationKey};

    use super::*;

    const SWAP_ID: [u8; 32] = [7; 32];

    /// Proving is slow in debug builds, so the tests share two proofs.
    static A: LazyLock<DleqOutput> = LazyLock::new(|| prove(&mut OsRng, &SWAP_ID, 0).unwrap());
    static B: LazyLock<DleqOutput> = LazyLock::new(|| prove(&mut OsRng, &SWAP_ID, 0).unwrap());

    /// `[s]·SpendAuthG` through reddsa, the crate Orchard signs with (`ak = [ask]·SpendAuthG`).
    fn reddsa_ak(s: &[u8; 32]) -> [u8; 32] {
        VerificationKey::from(&SigningKey::<SpendAuth>::from_bytes(s).unwrap()).into()
    }

    #[test]
    fn spec_3_2_r2_dleq_round_trip() {
        let (_, p, s, proof) = &*A;
        assert_eq!(verify(&SWAP_ID, 0, p, s, proof), Ok(()));
    }

    #[test]
    fn spec_3_2_r2_primary_is_spend_auth_g() {
        let mut one = [0u8; 32];
        one[0] = 1;
        assert_eq!(generators().unwrap().0.primary.to_bytes(), reddsa_ak(&one));
    }

    /// Pins the `HK_dleq_alt_v1` hash-to-curve outputs, so a changed domain can't slip through.
    #[test]
    fn spec_3_2_r2_alt_generators_are_pinned() {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let (p, s) = generators().unwrap();
        assert_eq!(
            hex(&p.alt.to_bytes()),
            "067d9d5b93a0fa23ad3960f1540e8b53f57567a15834925f32237ee70754700e"
        );
        assert_eq!(
            hex(&s.alt.to_bytes()),
            "035f2906d54fe7a6a5e164c947cad320bc35e51aef9c77a9de9c2555d0951f835e"
        );
    }

    #[test]
    fn spec_3_2_r2_points_match_returned_scalars() {
        let (secret, p, s, _) = &*A;
        assert_eq!(reddsa_ak(secret.pallas()), *p);

        let mut be = *secret.pallas();
        be.reverse();
        let s_s = k256::Scalar::from_repr(be.into()).unwrap();
        assert_eq!(
            (ProjectivePoint::GENERATOR * s_s).to_bytes().as_slice(),
            s.as_slice()
        );

        // Below dleq's mutual capacity of 2^254 (§1, ADR-028).
        assert!(secret.pallas()[31] < 0x40);
    }

    /// T-015 meets T-010: two real DLEq shares combine into a valid lock key (§3.3-R1).
    #[test]
    fn spec_3_3_r1_dleq_shares_combine_into_ak() {
        let (secret_a, p_a, s_a, _) = &*A;
        let (secret_b, p_b, s_b, _) = &*B;
        let key = crate::shared_ak(
            &crate::SharePub {
                pallas: *p_a,
                secp: *s_a,
            },
            &crate::SharePub {
                pallas: *p_b,
                secp: *s_b,
            },
        )
        .unwrap();
        let ask = crate::shared_ask(
            &crate::Share::from_bytes(*secret_a.pallas()),
            &crate::Share::from_bytes(*secret_b.pallas()),
            key.sigma,
        )
        .unwrap();
        assert_eq!(reddsa_ak(ask.expose_secret()), key.ak);
    }

    #[test]
    fn spec_3_2_r3_2_rejects_tampered_proof() {
        let (_, p, s, proof) = &*A;
        for i in [0, proof.len() / 2, proof.len() - 1] {
            let mut bad = proof.clone();
            bad[i] ^= 1;
            assert!(verify(&SWAP_ID, 0, p, s, &bad).is_err(), "flipped byte {i}");
        }
        let mut long = proof.clone();
        long.push(0);
        assert_eq!(verify(&SWAP_ID, 0, p, s, &long), Err(DleqError::Malformed));
        assert_eq!(
            verify(&SWAP_ID, 0, p, s, &proof[..proof.len() - 1]),
            Err(DleqError::Malformed)
        );
    }

    #[test]
    fn spec_3_2_r3_2_rejects_mismatched_pair() {
        let (_, p_a, s_a, proof_a) = &*A;
        let (_, p_b, s_b, proof_b) = &*B;
        assert_eq!(
            verify(&SWAP_ID, 0, p_a, s_b, proof_a),
            Err(DleqError::Invalid)
        );
        assert_eq!(
            verify(&SWAP_ID, 0, p_a, s_b, proof_b),
            Err(DleqError::Invalid)
        );
        assert_eq!(
            verify(&SWAP_ID, 0, p_b, s_a, proof_a),
            Err(DleqError::Invalid)
        );
    }

    #[test]
    fn spec_3_2_r3_2_rejects_wrong_transcript() {
        let (_, p, s, proof) = &*A;
        assert_eq!(verify(&[8; 32], 0, p, s, proof), Err(DleqError::Invalid));
        assert_eq!(verify(&SWAP_ID, 1, p, s, proof), Err(DleqError::Invalid));
    }
}

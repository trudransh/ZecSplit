//! The two-party Zcash spend key (PROTOCOL_SPEC §3.3-R1, §3.3-R2).
//!
//! The lock address is controlled by `ask = σ·(s_a + s_b)`. Each party holds one share;
//! whoever learns the other's share (from a Bitcoin adaptor signature) can sweep.

use pasta_curves::group::ff::{FromUniformBytes, PrimeField};
use pasta_curves::group::{Group, GroupEncoding};
use pasta_curves::pallas;
use zeroize::Zeroizing;

use crate::hash::{HK_NK_V1, HK_RIVK_V1, h512};
use crate::{Ask, Share};

/// A party's public share (§1): `S^P = [s]·SpendAuthG` on Pallas and `S^S = [s]·G` on
/// secp256k1 (SEC1 compressed). `S^S` is also that party's Bitcoin adaptor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharePub {
    pub pallas: [u8; 32],
    pub secp: [u8; 33],
}

/// The sign applied to `S_a + S_b` so that `ak` has `ỹ = 0`, as Orchard requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sigma {
    Plus,
    Minus,
}

/// The lock's spend validating key `ak = [σ]·(S_a + S_b)` and the `σ` that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedKey {
    pub ak: [u8; 32],
    pub sigma: Sigma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("public share is not a valid, non-identity Pallas point")]
    InvalidPoint,
    #[error("S_a + S_b is the identity, so the shares cancel out")]
    IdentitySum,
    #[error("share is not an integer in [1, 2^252)")]
    InvalidShare,
}

/// §3.3-R1: `P = S_a^P + S_b^P`, `σ = +1` if `ỹ(P) = 0` else `−1`, and `ak = [σ]·P`.
pub fn shared_ak(a: &SharePub, b: &SharePub) -> Result<SharedKey, KeyError> {
    let sum = pallas_point(&a.pallas)? + pallas_point(&b.pallas)?;
    if bool::from(sum.is_identity()) {
        return Err(KeyError::IdentitySum);
    }
    // The top bit of a Pallas point encoding is ỹ, the sign of its y-coordinate.
    let (sigma, ak) = if sum.to_bytes()[31] >> 7 == 0 {
        (Sigma::Plus, sum)
    } else {
        (Sigma::Minus, -sum)
    };
    Ok(SharedKey {
        ak: ak.to_bytes(),
        sigma,
    })
}

/// §3.3-R1: `ask = σ·(s_a + s_b) mod q`. The shares are never negated on their own; `σ`
/// is applied only to the sum, at signing time.
pub fn shared_ask(a: &Share, b: &Share, sigma: Sigma) -> Result<Ask, KeyError> {
    let sum = Zeroizing::new(*share_scalar(a)? + *share_scalar(b)?);
    let ask = Zeroizing::new(match sigma {
        Sigma::Plus => *sum,
        Sigma::Minus => -*sum,
    });
    Ok(Ask::from_bytes(ask.to_repr()))
}

/// §3.3-R2 (ADR-006): `nk` and `rivk` come from the transcript `T`, so neither party
/// chooses or biases the viewing keys and both derive the same lock address.
pub fn derive_nk_rivk(transcript: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let nk = pallas::Base::from_uniform_bytes(&h512(HK_NK_V1, transcript));
    let rivk = pallas::Scalar::from_uniform_bytes(&h512(HK_RIVK_V1, transcript));
    (nk.to_repr(), rivk.to_repr())
}

fn pallas_point(bytes: &[u8; 32]) -> Result<pallas::Point, KeyError> {
    Option::<pallas::Point>::from(pallas::Point::from_bytes(bytes))
        .filter(|p| !bool::from(p.is_identity()))
        .ok_or(KeyError::InvalidPoint)
}

fn share_scalar(share: &Share) -> Result<Zeroizing<pallas::Scalar>, KeyError> {
    let bytes = share.expose_secret();
    // §1: a share is an integer in [1, 2^252), so its top four bits are clear.
    if bytes[31] & 0xf0 != 0 || bytes == &[0; 32] {
        return Err(KeyError::InvalidShare);
    }
    Option::from(pallas::Scalar::from_repr(*bytes))
        .map(Zeroizing::new)
        .ok_or(KeyError::InvalidShare)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use reddsa::orchard::SpendAuth;

    /// `[s]·SpendAuthG`, computed by `reddsa` (independent of the code under test).
    fn spend_auth_point(s: &[u8; 32]) -> [u8; 32] {
        let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(s).unwrap();
        reddsa::VerificationKey::from(&sk).into()
    }

    /// A deterministic share in [1, 2^252) and its public Pallas share. The secp half is
    /// left zero: nothing in §3.3 reads it.
    fn share(seed: u8) -> (Share, SharePub) {
        let mut s: [u8; 32] = h512(b"test_share\0\0\0\0\0\0", &[seed])[..32]
            .try_into()
            .unwrap();
        s[31] &= 0x0f;
        let public = SharePub {
            pallas: spend_auth_point(&s),
            secp: [0; 33],
        };
        (Share::from_bytes(s), public)
    }

    fn first_pair_with(sigma: Sigma) -> (Share, SharePub, Share, SharePub) {
        (0..=u8::MAX / 2)
            .map(|i| (share(2 * i), share(2 * i + 1)))
            .find(|((_, a), (_, b))| shared_ak(a, b).unwrap().sigma == sigma)
            .map(|((sa, a), (sb, b))| (sa, a, sb, b))
            .unwrap()
    }

    fn assert_ask_matches_ak(sa: &Share, sb: &Share, key: &SharedKey) {
        let ask = shared_ask(sa, sb, key.sigma).unwrap();
        assert_eq!(spend_auth_point(ask.expose_secret()), key.ak);
        assert_eq!(key.ak[31] >> 7, 0, "ak must have ỹ = 0");
    }

    #[test]
    fn spec_3_3_r1_sigma_positive() {
        let (sa, a, sb, b) = first_pair_with(Sigma::Plus);
        let key = shared_ak(&a, &b).unwrap();
        let sum = pallas_point(&a.pallas).unwrap() + pallas_point(&b.pallas).unwrap();
        assert_eq!(key.ak, sum.to_bytes());
        assert_ask_matches_ak(&sa, &sb, &key);
    }

    #[test]
    fn spec_3_3_r1_sigma_negative() {
        let (sa, a, sb, b) = first_pair_with(Sigma::Minus);
        let key = shared_ak(&a, &b).unwrap();
        let sum = pallas_point(&a.pallas).unwrap() + pallas_point(&b.pallas).unwrap();
        assert_eq!(sum.to_bytes()[31] >> 7, 1, "this case has ỹ(S_a + S_b) = 1");
        assert_eq!(key.ak, (-sum).to_bytes());
        assert_ask_matches_ak(&sa, &sb, &key);
    }

    #[test]
    fn spec_3_3_r1_rejects_shares_that_cancel() {
        let (_, a) = share(1);
        let negated = SharePub {
            pallas: (-pallas_point(&a.pallas).unwrap()).to_bytes(),
            ..a
        };
        assert_eq!(shared_ak(&a, &negated), Err(KeyError::IdentitySum));
    }

    #[test]
    fn spec_3_3_r1_rejects_identity_and_invalid_points() {
        let (_, a) = share(1);
        let identity = SharePub {
            pallas: [0; 32],
            ..a
        };
        assert_eq!(shared_ak(&a, &identity), Err(KeyError::InvalidPoint));
        let off_curve = SharePub {
            pallas: [0xff; 32],
            ..a
        };
        assert_eq!(shared_ak(&a, &off_curve), Err(KeyError::InvalidPoint));
    }

    #[test]
    fn spec_1_rejects_shares_outside_range() {
        let (good, _) = share(1);
        let mut too_big = *good.expose_secret();
        too_big[31] |= 0x10;
        let big = Share::from_bytes(too_big);
        assert_eq!(
            shared_ask(&big, &good, Sigma::Plus).err(),
            Some(KeyError::InvalidShare)
        );
        let zero = Share::from_bytes([0; 32]);
        assert_eq!(
            shared_ask(&zero, &good, Sigma::Plus).err(),
            Some(KeyError::InvalidShare)
        );
    }

    #[test]
    fn spec_3_3_r2_viewing_keys_follow_the_transcript() {
        let (nk, rivk) = derive_nk_rivk(&[1; 32]);
        assert_eq!(derive_nk_rivk(&[1; 32]), (nk, rivk));
        let (nk2, rivk2) = derive_nk_rivk(&[2; 32]);
        assert_ne!(nk, nk2);
        assert_ne!(rivk, rivk2);
        assert_ne!(nk, rivk, "the two tags must separate the domains");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10_000))]

        /// TEST_PLAN §1: the σ rule over random pairs. `ak` always has ỹ = 0 and always
        /// equals `[shared_ask]·SpendAuthG`.
        #[test]
        fn spec_3_3_r1_sigma_rule_holds_for_random_pairs(
            mut sa in any::<[u8; 32]>(),
            mut sb in any::<[u8; 32]>(),
        ) {
            sa[31] &= 0x0f;
            sb[31] &= 0x0f;
            prop_assume!(sa != [0; 32] && sb != [0; 32]);
            let a = SharePub { pallas: spend_auth_point(&sa), secp: [0; 33] };
            let b = SharePub { pallas: spend_auth_point(&sb), secp: [0; 33] };
            let key = shared_ak(&a, &b).unwrap();
            let ask = shared_ask(&Share::from_bytes(sa), &Share::from_bytes(sb), key.sigma).unwrap();
            prop_assert_eq!(key.ak[31] >> 7, 0);
            prop_assert_eq!(spend_auth_point(ask.expose_secret()), key.ak);
        }
    }
}

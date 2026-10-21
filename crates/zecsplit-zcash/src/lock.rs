//! The lock address (PROTOCOL_SPEC §3.3-R3, §3.3-R4): an Orchard-only Unified Address whose
//! notes land in the Ironwood pool and can be spent only with `ask = σ·(s_a + s_b)`.

use orchard::keys::{FullViewingKey, Scope};
use zcash_address::unified::{Encoding, Fvk, Revision, Ufvk, Uitem};
use zcash_keys::address::UnifiedAddress;
use zcash_protocol::consensus::{BlockHeight, Parameters};
use zcash_protocol::local_consensus::LocalNetwork;

/// The lock's viewing material.
pub struct LockKeys {
    pub ufvk: String,
    pub ua: String,
    pub fvk_bytes: [u8; 96],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LockKeyError {
    #[error("ak ‖ nk ‖ rivk is not a valid Orchard full viewing key")]
    InvalidFvk,
    #[error("could not encode the lock keys: {0}")]
    Encoding(String),
}

/// §3.3-R3/R4: `FVK = from_bytes(ak ‖ nk ‖ rivk)`, the lock UA at index 0 (external scope,
/// Orchard receiver only) and the Orchard-only UFVK.
pub fn lock_keys<P: Parameters>(
    params: &P,
    ak: &[u8; 32],
    nk: &[u8; 32],
    rivk: &[u8; 32],
) -> Result<LockKeys, LockKeyError> {
    let mut fvk_bytes = [0; 96];
    fvk_bytes[..32].copy_from_slice(ak);
    fvk_bytes[32..64].copy_from_slice(nk);
    fvk_bytes[64..].copy_from_slice(rivk);
    let fvk = FullViewingKey::from_bytes(&fvk_bytes).ok_or(LockKeyError::InvalidFvk)?;

    let ua = UnifiedAddress::from_receivers(
        Some(fvk.address_at(0u32, Scope::External)),
        None,
        None,
        None,
        None,
    )
    .ok_or_else(|| LockKeyError::Encoding("no receiver".into()))?
    .encode(params);
    let ufvk = Ufvk::try_from_items(Revision::R0, vec![Uitem::Data(Fvk::Orchard(fvk_bytes))])
        .map_err(|e| LockKeyError::Encoding(e.to_string()))?
        .encode(&params.network_type());

    Ok(LockKeys {
        ufvk,
        ua,
        fvk_bytes,
    })
}

/// Consensus parameters of the devnet: every upgrade through NU7 active from height 1.
pub fn regtest() -> LocalNetwork {
    let h = Some(BlockHeight::from_u32(1));
    LocalNetwork {
        overwinter: h,
        sapling: h,
        blossom: h,
        heartwood: h,
        canopy: h,
        nu5: h,
        nu6: h,
        nu6_1: h,
        nu6_2: h,
        nu6_3: h,
        nu7: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reddsa::orchard::SpendAuth;
    use zcash_address::unified::Typecode;
    use zecsplit_crypto::{SharePub, derive_nk_rivk, shared_ak};

    fn share_pub(seed: u8) -> SharePub {
        let mut s = [seed; 32];
        s[31] &= 0x0f;
        let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(&s).unwrap();
        SharePub {
            pallas: reddsa::VerificationKey::from(&sk).into(),
            secp: [0; 33],
        }
    }

    fn keys_for(a: u8, b: u8, transcript: &[u8; 32]) -> Result<LockKeys, LockKeyError> {
        let key = shared_ak(&share_pub(a), &share_pub(b)).unwrap();
        let (nk, rivk) = derive_nk_rivk(transcript);
        lock_keys(&regtest(), &key.ak, &nk, &rivk)
    }

    #[test]
    fn spec_3_3_r3_invalid_fvk_aborts() {
        let key = shared_ak(&share_pub(1), &share_pub(2)).unwrap();
        let (nk, rivk) = derive_nk_rivk(&[9; 32]);
        assert!(lock_keys(&regtest(), &key.ak, &nk, &rivk).is_ok());

        let mut wrong_sign = key.ak;
        wrong_sign[31] |= 0x80;
        assert_eq!(
            lock_keys(&regtest(), &wrong_sign, &nk, &rivk).err(),
            Some(LockKeyError::InvalidFvk)
        );
        assert_eq!(
            lock_keys(&regtest(), &[0; 32], &nk, &rivk).err(),
            Some(LockKeyError::InvalidFvk)
        );
    }

    #[test]
    fn spec_3_3_r4_lock_ua_has_only_an_orchard_receiver() {
        let keys = keys_for(1, 2, &[9; 32]).unwrap();
        assert!(keys.ua.starts_with("uregtest1"));
        let ua = UnifiedAddress::decode(&regtest(), &keys.ua).unwrap();
        assert_eq!(ua.receiver_types(), vec![Typecode::ORCHARD]);
        assert!(keys.ufvk.starts_with("uviewregtest1"));

        let ufvk = zcash_keys::keys::UnifiedFullViewingKey::decode(&regtest(), &keys.ufvk).unwrap();
        assert!(ufvk.orchard().is_some());
        assert!(ufvk.sapling().is_none());
        assert_eq!(ufvk.orchard().unwrap().to_bytes(), keys.fvk_bytes);
    }

    /// Regression vector (PROTOCOL_SPEC §9, formalized in T-014): shares seeded 1 and 2, T =
    /// [9; 32].
    #[test]
    fn spec_9_lock_address_regression_vector() {
        assert_eq!(
            keys_for(1, 2, &[9; 32]).unwrap().ua,
            "uregtest1pg82taqfut2rek9v3c3fnekaxudxu4j48zurq49gfdtl3v4mttrukm362h4wap0sd40n60ph89e8863rw22ja8ew855kh9rhssd0gvfu"
        );
    }

    #[test]
    fn spec_3_3_lock_keys_are_deterministic_and_bound_to_the_transcript() {
        let ua = |a, b, t: [u8; 32]| keys_for(a, b, &t).unwrap().ua;
        assert_eq!(ua(1, 2, [9; 32]), ua(1, 2, [9; 32]));
        assert_eq!(
            ua(1, 2, [9; 32]),
            ua(2, 1, [9; 32]),
            "share order must not matter"
        );
        assert_ne!(ua(1, 2, [9; 32]), ua(1, 2, [8; 32]));
        assert_ne!(ua(1, 2, [9; 32]), ua(1, 3, [9; 32]));
    }
}

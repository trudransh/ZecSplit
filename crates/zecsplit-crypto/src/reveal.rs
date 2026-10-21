//! Commit-reveal with proofs of knowledge (PROTOCOL_SPEC §3.1, §3.2).
//!
//! Committing before revealing stops a party from choosing its share as a function of the
//! other's (rogue key, THREAT_MODEL T1). The DLEq ties each Pallas share to its secp256k1
//! twin (T2). The three proofs of knowledge show the revealer knows the discrete log of
//! every key it reveals, closing the gap in the original proof (ADR-005). Every hash and
//! signed message is bound to `swap_id` and the role (T13).

use pasta_curves::group::{Group, GroupEncoding};
use pasta_curves::pallas;
use rand_core::{CryptoRng, RngCore};
use reddsa::orchard::SpendAuth;
use secp256k1::{Keypair, Message, PublicKey, Secp256k1, SecretKey, ecdsa, schnorr};

use crate::dleq;
use crate::hash::{HK_COMMIT_V1, HK_POK_V1, h256};
use crate::{BtcSecret, Share, SharePub};

/// The party a message or proof belongs to. Alice is the taker, Bob the maker (§1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Alice,
    Bob,
}

impl Role {
    /// `role_byte` in every hash domain.
    pub fn byte(self) -> u8 {
        match self {
            Role::Alice => 0,
            Role::Bob => 1,
        }
    }
}

/// `C_x` (§3.1-R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Commitment(pub [u8; 32]);

/// Everything a party reveals (§3.2-R1). All public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reveal {
    pub share_pub: SharePub,
    /// `K_x`, the party's Bitcoin 2-of-2 key (SEC1 compressed).
    pub btc_key: [u8; 33],
    pub nonce: [u8; 32],
    pub dleq: Vec<u8>,
    /// RedPallas SpendAuth signature under `S^P`.
    pub pok_p: [u8; 64],
    /// BIP340 signature under `S^S`.
    pub pok_s: [u8; 64],
    /// ECDSA signature under `K` (compact encoding).
    pub pok_k: [u8; 64],
    /// Alice's redeem/punish address, or Bob's refund address.
    pub payout_addr: String,
}

/// One party's secrets for one swap. No `Debug`, `Clone` or `Serialize` (NEVER-2).
pub struct KeyMaterial {
    pub share: Share,
    pub btc_secret: BtcSecret,
    pub share_pub: SharePub,
    pub btc_key: [u8; 33],
    nonce: [u8; 32],
    dleq: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RevealError {
    #[error("the reveal does not match the commitment")]
    Commitment,
    #[error("the cross-group DLEq proof does not verify for (S^P, S^S)")]
    Dleq,
    #[error("the proof of knowledge for S^P does not verify")]
    PokPallas,
    #[error("the proof of knowledge for S^S does not verify")]
    PokSecp,
    #[error("the proof of knowledge for K does not verify")]
    PokBtcKey,
    #[error("a revealed point is the identity or not on its curve")]
    InvalidPoint,
    #[error("both parties revealed the same share")]
    SameShare,
    #[error("could not generate key material: {0}")]
    Generate(&'static str),
}

/// §3.2-R2: a fresh share with its DLEq proof, a Bitcoin key and a commitment nonce.
pub fn generate<R: RngCore + CryptoRng>(
    rng: &mut R,
    swap_id: &[u8; 32],
    role: Role,
) -> Result<KeyMaterial, RevealError> {
    let (secret, pallas, secp, proof) =
        dleq::prove(rng, swap_id, role.byte()).map_err(|_| RevealError::Generate("DLEq prover"))?;
    let btc_sk = random_secret_key(rng)?;
    let mut nonce = [0; 32];
    rng.fill_bytes(&mut nonce);
    Ok(KeyMaterial {
        share: Share::from_bytes(*secret.pallas()),
        btc_key: PublicKey::from_secret_key(&Secp256k1::signing_only(), &btc_sk).serialize(),
        btc_secret: BtcSecret::from_bytes(btc_sk.secret_bytes()),
        share_pub: SharePub { pallas, secp },
        nonce,
        dleq: proof,
    })
}

/// §3.1-R1: `C_x = H256(HK_commit_v1, swap_id ‖ role_byte ‖ S^P ‖ S^S ‖ K ‖ nonce)`.
pub fn commit(
    swap_id: &[u8; 32],
    role: Role,
    share_pub: &SharePub,
    btc_key: &[u8; 33],
    nonce: &[u8; 32],
) -> Commitment {
    let mut m = Vec::with_capacity(32 + 1 + 32 + 33 + 33 + 32);
    m.extend_from_slice(swap_id);
    m.push(role.byte());
    m.extend_from_slice(&share_pub.pallas);
    m.extend_from_slice(&share_pub.secp);
    m.extend_from_slice(btc_key);
    m.extend_from_slice(nonce);
    Commitment(h256(HK_COMMIT_V1, &m))
}

/// `m_pok = H256(HK_pok_v1, swap_id ‖ role_byte ‖ C_A ‖ C_B)` (§3.2-R3.3).
fn pok_message(swap_id: &[u8; 32], role: Role, c_a: &Commitment, c_b: &Commitment) -> [u8; 32] {
    let mut m = Vec::with_capacity(32 + 1 + 64);
    m.extend_from_slice(swap_id);
    m.push(role.byte());
    m.extend_from_slice(&c_a.0);
    m.extend_from_slice(&c_b.0);
    h256(HK_POK_V1, &m)
}

impl KeyMaterial {
    pub fn commitment(&self, swap_id: &[u8; 32], role: Role) -> Commitment {
        commit(swap_id, role, &self.share_pub, &self.btc_key, &self.nonce)
    }

    /// §3.2-R1. Call only once both commitments are known (§3.1-R2).
    pub fn reveal(
        &self,
        swap_id: &[u8; 32],
        role: Role,
        c_a: &Commitment,
        c_b: &Commitment,
        payout_addr: String,
    ) -> Result<Reveal, RevealError> {
        let m = pok_message(swap_id, role, c_a, c_b);
        let secp = Secp256k1::signing_only();

        let pallas_key = reddsa::SigningKey::<SpendAuth>::from_bytes(self.share.expose_secret())
            .map_err(|_| RevealError::Generate("share is not a Pallas scalar"))?;
        // ponytail: `UnwrapErr` panics if the OS RNG fails, which reddsa's infallible `Rng`
        // bound forces. Fine on any OS where getrandom works at all.
        let rng = getrandom::rand_core::UnwrapErr(getrandom::SysRng);
        let pok_p: [u8; 64] = pallas_key.sign(rng, &m).into();

        // The same integer as a secp256k1 scalar: Share is little-endian, secp big-endian.
        let mut be = *self.share.expose_secret();
        be.reverse();
        let share_keypair = Keypair::from_secret_key(
            &secp,
            &SecretKey::from_slice(&be).map_err(|_| RevealError::Generate("share as secp"))?,
        );
        be.fill(0);
        let mut aux = [0; 32];
        getrandom::fill(&mut aux).map_err(|_| RevealError::Generate("OS RNG"))?;
        let pok_s = secp
            .sign_schnorr_with_aux_rand(&Message::from_digest(m), &share_keypair, &aux)
            .serialize();

        let btc_sk = SecretKey::from_slice(self.btc_secret.expose_secret())
            .map_err(|_| RevealError::Generate("BTC key"))?;
        let pok_k = secp
            .sign_ecdsa(&Message::from_digest(m), &btc_sk)
            .serialize_compact();

        Ok(Reveal {
            share_pub: self.share_pub,
            btc_key: self.btc_key,
            nonce: self.nonce,
            dleq: self.dleq.clone(),
            pok_p,
            pok_s,
            pok_k,
            payout_addr,
        })
    }
}

/// §3.2-R3: every check a party runs on the other's reveal. `role` is the revealer's;
/// `own` is the verifier's own public share (for R3.7). Any error aborts the swap, with no
/// retry under the same `swap_id`.
pub fn verify_reveal(
    swap_id: &[u8; 32],
    role: Role,
    commitment: &Commitment,
    reveal: &Reveal,
    c_a: &Commitment,
    c_b: &Commitment,
    own: &SharePub,
) -> Result<(), RevealError> {
    // R3.1
    let expected = commit(
        swap_id,
        role,
        &reveal.share_pub,
        &reveal.btc_key,
        &reveal.nonce,
    );
    if expected != *commitment {
        return Err(RevealError::Commitment);
    }

    // R3.6 first, so the signature checks below never run on degenerate keys.
    let pallas_point =
        Option::<pallas::Point>::from(pallas::Point::from_bytes(&reveal.share_pub.pallas))
            .filter(|p| !bool::from(p.is_identity()))
            .ok_or(RevealError::InvalidPoint)?;
    // SEC1 compressed encodings can't express the point at infinity.
    let share_secp =
        PublicKey::from_slice(&reveal.share_pub.secp).map_err(|_| RevealError::InvalidPoint)?;
    let btc_key = PublicKey::from_slice(&reveal.btc_key).map_err(|_| RevealError::InvalidPoint)?;

    // R3.2
    dleq::verify(
        swap_id,
        role.byte(),
        &reveal.share_pub.pallas,
        &reveal.share_pub.secp,
        &reveal.dleq,
    )
    .map_err(|_| RevealError::Dleq)?;

    let m = pok_message(swap_id, role, c_a, c_b);
    let secp = Secp256k1::verification_only();

    // R3.3
    let vk = reddsa::VerificationKey::<SpendAuth>::try_from(pallas_point.to_bytes())
        .map_err(|_| RevealError::PokPallas)?;
    vk.verify(&m, &reddsa::Signature::from(reveal.pok_p))
        .map_err(|_| RevealError::PokPallas)?;

    // R3.4
    let (x_only, _) = share_secp.x_only_public_key();
    let sig_s = schnorr::Signature::from_slice(&reveal.pok_s).map_err(|_| RevealError::PokSecp)?;
    secp.verify_schnorr(&sig_s, &Message::from_digest(m), &x_only)
        .map_err(|_| RevealError::PokSecp)?;

    // R3.5
    let sig_k =
        ecdsa::Signature::from_compact(&reveal.pok_k).map_err(|_| RevealError::PokBtcKey)?;
    secp.verify_ecdsa(&Message::from_digest(m), &sig_k, &btc_key)
        .map_err(|_| RevealError::PokBtcKey)?;

    // R3.7
    if reveal.share_pub.pallas == own.pallas || reveal.share_pub.secp == own.secp {
        return Err(RevealError::SameShare);
    }
    Ok(())
}

/// §4.5-R2: before sweeping, the recovered share must open the other party's `S^P`.
pub fn check_share_matches(share: &Share, public: &SharePub) -> bool {
    reddsa::SigningKey::<SpendAuth>::from_bytes(share.expose_secret())
        .map(|sk| <[u8; 32]>::from(reddsa::VerificationKey::from(&sk)) == public.pallas)
        .unwrap_or(false)
}

fn random_secret_key<R: RngCore>(rng: &mut R) -> Result<SecretKey, RevealError> {
    // A uniform 32-byte string is a valid key except with probability about 2^-128.
    for _ in 0..8 {
        let mut bytes = [0; 32];
        rng.fill_bytes(&mut bytes);
        let key = SecretKey::from_slice(&bytes);
        bytes.fill(0);
        if let Ok(key) = key {
            return Ok(key);
        }
    }
    Err(RevealError::Generate("BTC key"))
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use rand_core::OsRng;

    use super::*;

    const SWAP: [u8; 32] = [3; 32];

    struct Party {
        keys: KeyMaterial,
        reveal: Reveal,
    }

    /// Two honest parties after commit and reveal. DLEq proving is slow, so tests share them.
    static PARTIES: LazyLock<(Party, Party, Commitment, Commitment)> = LazyLock::new(|| {
        let alice = generate(&mut OsRng, &SWAP, Role::Alice).unwrap();
        let bob = generate(&mut OsRng, &SWAP, Role::Bob).unwrap();
        let (c_a, c_b) = (
            alice.commitment(&SWAP, Role::Alice),
            bob.commitment(&SWAP, Role::Bob),
        );
        let r_a = alice
            .reveal(&SWAP, Role::Alice, &c_a, &c_b, "alice-addr".into())
            .unwrap();
        let r_b = bob
            .reveal(&SWAP, Role::Bob, &c_a, &c_b, "bob-addr".into())
            .unwrap();
        (
            Party {
                keys: alice,
                reveal: r_a,
            },
            Party {
                keys: bob,
                reveal: r_b,
            },
            c_a,
            c_b,
        )
    });

    /// Alice checking a (possibly tampered) reveal from Bob.
    fn alice_checks(reveal: &Reveal, bob_commitment: &Commitment) -> Result<(), RevealError> {
        let (alice, _, c_a, c_b) = &*PARTIES;
        verify_reveal(
            &SWAP,
            Role::Bob,
            bob_commitment,
            reveal,
            c_a,
            c_b,
            &alice.keys.share_pub,
        )
    }

    /// The commitment an attacker would have sent for `reveal`, so later checks get exercised.
    fn commitment_for(reveal: &Reveal) -> Commitment {
        commit(
            &SWAP,
            Role::Bob,
            &reveal.share_pub,
            &reveal.btc_key,
            &reveal.nonce,
        )
    }

    #[test]
    fn spec_3_2_r3_honest_reveals_verify_both_ways() {
        let (alice, bob, c_a, c_b) = &*PARTIES;
        assert_eq!(alice_checks(&bob.reveal, c_b), Ok(()));
        assert_eq!(
            verify_reveal(
                &SWAP,
                Role::Alice,
                c_a,
                &alice.reveal,
                c_a,
                c_b,
                &bob.keys.share_pub
            ),
            Ok(())
        );
    }

    #[test]
    fn spec_3_2_r3_1_rejects_reveal_not_matching_commitment() {
        let (_, bob, _, c_b) = &*PARTIES;
        let mut r = bob.reveal.clone();
        r.nonce[0] ^= 1;
        assert_eq!(alice_checks(&r, c_b), Err(RevealError::Commitment));
    }

    #[test]
    fn spec_3_2_r3_2_rejects_a_dleq_for_other_points() {
        let (alice, bob, _, c_b) = &*PARTIES;
        let mut r = bob.reveal.clone();
        r.dleq = alice.reveal.dleq.clone();
        assert_eq!(alice_checks(&r, c_b), Err(RevealError::Dleq));
    }

    #[test]
    fn spec_3_2_r3_3_to_5_reject_each_bad_proof_of_knowledge() {
        let (_, bob, _, c_b) = &*PARTIES;
        let mut r = bob.reveal.clone();
        r.pok_p[5] ^= 1;
        assert_eq!(alice_checks(&r, c_b), Err(RevealError::PokPallas));
        let mut r = bob.reveal.clone();
        r.pok_s[5] ^= 1;
        assert_eq!(alice_checks(&r, c_b), Err(RevealError::PokSecp));
        let mut r = bob.reveal.clone();
        r.pok_k[5] ^= 1;
        assert_eq!(alice_checks(&r, c_b), Err(RevealError::PokBtcKey));
    }

    #[test]
    fn spec_3_2_r3_3_proofs_are_bound_to_both_commitments() {
        let (alice, bob, c_a, c_b) = &*PARTIES;
        let other = Commitment([9; 32]);
        assert_eq!(
            verify_reveal(
                &SWAP,
                Role::Bob,
                c_b,
                &bob.reveal,
                &other,
                c_b,
                &alice.keys.share_pub
            ),
            Err(RevealError::PokPallas)
        );
        assert_eq!(
            verify_reveal(
                &SWAP,
                Role::Bob,
                c_b,
                &bob.reveal,
                c_a,
                &other,
                &alice.keys.share_pub
            ),
            Err(RevealError::PokPallas)
        );
    }

    #[test]
    fn spec_3_2_r3_6_rejects_identity_points() {
        let (_, bob, _, _) = &*PARTIES;
        let mut r = bob.reveal.clone();
        r.share_pub.pallas = [0; 32];
        assert_eq!(
            alice_checks(&r, &commitment_for(&r)),
            Err(RevealError::InvalidPoint)
        );
        let mut r = bob.reveal.clone();
        r.btc_key = [0; 33];
        assert_eq!(
            alice_checks(&r, &commitment_for(&r)),
            Err(RevealError::InvalidPoint)
        );
    }

    #[test]
    fn spec_3_2_r3_7_rejects_a_copy_of_the_verifiers_own_share() {
        let (alice, _, c_a, c_b) = &*PARTIES;
        // Alice's own reveal, played back to her as if it were Bob's share.
        assert_eq!(
            verify_reveal(
                &SWAP,
                Role::Alice,
                c_a,
                &alice.reveal,
                c_a,
                c_b,
                &alice.keys.share_pub
            ),
            Err(RevealError::SameShare)
        );
    }

    /// T13 / ADV-12: a reveal can't be replayed into another swap or under the other role.
    #[test]
    fn spec_3_2_r4_reveal_is_bound_to_swap_id_and_role() {
        let (alice, bob, c_a, c_b) = &*PARTIES;
        let own = &alice.keys.share_pub;
        assert_eq!(
            verify_reveal(&[4; 32], Role::Bob, c_b, &bob.reveal, c_a, c_b, own),
            Err(RevealError::Commitment)
        );
        assert_eq!(
            verify_reveal(&SWAP, Role::Alice, c_b, &bob.reveal, c_a, c_b, own),
            Err(RevealError::Commitment)
        );
    }

    /// ADV-01: Bob tries `S_b = X − S_a`, so that `S_a + S_b = X` with `X` his alone. He
    /// doesn't know the discrete log of `S_b`, so he can't prove it.
    #[test]
    fn adv_01_rogue_key_is_rejected() {
        let (alice, bob, _, c_b) = &*PARTIES;
        let x = generate(&mut OsRng, &SWAP, Role::Bob).unwrap();
        let s_a_p = pallas::Point::from_bytes(&alice.keys.share_pub.pallas).unwrap();
        let x_p = pallas::Point::from_bytes(&x.share_pub.pallas).unwrap();
        let secp = Secp256k1::new();
        let s_a_s = PublicKey::from_slice(&alice.keys.share_pub.secp)
            .unwrap()
            .negate(&secp);
        let x_s = PublicKey::from_slice(&x.share_pub.secp).unwrap();
        let mut rogue = bob.reveal.clone();
        rogue.share_pub = SharePub {
            pallas: (x_p - s_a_p).to_bytes(),
            secp: x_s.combine(&s_a_s).unwrap().serialize(),
        };
        rogue.dleq = x.dleq.clone();

        // Committed before seeing S_a (the protocol's order): the commitment catches it.
        assert_eq!(alice_checks(&rogue, c_b), Err(RevealError::Commitment));
        // Even with a commitment to the rogue key, no proof verifies for it.
        assert!(matches!(
            alice_checks(&rogue, &commitment_for(&rogue)),
            Err(RevealError::Dleq | RevealError::PokPallas)
        ));
    }

    /// ADV-02: `S^P` and `S^S` with different discrete logs, so revealing the secp256k1
    /// share on Bitcoin would not unlock the Zcash side.
    #[test]
    fn adv_02_mismatched_twins_are_rejected() {
        let (alice, bob, _, _) = &*PARTIES;
        let mut r = bob.reveal.clone();
        r.share_pub.secp = alice.keys.share_pub.secp;
        assert_eq!(
            alice_checks(&r, &commitment_for(&r)),
            Err(RevealError::Dleq)
        );
    }

    #[test]
    fn spec_4_5_r2_recovered_share_must_open_the_public_share() {
        let (alice, bob, _, _) = &*PARTIES;
        assert!(check_share_matches(
            &alice.keys.share,
            &alice.keys.share_pub
        ));
        assert!(!check_share_matches(&alice.keys.share, &bob.keys.share_pub));
    }
}

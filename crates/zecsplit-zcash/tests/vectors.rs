//! PROTOCOL_SPEC §9 test vectors (T-014): fixed inputs give fixed C_A, C_B, T, σ, ak, nk, rivk
//! and the regtest lock UA.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use reddsa::orchard::SpendAuth;
use zecsplit_crypto::{
    Network, Reveal, Role, SharePub, TranscriptInput, commit, derive_nk_rivk, shared_ak, transcript,
};
use zecsplit_zcash::{lock_keys, regtest};

const PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../zecsplit-crypto/vectors/v1.json"
);
const SWAP_ID: [u8; 32] = [0x5a; 32];

/// A fixed share in [1, 2^254) and its two public points.
fn share_pub(seed: u8) -> SharePub {
    let mut s = [seed; 32];
    s[31] &= 0x3f;
    let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(&s).unwrap();
    let pallas = reddsa::VerificationKey::from(&sk).into();
    s.reverse();
    let secp = secp256k1::PublicKey::from_secret_key(
        &secp256k1::Secp256k1::signing_only(),
        &secp256k1::SecretKey::from_slice(&s).unwrap(),
    )
    .serialize();
    SharePub { pallas, secp }
}

fn btc_key(seed: u8) -> [u8; 33] {
    secp256k1::PublicKey::from_secret_key(
        &secp256k1::Secp256k1::signing_only(),
        &secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap(),
    )
    .serialize()
}

/// The reveal fields `T` hashes.
fn reveal(share_pub: SharePub, btc_key: [u8; 33], nonce: [u8; 32], addr: &str) -> Reveal {
    Reveal {
        share_pub,
        btc_key,
        nonce,
        dleq: vec![0xd1; 48],
        pok_p: [0xe1; 64],
        pok_s: [0xe2; 64],
        pok_k: [0xe3; 64],
        payout_addr: addr.into(),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn compute() -> BTreeMap<&'static str, String> {
    let (pub_a, pub_b) = (share_pub(0x11), share_pub(0x22));
    let (k_a, k_b) = (btc_key(0x33), btc_key(0x44));
    let (n_a, n_b) = ([0x55; 32], [0x66; 32]);
    let c_a = commit(&SWAP_ID, Role::Alice, &pub_a, &k_a, &n_a);
    let c_b = commit(&SWAP_ID, Role::Bob, &pub_b, &k_b, &n_b);
    let ra = reveal(pub_a, k_a, n_a, "bcrt1qalice");
    let rb = reveal(pub_b, k_b, n_b, "bcrt1qbob");
    let t = transcript(&TranscriptInput {
        protocol_version: 1,
        swap_id: SWAP_ID,
        network: Network::Regtest,
        quote_hash: [0x9b; 32],
        c_a,
        c_b,
        reveal_a: &ra,
        reveal_b: &rb,
    })
    .unwrap();
    let key = shared_ak(&pub_a, &pub_b).unwrap();
    let (nk, rivk) = derive_nk_rivk(&t);
    let ua = lock_keys(&regtest(), &key.ak, &nk, &rivk).unwrap().ua;

    BTreeMap::from([
        ("c_a", hex(&c_a.0)),
        ("c_b", hex(&c_b.0)),
        ("t", hex(&t)),
        ("sigma", format!("{:?}", key.sigma)),
        ("ak", hex(&key.ak)),
        ("nk", hex(&nk)),
        ("rivk", hex(&rivk)),
        ("lock_ua_regtest", ua),
    ])
}

#[test]
fn spec_9_vectors_match() {
    let actual = serde_json::to_string_pretty(&compute()).unwrap() + "\n";
    if std::env::var_os("ZECSPLIT_WRITE_VECTORS").is_some() {
        std::fs::write(PATH, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(PATH).expect("vectors/v1.json missing");
    assert_eq!(
        actual, expected,
        "§9 vector drift: a protocol-visible value changed"
    );
}

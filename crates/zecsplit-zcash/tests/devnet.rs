//! G1 items 2–4 on the regtest devnet (T-011, T-012, T-013).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

use reddsa::orchard::SpendAuth;
use zecsplit_crypto::{Ask, Share, SharePub, Sigma, derive_nk_rivk, shared_ak, shared_ask};
use zecsplit_zcash::{LockKeys, SweepError, lock_keys, lock_status, regtest, sweep};

const LIGHT_SERVER: &str = "http://127.0.0.1:9067";
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../devnet/scripts");

fn devnet() -> bool {
    let on = std::env::var_os("ZECSPLIT_DEVNET").is_some();
    if !on {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
    }
    on
}

fn share(seed: u8) -> (Share, SharePub) {
    let mut s = [seed; 32];
    s[31] &= 0x3f;
    let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(&s).unwrap();
    let public = SharePub {
        pallas: reddsa::VerificationKey::from(&sk).into(),
        secp: [0; 33],
    };
    (Share::from_bytes(s), public)
}

fn zebra_rpc(method: &str, params: &str) -> serde_json::Value {
    let body = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{params}}}"#);
    let out = Command::new("curl")
        .args([
            "-s",
            "-H",
            "content-type: application/json",
            "--data",
            &body,
        ])
        .arg("http://127.0.0.1:18232")
        .output()
        .unwrap();
    serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["result"].clone()
}

fn zebra_height() -> u32 {
    u32::try_from(zebra_rpc("getblockcount", "[]").as_u64().unwrap()).unwrap()
}

fn script(args: &[&str]) -> String {
    let out = Command::new(format!("{SCRIPTS}/{}", args[0]))
        .args(&args[1..])
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// `zec-wallet send` pays from the funded devnet wallet and mines one block.
fn pay(ua: &str, zats: u64) {
    script(&["zec-wallet", "send", ua, &zats.to_string()]);
}

fn mine_zec(blocks: u32) {
    script(&["mine-zec", &blocks.to_string()]);
}

/// A fresh two-share lock funded with `zats` and confirmed twice.
fn funded_lock(zats: u64) -> (Share, Share, Sigma, LockKeys, u32) {
    let (s_a, a) = share(3);
    let (s_b, b) = share(4);
    let mut transcript = [0; 32];
    getrandom::fill(&mut transcript).unwrap();
    let key = shared_ak(&a, &b).unwrap();
    let (nk, rivk) = derive_nk_rivk(&transcript);
    let keys = lock_keys(&regtest(), &key.ak, &nk, &rivk).unwrap();
    mine_zec(3);
    let birthday = zebra_height() + 1;
    pay(&keys.ua, zats);
    mine_zec(2);
    (s_a, s_b, key.sigma, keys, birthday)
}

async fn sweep_with(
    db: &Path,
    keys: &LockKeys,
    birthday: u32,
    ask: &Ask,
) -> Result<String, SweepError> {
    let to = script(&["zec-wallet", "address"]);
    sweep(&regtest(), db, &keys.ufvk, birthday, LIGHT_SERVER, ask, &to)
        .await
        .map(|txid| txid.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_4_3_r2_sums_notes_seen_through_the_lock_ufvk() {
    if !devnet() {
        return;
    }
    let (_, _, _, keys, birthday) = funded_lock(5_000_000);
    pay(&keys.ua, 3_000_000);

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lock.sqlite");
    let one = lock_status(&regtest(), &db, &keys.ufvk, birthday, LIGHT_SERVER, 1)
        .await
        .unwrap();
    assert_eq!(one.total, 8_000_000, "both notes summed");
    assert_eq!(one.confirmed, 8_000_000);

    let two = lock_status(&regtest(), &db, &keys.ufvk, birthday, LIGHT_SERVER, 2)
        .await
        .unwrap();
    assert_eq!(two.total, 8_000_000);
    assert_eq!(two.confirmed, 5_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_4_5_r3_sweeps_the_lock_with_both_shares() {
    if !devnet() {
        return;
    }
    let (s_a, s_b, sigma, keys, birthday) = funded_lock(5_000_000);
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lock.sqlite");

    let ask = shared_ask(&s_a, &s_b, sigma).unwrap();
    let txid = sweep_with(&db, &keys, birthday, &ask).await.unwrap();
    mine_zec(1);

    let mined = zebra_rpc("getrawtransaction", &format!(r#"["{txid}", 1]"#));
    assert!(
        mined["height"].as_u64().is_some(),
        "sweep {txid} not mined: {mined}"
    );
    let after = lock_status(&regtest(), &db, &keys.ufvk, birthday, LIGHT_SERVER, 1)
        .await
        .unwrap();
    assert_eq!(after.total, 0, "lock emptied");
    println!("sweep {txid} mined at height {}", mined["height"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_3_3_r1_single_share_rejected() {
    if !devnet() {
        return;
    }
    let (s_a, s_b, _, keys, birthday) = funded_lock(2_000_000);
    let dir = tempfile::tempdir().unwrap();
    for (name, half) in [("s_a", &s_a), ("s_b", &s_b)] {
        let db = dir.path().join(format!("{name}.sqlite"));
        let ask = Ask::from_bytes(*half.expose_secret());
        let err = sweep_with(&db, &keys, birthday, &ask).await.unwrap_err();
        assert!(matches!(err, SweepError::WrongKey), "{name} alone: {err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_3_3_r1_wrong_sigma_rejected() {
    if !devnet() {
        return;
    }
    let (s_a, s_b, sigma, keys, birthday) = funded_lock(2_000_000);
    let wrong = match sigma {
        Sigma::Plus => Sigma::Minus,
        Sigma::Minus => Sigma::Plus,
    };
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lock.sqlite");
    let ask = shared_ask(&s_a, &s_b, wrong).unwrap();
    let err = sweep_with(&db, &keys, birthday, &ask).await.unwrap_err();
    assert!(matches!(err, SweepError::WrongKey), "wrong σ: {err}");
}

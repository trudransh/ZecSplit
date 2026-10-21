//! G1 items 2 on the regtest devnet (T-011): ZEC sent to a two-share lock address is seen
//! through the imported UFVK, with the right total and confirmations.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use reddsa::orchard::SpendAuth;
use zecsplit_crypto::{SharePub, derive_nk_rivk, shared_ak};
use zecsplit_zcash::{lock_keys, lock_status, regtest};

const LIGHT_SERVER: &str = "http://127.0.0.1:9067";
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../devnet/scripts");

fn devnet() -> bool {
    std::env::var_os("ZECSPLIT_DEVNET").is_some()
}

fn share_pub(seed: u8) -> SharePub {
    let mut s = [seed; 32];
    s[31] &= 0x3f;
    let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(&s).unwrap();
    SharePub {
        pallas: reddsa::VerificationKey::from(&sk).into(),
        secp: [0; 33],
    }
}

fn zebra_height() -> u32 {
    let out = Command::new("curl")
        .args([
            "-s",
            "-H",
            "content-type: application/json",
            "--data",
            r#"{"jsonrpc":"2.0","id":1,"method":"getblockcount","params":[]}"#,
            "http://127.0.0.1:18232",
        ])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    u32::try_from(v["result"].as_u64().unwrap()).unwrap()
}

/// `zec-wallet send` pays from the funded devnet wallet and mines one block.
fn pay(ua: &str, zats: u64) {
    let status = Command::new(format!("{SCRIPTS}/zec-wallet"))
        .args(["send", ua, &zats.to_string()])
        .status()
        .unwrap();
    assert!(status.success(), "zec-wallet send failed");
}

fn mine_zec(blocks: u32) {
    let status = Command::new(format!("{SCRIPTS}/mine-zec"))
        .arg(blocks.to_string())
        .status()
        .unwrap();
    assert!(status.success(), "mine-zec failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn spec_4_3_r2_sums_notes_seen_through_the_lock_ufvk() {
    if !devnet() {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
        return;
    }
    let mut transcript = [0; 32];
    getrandom::fill(&mut transcript).unwrap();
    let key = shared_ak(&share_pub(1), &share_pub(2)).unwrap();
    let (nk, rivk) = derive_nk_rivk(&transcript);
    let keys = lock_keys(&regtest(), &key.ak, &nk, &rivk).unwrap();

    mine_zec(3);
    let birthday = zebra_height() + 1;
    pay(&keys.ua, 30_000_000);
    mine_zec(2);
    pay(&keys.ua, 20_000_000);

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lock.sqlite");
    let one = lock_status(&regtest(), &db, &keys.ufvk, birthday, LIGHT_SERVER, 1)
        .await
        .unwrap();
    assert_eq!(one.total, 50_000_000, "both notes summed");
    assert_eq!(one.confirmed, 50_000_000);

    let two = lock_status(&regtest(), &db, &keys.ufvk, birthday, LIGHT_SERVER, 2)
        .await
        .unwrap();
    assert_eq!(two.total, 50_000_000);
    assert_eq!(two.confirmed, 30_000_000);
    println!(
        "lock UA {} holds {} zats at tip {}",
        keys.ua, two.total, two.tip
    );
}

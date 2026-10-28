//! The regtest devnet: a full happy-path swap in-process, real BTC lock and redeem, real ZEC lock
//! and sweep (T-031); the cancel, refund and punish paths on Bitcoin with their timelocks (T-024, G3).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::{Address, Psbt, Transaction};
use zecsplit_btc::sources::{BitcoindRpc, Broadcast, SourceError};
use zecsplit_crypto::Role;
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_protocol::messages::{Network, Quote};
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_protocol::session::{Core, Shared};
use zecsplit_zcash::{lock_keys, lock_status, regtest, sweep};

const LIGHT_SERVER: &str = "http://127.0.0.1:9067";
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../devnet/scripts");
const COMPOSE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../devnet/docker-compose.yml"
);
const ZEC_AMOUNT: u64 = 5_000_000;
const NODE: &str = "http://127.0.0.1:18443";
const T1: u16 = 20;
const T2: u16 = 20;

fn script(args: &[&str]) -> String {
    let out = Command::new(format!("{SCRIPTS}/{}", args[0]))
        .args(&args[1..])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// `bitcoin-cli` on the devnet wallet, run through docker directly (as `btc-cli` does) so it also
/// works where the bash scripts can't run, such as Windows.
fn btc(args: &[&str]) -> String {
    let out = Command::new("docker")
        .args([
            "compose",
            "-f",
            COMPOSE,
            "exec",
            "-T",
            "bitcoind",
            "bitcoin-cli",
        ])
        .args([
            "-regtest",
            "-rpcuser=devnet",
            "-rpcpassword=devnet",
            "-rpcwallet=devnet",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn mine_btc(n: u32) {
    btc(&["-generate", &n.to_string()]);
}

fn btc_json(args: &[&str]) -> serde_json::Value {
    serde_json::from_str(&btc(args)).unwrap()
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

fn quote() -> Quote {
    let mut swap_id = [0; 32];
    getrandom::fill(&mut swap_id).unwrap();
    Quote {
        protocol_version: 1,
        swap_id,
        network: Network::Regtest,
        zec_amount: ZEC_AMOUNT,
        btc_amount: 150_000,
        btc_fee_rate: 2,
        t1: T1,
        t2: T2,
        n_conf_btc: 1,
        n_conf_zec: 1,
        redeem_margin: 5,
        expires_at: u64::MAX,
    }
}

struct Locked {
    alice: AliceSession,
    bob: BobSession,
    shared: Shared,
    alice_payout: String,
    bob_refund: String,
    lock_txid: String,
}

/// Setup through §3.5 and a confirmed `tx_lock`: the common start of every path.
fn locked_swap() -> Locked {
    let alice_payout = btc(&["getnewaddress", "alice", "bech32"]);
    let bob_payout = btc(&["getnewaddress", "bob", "bech32"]);
    let bob_refund = btc(&["getnewaddress", "refund", "bech32"]);

    let q = quote();
    let mut a = Core::new(&mut OsRng, q.clone(), Role::Alice, alice_payout.clone()).unwrap();
    let mut b = Core::new(&mut OsRng, q, Role::Bob, bob_payout).unwrap();
    a.on_commit(&b.commit_msg()).unwrap();
    b.on_commit(&a.commit_msg()).unwrap();
    let (reveal_a, reveal_b) = (a.reveal_msg().unwrap(), b.reveal_msg().unwrap());
    let shared = a.on_reveal(&reveal_b).unwrap();
    assert_eq!(b.on_reveal(&reveal_a).unwrap(), shared);
    let mut alice = AliceSession::new(a);
    let mut bob = BobSession::new(b);

    let (lock_spk, v_lock) = bob.lock_output(&bob_refund).unwrap();
    let lock_addr = Address::from_script(&lock_spk, bitcoin::Network::Regtest).unwrap();
    let outputs = format!(
        r#"[{{"{lock_addr}":{}}}]"#,
        bitcoin::Amount::from_sat(v_lock.to_sat()).to_btc()
    );
    let funded = btc_json(&[
        "walletcreatefundedpsbt",
        "[]",
        &outputs,
        "0",
        r#"{"change_type":"bech32"}"#,
    ]);
    let psbt_b64 = funded["psbt"].as_str().unwrap().to_string();
    let psbt: Psbt = psbt_b64.parse().unwrap();

    let tpl = bob.lock_template(psbt, &bob_refund).unwrap();
    let sigs_a = alice.on_lock_template(&tpl).unwrap();
    let sigs_b = bob.on_setup_sigs_alice(&sigs_a).unwrap();
    alice.on_setup_sigs_bob(&sigs_b).unwrap();
    assert!(bob.may_broadcast_lock(), "§3.5-R5");

    let signed = btc_json(&["walletprocesspsbt", &psbt_b64]);
    assert_eq!(signed["complete"], true);
    let lock_txid = btc(&["sendrawtransaction", signed["hex"].as_str().unwrap()]);
    mine_btc(1);
    let lock_conf = btc_json(&["gettransaction", &lock_txid])["confirmations"]
        .as_u64()
        .unwrap();
    assert!(lock_conf >= 1, "tx_lock confirmed");
    Locked {
        alice,
        bob,
        shared,
        alice_payout,
        bob_refund,
        lock_txid,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_031_happy_path_swap_on_regtest() {
    if std::env::var_os("ZECSPLIT_DEVNET").is_none() {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
        return;
    }
    let Locked {
        mut alice,
        mut bob,
        shared,
        alice_payout,
        lock_txid,
        ..
    } = locked_swap();

    let keys = lock_keys(&regtest(), &shared.ak, &shared.nk, &shared.rivk).unwrap();
    script(&["mine-zec", "3"]);
    let birthday = u32::try_from(zebra_rpc("getblockcount", "[]").as_u64().unwrap()).unwrap() + 1;
    script(&["zec-wallet", "send", &keys.ua, &ZEC_AMOUNT.to_string()]);
    script(&["mine-zec", "1"]);

    let dir = tempfile::tempdir().unwrap();
    let bob_db = dir.path().join("bob-lock.sqlite");
    let seen = lock_status(&regtest(), &bob_db, &keys.ufvk, birthday, LIGHT_SERVER, 1)
        .await
        .unwrap();
    assert!(
        seen.confirmed >= ZEC_AMOUNT,
        "§4.3-R2: Bob sees the ZEC lock"
    );

    let esig = bob.encsig().unwrap();
    alice.on_encsig(&esig).unwrap();
    let redeem = alice.signed_redeem().unwrap();
    let redeem_txid = btc(&["sendrawtransaction", &serialize_hex(&redeem)]);
    mine_btc(1);
    let mined = btc_json(&["getrawtransaction", &redeem_txid, "true"]);
    assert!(
        mined["confirmations"].as_u64().unwrap() >= 1,
        "tx_redeem confirmed"
    );
    let paid = btc_json(&["getreceivedbyaddress", &alice_payout, "1"])
        .as_f64()
        .unwrap();
    assert!((paid - 0.0015).abs() < 1e-9, "Alice got btc_amount: {paid}");

    let ask = bob.ask_from_redeem(&redeem).unwrap();
    let to = script(&["zec-wallet", "address"]);
    let sweep_txid = sweep(
        &regtest(),
        &bob_db,
        &keys.ufvk,
        birthday,
        LIGHT_SERVER,
        &ask,
        &to,
    )
    .await
    .unwrap()
    .to_string();
    script(&["mine-zec", "1"]);
    let swept = zebra_rpc("getrawtransaction", &format!(r#"["{sweep_txid}", 1]"#));
    assert!(
        swept["height"].as_u64().is_some(),
        "ZEC sweep mined: {swept}"
    );
    println!("swap done: lock {lock_txid}, redeem {redeem_txid}, sweep {sweep_txid}");
}

fn devnet() -> bool {
    let on = std::env::var_os("ZECSPLIT_DEVNET").is_some();
    if !on {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
    }
    on
}

fn node() -> BitcoindRpc {
    BitcoindRpc::new(NODE, "devnet", "devnet").unwrap()
}

fn tip() -> u32 {
    btc(&["getblockcount"]).parse().unwrap()
}

fn mine_to(height: u32) {
    let n = height - tip();
    if n > 0 {
        mine_btc(n);
    }
}

fn height_of(txid: &str) -> u32 {
    let tx = btc_json(&["getrawtransaction", txid, "true"]);
    let header = btc_json(&["getblockheader", tx["blockhash"].as_str().unwrap()]);
    u32::try_from(header["height"].as_u64().unwrap()).unwrap()
}

/// §3.4 relative timelock: `tx` can't enter a block below `unlock`. It is refused while the next
/// block would be `unlock - 1`, accepted once the next block is `unlock`, then mined.
async fn timelocked(tx: &Transaction, unlock: u32) -> String {
    mine_to(unlock - 2);
    let early = node().broadcast(tx).await;
    assert!(
        matches!(&early, Err(SourceError::Rpc { message, .. }) if message.contains("non-BIP68-final")),
        "before its timelock: {early:?}"
    );
    mine_btc(1);
    node().broadcast(tx).await.unwrap();
    mine_btc(1);
    let txid = tx.compute_txid().to_string();
    assert_eq!(height_of(&txid), unlock);
    txid
}

fn received(addr: &str) -> u64 {
    let btc = btc_json(&["getreceivedbyaddress", addr, "1"])
        .as_f64()
        .unwrap();
    bitcoin::Amount::from_btc(btc).unwrap().to_sat()
}

/// G3 items 2, 3 and 5: lock → cancel (refused before t1) → refund, and Alice recovers `s_b`.
#[tokio::test(flavor = "multi_thread")]
async fn t_024_cancel_then_refund_on_regtest() {
    if !devnet() {
        return;
    }
    let Locked {
        alice,
        bob,
        bob_refund,
        lock_txid,
        ..
    } = locked_swap();

    let cancel = bob.signed_cancel().unwrap();
    let cancel_txid = timelocked(&cancel, height_of(&lock_txid) + u32::from(T1)).await;

    let refund = bob.signed_refund().unwrap();
    node().broadcast(&refund).await.unwrap();
    mine_btc(1);
    let refund_txid = refund.compute_txid().to_string();
    assert_eq!(height_of(&refund_txid), tip());
    assert_eq!(received(&bob_refund), refund.output[0].value.to_sat());

    alice.ask_from_refund(&refund).unwrap();
    println!("refund path: lock {lock_txid}, cancel {cancel_txid}, refund {refund_txid}");
}

/// G3 items 2 and 4: lock → cancel (refused before t1) → punish (refused before t2).
#[tokio::test(flavor = "multi_thread")]
async fn t_024_cancel_then_punish_on_regtest() {
    if !devnet() {
        return;
    }
    let Locked {
        alice,
        alice_payout,
        lock_txid,
        ..
    } = locked_swap();

    let cancel = alice.signed_cancel().unwrap();
    let cancel_txid = timelocked(&cancel, height_of(&lock_txid) + u32::from(T1)).await;

    let punish = alice.signed_punish().unwrap();
    let punish_txid = timelocked(&punish, height_of(&cancel_txid) + u32::from(T2)).await;
    assert_eq!(received(&alice_payout), punish.output[0].value.to_sat());
    println!("punish path: lock {lock_txid}, cancel {cancel_txid}, punish {punish_txid}");
}

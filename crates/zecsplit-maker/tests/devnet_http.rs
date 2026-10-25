//! G4 on the regtest devnet: S-01 three times in a row, engine and maker as separate processes over
//! HTTP; then PRIV-01/02 on every Zcash transaction and PRIV-03 on every file both wrote.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zecsplit_btc::sources::{BitcoindRpc, BtcSource};
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_crypto::{MakerSecret, Role, maker_public_key, shared_ask};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::bob::BobState;
use zecsplit_protocol::machine::Outcome;
use zecsplit_protocol::messages::Network;
use zecsplit_protocol::recovery::{SecretString, open};
use zecsplit_protocol::session::Core;
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_store::Store;
use zecsplit_zcash::{LockWallets, regtest};
use zecsplitd::maker_client::HttpMaker;
use zecsplitd::runner::{AliceRunner, Ports};

const NODE: &str = "http://127.0.0.1:18443";
const LIGHT_SERVER: &str = "http://127.0.0.1:9067";
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../devnet/scripts");
const IDENTITY: [u8; 32] = [0x11; 32];
const ZEC_AMOUNT: u64 = 5_000_000;

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

fn rpc(path: &str) -> BitcoindRpc {
    BitcoindRpc::new(&format!("{NODE}{path}"), "devnet", "devnet").unwrap()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The maker binary, killed when the test ends.
struct MakerProcess(Child);

impl Drop for MakerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn zebra(method: &str, params: &str) -> serde_json::Value {
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

fn zec_tip() -> u64 {
    zebra("getblockcount", "[]").as_u64().unwrap()
}

/// PRIV-01, PRIV-02: every non-coinbase Zcash tx mined in `from..=to` is a plain 2-action
/// Ironwood transaction with nothing transparent, Sapling or Sprout.
fn assert_shielded_only(from: u64, to: u64) {
    let mut seen = 0;
    for h in from..=to {
        let block = zebra("getblock", &format!(r#"["{h}",1]"#));
        for txid in block["tx"].as_array().unwrap().iter().skip(1) {
            let tx = zebra("getrawtransaction", &format!(r#"[{txid},1]"#));
            for part in [
                "vin",
                "vout",
                "vShieldedSpend",
                "vShieldedOutput",
                "vjoinsplit",
            ] {
                assert_eq!(
                    tx[part].as_array().map_or(0, Vec::len),
                    0,
                    "PRIV-01 {txid} {part}"
                );
            }
            assert_eq!(
                tx["orchard"]["actions"].as_array().map_or(0, Vec::len),
                0,
                "{txid}"
            );
            assert_eq!(
                tx["ironwood"]["actions"].as_array().unwrap().len(),
                2,
                "PRIV-02 {txid}"
            );
            seen += 1;
        }
    }
    assert!(
        seen >= 6,
        "PRIV-01: expected a lock and a sweep per round, saw {seen}"
    );
}

/// PRIV-03: no file under `dir` holds any secret, raw, hex or byte-reversed.
fn assert_no_secret_on_disk(dir: &Path, secrets: &[[u8; 32]]) {
    let mut needles = Vec::new();
    for s in secrets {
        let mut rev = *s;
        rev.reverse();
        for b in [s, &rev] {
            needles.push(b.to_vec());
            needles.push(hex::encode(b).into_bytes());
            needles.push(hex::encode_upper(b).into_bytes());
        }
    }
    let mut stack = vec![dir.to_path_buf()];
    let mut files = 0;
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            for n in &needles {
                assert!(
                    !bytes.windows(n.len()).any(|w| w == n.as_slice()),
                    "PRIV-03: a secret is in {}",
                    path.display()
                );
            }
            files += 1;
        }
    }
    assert!(files >= 4, "PRIV-03 scanned only {files} files");
}

/// `s_a`, `k_a`, `s_b`, `k_b` and `ask` of one swap, from both sealed sessions.
fn swap_secrets(engine: &Store, maker_db: &Path, id: &[u8; 32]) -> Vec<[u8; 32]> {
    let sealed_a = engine.secret(id).unwrap().unwrap();
    let pass_a = || SecretString::from("engine passphrase".to_owned());
    let a = open(&sealed_a, pass_a()).unwrap();
    let sigma = AliceSession::restore(open(&sealed_a, pass_a()).unwrap())
        .unwrap()
        .core
        .shared_key()
        .unwrap()
        .sigma;
    let sealed_b = Store::open(maker_db).unwrap().secret(id).unwrap().unwrap();
    let b = open(&sealed_b, SecretString::from("maker passphrase".to_owned())).unwrap();
    let ask = shared_ask(&a.share, &b.share, sigma).unwrap();
    vec![
        *a.share.expose_secret(),
        *a.btc_key.expose_secret(),
        *b.share.expose_secret(),
        *b.btc_key.expose_secret(),
        *ask.expose_secret(),
    ]
}

fn spawn_maker(data: &Path, listen: &str, zec_payout: &str) -> MakerProcess {
    let log = std::fs::File::create(data.join("maker.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_zecsplit-maker"))
        .env("ZECSPLIT_MAKER_NETWORK", "regtest")
        .env("ZECSPLIT_MAKER_IDENTITY", hex::encode(IDENTITY))
        .env("ZECSPLIT_MAKER_DATA", data)
        .env("ZECSPLIT_MAKER_BITCOIND", NODE)
        .env("ZECSPLIT_MAKER_RPC_USER", "devnet")
        .env("ZECSPLIT_MAKER_RPC_PASS", "devnet")
        .env("ZECSPLIT_MAKER_WALLET", "devnet")
        .env("ZECSPLIT_MAKER_LIGHT_SERVER", LIGHT_SERVER)
        .env("ZECSPLIT_MAKER_ZEC_PAYOUT", zec_payout)
        .env("ZECSPLIT_MAKER_LISTEN", listen)
        .env("ZECSPLIT_MAKER_SATS_PER_ZEC", "500000")
        .env("ZECSPLIT_MAKER_MIN_ZEC", "1000000")
        .env("ZECSPLIT_MAKER_MAX_ZEC", "200000000")
        .env("ZECSPLIT_MAKER_PASSPHRASE", "maker passphrase")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    MakerProcess(child)
}

fn bob_state(maker_db: &Path, id: &[u8; 32]) -> Option<BobState> {
    let json = Store::open(maker_db).ok()?.state(id).ok()??;
    serde_json::from_str(&json).ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn g4_s_01_three_times_over_http_then_priv_01_02_03() {
    if std::env::var_os("ZECSPLIT_DEVNET").is_none() {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let maker_dir = dir.path().join("maker");
    let alice_zec_dir = dir.path().join("alice-zec");
    std::fs::create_dir_all(&maker_dir).unwrap();
    std::fs::create_dir_all(&alice_zec_dir).unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let listen = format!("127.0.0.1:{port}");
    let zec_payout = script(&["zec-wallet", "address"]);
    let _maker = spawn_maker(&maker_dir, &listen, &zec_payout);

    let maker = HttpMaker::new(&format!("http://{listen}")).unwrap();
    let pinned = maker_public_key(&MakerSecret::from_bytes(IDENTITY)).unwrap();
    for _ in 0..60 {
        if maker.identity_key().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(maker.identity_key().await.unwrap(), pinned, "§2-R1");

    let (source_a, source_b) = (rpc(""), rpc(""));
    let sources: [&dyn BtcSource; 2] = [&source_a, &source_b];
    let alice_zec = LockWallets::new(regtest(), alice_zec_dir, LIGHT_SERVER, &zec_payout);
    let ports = Ports {
        maker: &maker,
        sources: &sources,
        broadcast: &source_b,
        zec: &alice_zec,
    };
    let mut store = Store::open(&dir.path().join("engine.sqlite")).unwrap();
    let maker_db = maker_dir.join("maker.sqlite");
    let zec_from = zec_tip() + 1;
    let mut secrets = Vec::new();

    for round in 1..=3 {
        let signed = maker.quote(ZEC_AMOUNT, Network::Regtest).await.unwrap();
        let quote = signed.verify(&pinned, unix_now()).unwrap().clone();
        let payout = script(&["btc-cli", "getnewaddress", "alice", "bech32"]);
        let core = Core::new(&mut OsRng, quote, Role::Alice, payout).unwrap();
        let passphrase = SecretString::from("engine passphrase".to_owned());
        let mut alice =
            AliceRunner::start(&mut store, AliceSession::new(core), passphrase, "maker", 0)
                .unwrap();
        let id = alice.swap_id;
        alice
            .feed(&mut store, &ports, AliceInput::Begin, 0)
            .await
            .unwrap();

        let mut paid = false;
        let mut bob = None;
        for t in 1..90 {
            alice.tick(&mut store, &ports, t).await.unwrap();
            if let Some((ua, zats)) = &alice.payment_request
                && !paid
            {
                script(&["zec-wallet", "send", ua, &zats.to_string()]);
                paid = true;
            }
            bob = bob_state(&maker_db, &id);
            eprintln!(
                "round {round} tick {t}: alice {:?}, bob {bob:?}",
                alice.state
            );
            if alice.state.is_terminal() && bob.is_some_and(|b| b.is_terminal()) {
                break;
            }
            script(&["mine-btc", "1"]);
            script(&["mine-zec", "1"]);
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        assert_eq!(
            alice.state,
            AliceState::Done(Outcome::Swapped),
            "round {round}"
        );
        assert_eq!(bob, Some(BobState::Done(Outcome::Swapped)), "round {round}");
        let maker_store = Store::open(&maker_db).unwrap();
        let txs: Vec<String> = [&store, &maker_store]
            .into_iter()
            .flat_map(|s| s.txs(&id).unwrap())
            .map(|t| format!("{} {}", t.name, t.txid))
            .collect();
        println!("round {round} swap {}: {txs:?}", hex::encode(id));
        secrets.extend(swap_secrets(&store, &maker_db, &id));
    }
    assert_shielded_only(zec_from, zec_tip());
    drop(store);
    assert_no_secret_on_disk(dir.path(), &secrets);
}

//! G4 on the regtest devnet: S-01 three times in a row, engine and maker as separate processes over HTTP.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zecsplit_btc::sources::{BitcoindRpc, BtcSource};
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_crypto::{MakerSecret, Role, maker_public_key};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::bob::BobState;
use zecsplit_protocol::machine::Outcome;
use zecsplit_protocol::messages::Network;
use zecsplit_protocol::recovery::SecretString;
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
const ZEC_AMOUNT: u64 = 30_000_000;

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

fn spawn_maker(data: &Path, listen: &str, zec_payout: &str) -> MakerProcess {
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
        .spawn()
        .unwrap();
    MakerProcess(child)
}

fn bob_state(maker_db: &Path, id: &[u8; 32]) -> Option<BobState> {
    let json = Store::open(maker_db).ok()?.state(id).ok()??;
    serde_json::from_str(&json).ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn g4_s_01_three_times_over_http_between_processes() {
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
        let txs: Vec<String> = store
            .txs(&id)
            .unwrap()
            .into_iter()
            .map(|t| format!("{} {}", t.name, t.txid))
            .collect();
        println!("round {round} swap {}: {txs:?}", hex::encode(id));
    }
}

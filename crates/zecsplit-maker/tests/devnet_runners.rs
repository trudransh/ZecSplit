//! G4 on the regtest devnet: the engine's runner and the maker's runner swap over real bitcoind and Zebra.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::process::Command;

use zecsplit_btc::sources::{BitcoindRpc, BtcSource};
use zecsplit_crypto::Role;
use zecsplit_crypto::rand_core::{OsRng, RngCore};
use zecsplit_maker::ports::BitcoindWallet;
use zecsplit_maker::runner::{self as bob, BobRunner};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::bob::BobState;
use zecsplit_protocol::machine::Outcome;
use zecsplit_protocol::messages::{Network, Quote};
use zecsplit_protocol::session::Core;
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_store::Store;
use zecsplit_zcash::{LockWallets, regtest};
use zecsplitd::runner::{self as alice, AliceRunner};

use common::{Link, pass};

const NODE: &str = "http://127.0.0.1:18443";
const LIGHT_SERVER: &str = "http://127.0.0.1:9067";
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../devnet/scripts");
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

fn quote() -> Quote {
    let mut swap_id = [0; 32];
    OsRng.fill_bytes(&mut swap_id);
    Quote {
        protocol_version: 1,
        swap_id,
        network: Network::Regtest,
        zec_amount: ZEC_AMOUNT,
        btc_amount: 150_000,
        btc_fee_rate: 2,
        t1: 20,
        t2: 20,
        n_conf_btc: 1,
        n_conf_zec: 1,
        redeem_margin: 5,
        expires_at: u64::MAX,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn g4_engine_and_maker_runners_swap_on_regtest() {
    if std::env::var_os("ZECSPLIT_DEVNET").is_none() {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
        return;
    }
    let (source_a, source_b) = (rpc(""), rpc(""));
    let sources: [&dyn BtcSource; 2] = [&source_a, &source_b];
    let wallet = BitcoindWallet::new(rpc("/wallet/devnet"), bitcoin::Network::Regtest);
    let zec_payout = script(&["zec-wallet", "address"]);
    let dir = tempfile::tempdir().unwrap();
    for sub in ["alice-zec", "bob-zec"] {
        std::fs::create_dir(dir.path().join(sub)).unwrap();
    }
    let alice_zec = LockWallets::new(
        regtest(),
        dir.path().join("alice-zec"),
        LIGHT_SERVER,
        &zec_payout,
    );
    let bob_zec = LockWallets::new(
        regtest(),
        dir.path().join("bob-zec"),
        LIGHT_SERVER,
        &zec_payout,
    );

    let q = quote();
    let mut store_b = Store::open(&dir.path().join("maker.sqlite")).unwrap();
    let b = Core::new(
        &mut OsRng,
        q.clone(),
        Role::Bob,
        script(&["btc-cli", "getnewaddress", "bob", "bech32"]),
    )
    .unwrap();
    let bob_runner =
        BobRunner::start(&mut store_b, BobSession::new(b), pass(), "http://maker", 0).unwrap();
    let link = Link {
        bob: tokio::sync::Mutex::new((bob_runner, store_b)),
        ports: bob::Ports {
            sources: &sources,
            broadcast: &source_a,
            wallet: &wallet,
            zec: &bob_zec,
        },
    };

    let mut store_a = Store::open(&dir.path().join("engine.sqlite")).unwrap();
    let ports_a = alice::Ports {
        maker: &link,
        sources: &sources,
        broadcast: &source_b,
        zec: &alice_zec,
    };
    let alice_payout = script(&["btc-cli", "getnewaddress", "alice", "bech32"]);
    let a = Core::new(&mut OsRng, q, Role::Alice, alice_payout.clone()).unwrap();
    let mut alice = AliceRunner::start(
        &mut store_a,
        AliceSession::new(a),
        pass(),
        "http://maker",
        0,
    )
    .unwrap();
    alice
        .feed(&mut store_a, &ports_a, AliceInput::Begin, 0)
        .await
        .unwrap();

    let mut paid = false;
    for t in 1..60 {
        alice.tick(&mut store_a, &ports_a, t).await.unwrap();
        if let Some((ua, zats)) = &alice.payment_request
            && !paid
        {
            script(&["zec-wallet", "send", ua, &zats.to_string()]);
            paid = true;
        }
        {
            let mut guard = link.bob.lock().await;
            let (runner, store) = &mut *guard;
            runner.tick(store, &link.ports, t).await.unwrap();
        }
        let bob_state = link.bob.lock().await.0.state;
        eprintln!("tick {t}: alice {:?}, bob {bob_state:?}", alice.state);
        if alice.state.is_terminal() && bob_state.is_terminal() {
            break;
        }
        script(&["mine-btc", "1"]);
        script(&["mine-zec", "1"]);
    }

    let bob_state = link.bob.lock().await.0.state;
    assert_eq!(alice.state, AliceState::Done(Outcome::Swapped));
    assert_eq!(bob_state, BobState::Done(Outcome::Swapped));
    let txs = |s: &Store, id: &[u8; 32]| {
        s.txs(id)
            .unwrap()
            .into_iter()
            .map(|t| format!("{} {}", t.name, t.txid))
            .collect::<Vec<_>>()
    };
    let swap_id = alice.swap_id;
    println!("engine: {:?}", txs(&store_a, &swap_id));
    println!("maker: {:?}", txs(&link.bob.lock().await.1, &swap_id));
}

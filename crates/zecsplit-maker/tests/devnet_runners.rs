//! G4/G5 on the regtest devnet: the engine's and the maker's runners over real bitcoind and Zebra,
//! through the happy path and every failure path (S-01 to S-07).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::process::Command;
use std::sync::Mutex;

use async_trait::async_trait;
use bitcoin::Transaction;

use zecsplit_btc::sources::{BitcoindRpc, Broadcast, BtcSource, SourceError};
use zecsplit_crypto::rand_core::{OsRng, RngCore};
use zecsplit_crypto::{Ask, Role};
use zecsplit_maker::ports::BitcoindWallet;
use zecsplit_maker::runner::{self as bob, BobRunner, ZecNode};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::bob::{BobInput, BobState};
use zecsplit_protocol::machine::{AbortReason, Outcome, ZecLock};
use zecsplit_protocol::messages::{Network, Quote};
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_protocol::session::{Core, Shared};
use zecsplit_store::Store;
use zecsplit_zcash::{LockWallets, regtest};
use zecsplitd::runner::{self as alice, AliceRunner};

use common::{Link, pass};

const NODE: &str = "http://127.0.0.1:18443";
const LIGHT_SERVER: &str = "http://127.0.0.1:9067";
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../devnet/scripts");
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

/// What goes wrong in a run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fault {
    None,
    MakerNeverLocks,
    TakerNeverPays,
    MakerBlindToZec,
    TakerOverpays,
    MakerRefundBlocked,
    CrashEveryTick,
}

/// Bob's line to bitcoind: with `MakerNeverLocks` it drops everything, with
/// `MakerRefundBlocked` everything but his first transaction, `tx_lock`.
struct Censor<'a> {
    node: &'a BitcoindRpc,
    fault: Fault,
    first: Mutex<Option<bitcoin::Txid>>,
}

#[async_trait]
impl Broadcast for Censor<'_> {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        let txid = tx.compute_txid();
        let first = *self.first.lock().unwrap().get_or_insert(txid);
        match self.fault {
            Fault::MakerNeverLocks => Ok(()),
            Fault::MakerRefundBlocked if txid != first => Ok(()),
            _ => self.node.broadcast(tx).await,
        }
    }
}

/// Bob's Zebra, blind to the lock (S-04, and S-05 so the swap reaches the refund).
struct BobZec<'a> {
    wallets: &'a LockWallets<zcash_protocol::local_consensus::LocalNetwork>,
    blind: bool,
}

#[async_trait]
impl ZecNode for BobZec<'_> {
    async fn lock(&self, shared: &Shared, zatoshis: u64, min_conf: u32) -> Result<ZecLock, String> {
        if self.blind {
            return Ok(ZecLock::Unseen);
        }
        ZecNode::lock(self.wallets, shared, zatoshis, min_conf).await
    }
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String> {
        ZecNode::sweep(self.wallets, shared, ask).await
    }
    async fn confirmations(&self, shared: &Shared, txid: &str) -> Result<Option<u32>, String> {
        ZecNode::confirmations(self.wallets, shared, txid).await
    }
}

/// Seconds per tick, so wall-clock timers (S-02) fire within a test.
const SECS_PER_TICK: i64 = 120;

/// One swap between the real runners under `fault`; returns both end states and every txid.
async fn run(fault: Fault) -> (AliceState, BobState, Vec<String>) {
    let node = rpc("");
    let sources: [&dyn BtcSource; 2] = [&node, &node];
    let censor = Censor {
        node: &node,
        fault,
        first: Mutex::new(None),
    };
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
    let bob_wallets = LockWallets::new(
        regtest(),
        dir.path().join("bob-zec"),
        LIGHT_SERVER,
        &zec_payout,
    );
    let bob_zec = BobZec {
        wallets: &bob_wallets,
        blind: matches!(fault, Fault::MakerBlindToZec | Fault::MakerRefundBlocked),
    };

    let q = quote();
    let mut store_b = Store::open(&dir.path().join("maker.sqlite")).unwrap();
    let bob_refund = script(&["btc-cli", "getnewaddress", "bob", "bech32"]);
    let b = Core::new(&mut OsRng, q.clone(), Role::Bob, bob_refund).unwrap();
    let bob_runner =
        BobRunner::start(&mut store_b, BobSession::new(b), pass(), "http://maker", 0).unwrap();
    let link = Link {
        bob: tokio::sync::Mutex::new((bob_runner, store_b)),
        ports: bob::Ports {
            sources: &sources,
            broadcast: &censor,
            wallet: &wallet,
            zec: &bob_zec,
        },
    };

    let mut store_a = Store::open(&dir.path().join("engine.sqlite")).unwrap();
    let ports_a = alice::Ports {
        maker: &link,
        sources: &sources,
        broadcast: &node,
        zec: &alice_zec,
    };
    let alice_payout = script(&["btc-cli", "getnewaddress", "alice", "bech32"]);
    let a = Core::new(&mut OsRng, q, Role::Alice, alice_payout).unwrap();
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

    let mut paid = fault == Fault::TakerNeverPays;
    for t in 1..120 {
        let now = t * SECS_PER_TICK;
        if fault == Fault::CrashEveryTick {
            if let Some(row) = store_a.active_swaps().unwrap().pop()
                && store_a.secret(&row.id).unwrap().is_some()
                && let Some(r) = AliceRunner::resume(&mut store_a, &row, pass(), now).unwrap()
            {
                alice = r;
                alice
                    .feed(&mut store_a, &ports_a, AliceInput::Resume, now)
                    .await
                    .unwrap();
            }
            let mut guard = link.bob.lock().await;
            let (runner, store) = &mut *guard;
            if let Some(row) = store.active_swaps().unwrap().pop()
                && store.secret(&row.id).unwrap().is_some()
                && let Some(r) = BobRunner::resume(store, &row, pass(), now).unwrap()
            {
                *runner = r;
                runner
                    .feed(store, &link.ports, BobInput::Resume, now)
                    .await
                    .unwrap();
            }
        }
        let _ = alice.tick(&mut store_a, &ports_a, now).await;
        if let Some((ua, zats)) = &alice.payment_request
            && !paid
        {
            let zats = zats
                + if fault == Fault::TakerOverpays {
                    1_000
                } else {
                    0
                };
            script(&["zec-wallet", "send", ua, &zats.to_string()]);
            paid = true;
        }
        {
            let mut guard = link.bob.lock().await;
            let (runner, store) = &mut *guard;
            runner.tick(store, &link.ports, now).await.unwrap();
        }
        let bob_state = link.bob.lock().await.0.state;
        eprintln!(
            "{fault:?} tick {t}: alice {:?}, bob {bob_state:?}",
            alice.state
        );
        if alice.state.is_terminal() && (bob_state.is_terminal() || fault == Fault::MakerNeverLocks)
        {
            break;
        }
        script(&["mine-btc", "1"]);
        script(&["mine-zec", "1"]);
    }

    let bob_state = link.bob.lock().await.0.state;
    let id = alice.swap_id;
    let mut txs: Vec<String> = store_a
        .txs(&id)
        .unwrap()
        .into_iter()
        .map(|t| format!("engine {} {}", t.name, t.txid))
        .collect();
    txs.extend(
        link.bob
            .lock()
            .await
            .1
            .txs(&id)
            .unwrap()
            .into_iter()
            .map(|t| format!("maker {} {}", t.name, t.txid)),
    );
    println!(
        "{fault:?}: alice {:?}, bob {bob_state:?}, txs {txs:?}",
        alice.state
    );
    (alice.state, bob_state, txs)
}

fn devnet() -> bool {
    let on = std::env::var_os("ZECSPLIT_DEVNET").is_some();
    if !on {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
    }
    on
}

#[tokio::test(flavor = "multi_thread")]
async fn g4_engine_and_maker_runners_swap_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, b, _) = run(Fault::None).await;
    assert_eq!(
        (a, b),
        (
            AliceState::Done(Outcome::Swapped),
            BobState::Done(Outcome::Swapped)
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn s_02_maker_never_locks_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, _, _) = run(Fault::MakerNeverLocks).await;
    assert_eq!(a, AliceState::Aborted(AbortReason::NoLock));
}

#[tokio::test(flavor = "multi_thread")]
async fn s_03_taker_never_pays_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, b, _) = run(Fault::TakerNeverPays).await;
    assert_eq!(
        (a, b),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn s_04_maker_blind_both_refund_and_taker_sweeps_zec_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, b, txs) = run(Fault::MakerBlindToZec).await;
    assert_eq!(
        (a, b),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
    assert!(
        txs.iter().any(|t| t.starts_with("engine zec_sweep")),
        "{txs:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn s_06_overpaid_zec_gets_no_enc_sig_and_taker_sweeps_it_back_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, b, txs) = run(Fault::TakerOverpays).await;
    assert_eq!(
        (a, b),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
    assert!(
        txs.iter().any(|t| t.starts_with("engine zec_sweep")),
        "{txs:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn s_05_maker_never_refunds_taker_punishes_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, b, _) = run(Fault::MakerRefundBlocked).await;
    assert_eq!(
        (a, b),
        (
            AliceState::Done(Outcome::Punished),
            BobState::Done(Outcome::Punished)
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn s_07_both_runners_restart_every_tick_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, b, _) = run(Fault::CrashEveryTick).await;
    assert_eq!(
        (a, b),
        (
            AliceState::Done(Outcome::Swapped),
            BobState::Done(Outcome::Swapped)
        )
    );
}

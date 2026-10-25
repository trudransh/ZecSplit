//! The engine's Alice runner against the maker's Bob runner, in-process, over a fake chain, wallet and Zcash node.

#![allow(clippy::unwrap_used, clippy::type_complexity)]

mod common;
mod fakes;

use std::sync::Mutex;

use async_trait::async_trait;
use zecsplit_btc::sources::BtcSource;
use zecsplit_crypto::MakerSecret;
use zecsplit_crypto::Role;
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_maker::runner::{self as bob, BobRunner};
use zecsplit_maker::service::{Config as MakerConfig, Service};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::bob::{BobInput, BobState};
use zecsplit_protocol::machine::{AbortReason, Alert, Outcome};
use zecsplit_protocol::messages::{Envelope, Message, Network};
use zecsplit_protocol::session::Core;
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_store::Store;
use zecsplitd::runner::{self as alice, AliceRunner};

use common::{Link, pass};
use fakes::{Censor, FakeChain, FakeWallet, FakeZcash, addr, quote};

/// Runs one swap to the end; `restart` rebuilds each runner from SQLite before every tick once it is saved (S-07).
async fn swap(
    zec: &FakeZcash,
    restart: bool,
    bob_blocked: bool,
) -> (AliceState, BobState, Vec<Alert>) {
    let chain = FakeChain::default();
    let censor = Censor {
        chain: &chain,
        blocked: bob_blocked,
        lock: Mutex::new(None),
    };
    let sources: [&dyn BtcSource; 2] = [&chain, &chain];
    let dirs = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut store_a = Store::open(&dirs.0.path().join("engine.sqlite")).unwrap();
    let mut store_b = Store::open(&dirs.1.path().join("maker.sqlite")).unwrap();

    let b = Core::new(&mut OsRng, quote(), Role::Bob, addr(2)).unwrap();
    let bob_runner =
        BobRunner::start(&mut store_b, BobSession::new(b), pass(), "http://maker", 0).unwrap();
    let link = Link {
        bob: tokio::sync::Mutex::new((bob_runner, store_b)),
        ports: bob::Ports {
            sources: &sources,
            broadcast: &censor,
            wallet: &FakeWallet,
            zec,
        },
    };
    let ports_a = alice::Ports {
        maker: &link,
        sources: &sources,
        broadcast: &chain,
        zec,
    };
    let a = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
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

    for t in 1..200 {
        if restart {
            if let Some(row) = store_a.active_swaps().unwrap().pop()
                && store_a.secret(&row.id).unwrap().is_some()
                && let Some(r) = AliceRunner::resume(&mut store_a, &row, pass(), t).unwrap()
            {
                alice = r;
                alice
                    .feed(&mut store_a, &ports_a, AliceInput::Resume, t)
                    .await
                    .unwrap();
            }
            let mut guard = link.bob.lock().await;
            let (runner, store) = &mut *guard;
            if let Some(row) = store.active_swaps().unwrap().pop()
                && store.secret(&row.id).unwrap().is_some()
                && let Some(r) = BobRunner::resume(store, &row, pass(), t).unwrap()
            {
                *runner = r;
                runner
                    .feed(store, &link.ports, BobInput::Resume, t)
                    .await
                    .unwrap();
            }
        }
        alice.tick(&mut store_a, &ports_a, t).await.unwrap();
        if alice.payment_request.is_some() {
            zec.paid.lock().unwrap().get_or_insert(0);
        }
        {
            let mut guard = link.bob.lock().await;
            let (runner, store) = &mut *guard;
            runner.tick(store, &link.ports, t).await.unwrap();
        }
        let bob_state = link.bob.lock().await.0.state;
        if alice.state.is_terminal() && bob_state.is_terminal() {
            break;
        }
        chain.mine();
        zec.mine();
    }
    let bob_state = link.bob.lock().await.0.state;
    (alice.state, bob_state, alice.alerts)
}

fn swept(zec: &FakeZcash) -> Vec<(&'static str, bool)> {
    let sweeps = zec.sweeps.lock().unwrap();
    sweeps.iter().map(|s| (s.0, s.1)).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn s_01_engine_and_maker_runners_swap() {
    let zec = FakeZcash::default();
    let (alice, bob, _) = swap(&zec, false, false).await;
    assert_eq!(
        (alice, bob),
        (
            AliceState::Done(Outcome::Swapped),
            BobState::Done(Outcome::Swapped)
        )
    );
    assert_eq!(swept(&zec), [("bob", true)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn s_04_maker_never_sees_zec_both_refund() {
    let zec = FakeZcash {
        bob_blind: true,
        ..FakeZcash::default()
    };
    let (alice, bob, _) = swap(&zec, false, false).await;
    assert_eq!(
        (alice, bob),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
    assert_eq!(swept(&zec), [("alice", true)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn adv_05_wrong_zec_amount_gets_no_enc_sig_and_both_refund() {
    let zec = FakeZcash {
        wrong_amount: true,
        ..FakeZcash::default()
    };
    let (alice, bob, _) = swap(&zec, false, false).await;
    assert_eq!(
        (alice, bob),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
    assert_eq!(swept(&zec), [("alice", true)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn adv_06_enc_sig_inside_the_margin_is_not_redeemed_and_both_refund() {
    let zec = FakeZcash {
        bob_delay: 15,
        ..FakeZcash::default()
    };
    let (alice, bob, alerts) = swap(&zec, false, false).await;
    assert_eq!(
        (alice, bob),
        (
            AliceState::Done(Outcome::Refunded),
            BobState::Done(Outcome::Refunded)
        )
    );
    assert!(alerts.contains(&Alert::MarginTooSmall));
    assert_eq!(swept(&zec), [("alice", true)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn adv_07_blocked_maker_refund_lets_alice_punish_at_t2() {
    let zec = FakeZcash {
        bob_blind: true,
        ..FakeZcash::default()
    };
    let (alice, bob, _) = swap(&zec, false, true).await;
    assert_eq!(
        (alice, bob),
        (
            AliceState::Done(Outcome::Punished),
            BobState::Done(Outcome::Punished)
        )
    );
    assert_eq!(swept(&zec), []);
}

/// A maker that takes the quote and then never answers (S-02), or is down altogether.
struct Silent {
    down: bool,
}

#[async_trait]
impl alice::Maker for Silent {
    async fn send(&self, _: &Envelope) -> Result<Option<Envelope>, String> {
        Ok(None)
    }
    async fn next(&self, _: &[u8; 32]) -> Result<Option<Envelope>, String> {
        if self.down {
            return Err("maker unreachable".into());
        }
        Ok(None)
    }
}

/// S-02, and a dead maker never stops Alice's chain step.
#[tokio::test(flavor = "multi_thread")]
async fn s_02_maker_never_locks_alice_aborts_after_the_timeout() {
    for down in [false, true] {
        let chain = FakeChain::default();
        let sources: [&dyn BtcSource; 2] = [&chain, &chain];
        let zec = FakeZcash::default();
        let maker = Silent { down };
        let ports = alice::Ports {
            maker: &maker,
            sources: &sources,
            broadcast: &chain,
            zec: &zec,
        };
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("engine.sqlite")).unwrap();
        let a = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
        let mut alice =
            AliceRunner::start(&mut store, AliceSession::new(a), pass(), "http://maker", 0)
                .unwrap();
        alice
            .feed(&mut store, &ports, AliceInput::Begin, 0)
            .await
            .unwrap();
        let early = alice
            .tick(&mut store, &ports, alice::LOCK_TIMEOUT_SECS - 1)
            .await;
        assert_eq!(early.is_err(), down);
        assert!(!alice.state.is_terminal());
        let late = alice
            .tick(&mut store, &ports, alice::LOCK_TIMEOUT_SECS)
            .await;
        assert_eq!(late.is_err(), down);
        assert_eq!(alice.state, AliceState::Aborted(AbortReason::NoLock));
        assert!(alice.payment_request.is_none());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn s_07_both_runners_restart_before_every_tick() {
    let zec = FakeZcash::default();
    let (alice, bob, _) = swap(&zec, true, false).await;
    assert_eq!(
        (alice, bob),
        (
            AliceState::Done(Outcome::Swapped),
            BobState::Done(Outcome::Swapped)
        )
    );
    assert_eq!(swept(&zec), [("bob", true)]);
}

/// ADV-08: quote spam holds at most the §8 swap slots, and a stalled setup expires.
#[tokio::test(flavor = "multi_thread")]
async fn adv_08_quote_flood_is_capped_and_stalled_setups_expire() {
    let chain: &'static FakeChain = Box::leak(Box::default());
    let zec: &'static FakeZcash = Box::leak(Box::default());
    let sources: &'static [&'static dyn BtcSource] =
        Box::leak(Box::new([chain as &dyn BtcSource, chain]));
    let ports = || bob::Ports {
        sources,
        broadcast: chain,
        wallet: &FakeWallet,
        zec,
    };
    let dir = tempfile::tempdir().unwrap();
    let config = MakerConfig {
        network: Network::Regtest,
        sats_per_zec: 500_000,
        min_zec: 1_000_000,
        max_zec: 200_000_000,
        btc_fee_rate: 2,
        endpoint: "http://maker".into(),
        passphrase: pass(),
    };
    let store = Store::open(&dir.path().join("maker.sqlite")).unwrap();
    let mut svc =
        Service::open(config, MakerSecret::from_bytes([0x11; 32]), store, ports()).unwrap();
    let req = zecsplit_protocol::messages::QuoteRequest {
        zec_amount: 10_000_000,
        network: Network::Regtest,
    };
    for _ in 0..zecsplit_maker::service::max_swaps(Network::Regtest) {
        assert!(matches!(svc.quote(&req).await.message, Message::Quote(_)));
    }
    assert!(matches!(svc.quote(&req).await.message, Message::Error(_)));

    let mut store = Store::open(&dir.path().join("other.sqlite")).unwrap();
    let b = Core::new(&mut OsRng, quote(), Role::Bob, addr(2)).unwrap();
    let mut runner =
        BobRunner::start(&mut store, BobSession::new(b), pass(), "http://maker", 0).unwrap();
    let timeout = bob::SETUP_TIMEOUT_SECS;
    runner
        .tick(&mut store, &ports(), timeout - 1)
        .await
        .unwrap();
    assert_eq!(runner.state, BobState::Quoted);
    runner.tick(&mut store, &ports(), timeout).await.unwrap();
    assert_eq!(runner.state, BobState::Aborted(AbortReason::NoLock));
}

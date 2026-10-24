//! The engine's Alice runner against the maker's Bob runner, in-process, over a fake chain, wallet and Zcash node;
//! then the engine API against the maker service over HTTP on the same fakes.

#![allow(clippy::unwrap_used, clippy::type_complexity)]

mod common;

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Address, Amount, CompressedPublicKey, Psbt, ScriptBuf, Transaction, Txid};
use reddsa::orchard::SpendAuth;
use zecsplit_btc::sources::{Broadcast, BtcSource, SourceError, SourceTx};
use zecsplit_crypto::rand_core::OsRng;
use zecsplit_crypto::{Ask, Role};
use zecsplit_maker::runner::{self as bob, BobRunner, Wallet, ZecNode};
use zecsplit_protocol::alice::{AliceInput, AliceState};
use zecsplit_protocol::bob::{BobInput, BobState};
use zecsplit_protocol::machine::{AbortReason, Alert, Outcome, ZecLock};
use zecsplit_protocol::messages::{Envelope, Network, Quote};
use zecsplit_protocol::session::alice::AliceSession;
use zecsplit_protocol::session::bob::BobSession;
use zecsplit_protocol::session::{Core, Shared};
use zecsplit_store::Store;
use zecsplitd::runner::{self as alice, AliceRunner, ZecSweep};

use bitcoin::base64::Engine as _;
use bitcoin::base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::json;
use zecsplit_crypto::{MakerSecret, maker_public_key};
use zecsplit_maker::service::{Config as MakerConfig, Service};
use zecsplit_protocol::recovery::{SecretString, open};
use zecsplitd::engine::{Config as EngineConfig, Engine};
use zecsplitd::http::Access;
use zecsplitd::maker_client::HttpMaker;

use common::{Link, pass};

#[derive(Default)]
struct FakeChain(Mutex<(u32, HashMap<Txid, (Transaction, Option<u32>)>)>);

impl FakeChain {
    fn mine(&self) {
        let mut c = self.0.lock().unwrap();
        c.0 += 1;
        let tip = c.0;
        for (_, h) in c.1.values_mut() {
            h.get_or_insert(tip);
        }
    }
}

#[async_trait]
impl BtcSource for FakeChain {
    async fn tip(&self) -> Result<u32, SourceError> {
        Ok(self.0.lock().unwrap().0)
    }
    async fn tx(&self, txid: &Txid) -> Result<Option<SourceTx>, SourceError> {
        let c = self.0.lock().unwrap();
        Ok(c.1.get(txid).map(|(tx, height)| SourceTx {
            tx: tx.clone(),
            height: *height,
        }))
    }
}

#[async_trait]
impl Broadcast for FakeChain {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        let mut c = self.0.lock().unwrap();
        c.1.entry(tx.compute_txid()).or_insert((tx.clone(), None));
        Ok(())
    }
}

/// Bob's line to `FakeChain` that, when `blocked`, carries only his `tx_lock` (ADV-07).
struct Censor<'a> {
    chain: &'a FakeChain,
    blocked: bool,
    lock: Mutex<Option<Txid>>,
}

#[async_trait]
impl Broadcast for Censor<'_> {
    async fn broadcast(&self, tx: &Transaction) -> Result<(), SourceError> {
        let txid = tx.compute_txid();
        if self.blocked && *self.lock.lock().unwrap().get_or_insert(txid) != txid {
            return Ok(());
        }
        self.chain.broadcast(tx).await
    }
}

struct FakeWallet;

#[async_trait]
impl Wallet for FakeWallet {
    async fn refund_address(&self) -> Result<String, String> {
        Ok(addr(3))
    }
    async fn fund(&self, spk: &ScriptBuf, value: Amount) -> Result<Psbt, String> {
        Ok(lock_psbt(spk.clone(), value))
    }
    async fn sign(&self, psbt: &Psbt) -> Result<Transaction, String> {
        Ok(psbt.unsigned_tx.clone())
    }
}

/// One Zcash chain both parties see; `bob_blind` hides the lock from Bob (S-04), `wrong_amount` mispays it (ADV-05), `bob_delay` shows it to Bob only at that many confirmations (ADV-06).
#[derive(Default)]
struct FakeZcash {
    bob_blind: bool,
    wrong_amount: bool,
    bob_delay: u32,
    paid: Mutex<Option<u32>>,
    sweeps: Mutex<Vec<(&'static str, bool, u32)>>,
}

impl FakeZcash {
    fn mine(&self) {
        if let Some(c) = self.paid.lock().unwrap().as_mut() {
            *c += 1;
        }
        for s in self.sweeps.lock().unwrap().iter_mut() {
            s.2 += 1;
        }
    }
    fn sweep_as(&self, who: &'static str, shared: &Shared, ask: &Ask) -> String {
        self.sweeps
            .lock()
            .unwrap()
            .push((who, signs_for(ask, shared), 0));
        format!("sweep-{who}")
    }
    fn confs(&self, txid: &str) -> Option<u32> {
        let sweeps = self.sweeps.lock().unwrap();
        sweeps
            .iter()
            .find(|s| format!("sweep-{}", s.0) == txid)
            .map(|s| s.2)
    }
}

#[async_trait]
impl ZecSweep for FakeZcash {
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String> {
        Ok(self.sweep_as("alice", shared, ask))
    }
    async fn confirmations(&self, _: &Shared, txid: &str) -> Result<Option<u32>, String> {
        Ok(self.confs(txid))
    }
}

#[async_trait]
impl ZecNode for FakeZcash {
    async fn lock(&self, _: &Shared, _: u64, _: u32) -> Result<ZecLock, String> {
        Ok(match *self.paid.lock().unwrap() {
            Some(_) if self.wrong_amount => ZecLock::WrongAmount,
            Some(confirmations) if !self.bob_blind && confirmations >= self.bob_delay => {
                ZecLock::Correct { confirmations }
            }
            _ => ZecLock::Unseen,
        })
    }
    async fn sweep(&self, shared: &Shared, ask: &Ask) -> Result<String, String> {
        Ok(self.sweep_as("bob", shared, ask))
    }
    async fn confirmations(&self, _: &Shared, txid: &str) -> Result<Option<u32>, String> {
        Ok(self.confs(txid))
    }
}

fn quote() -> Quote {
    Quote {
        protocol_version: 1,
        swap_id: [0x77; 32],
        network: Network::Regtest,
        zec_amount: 100_000_000,
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

fn addr(seed: u8) -> String {
    let sk = bitcoin::secp256k1::SecretKey::from_slice(&[seed; 32]).unwrap();
    let pk = CompressedPublicKey(sk.public_key(&Secp256k1::new()));
    Address::p2wpkh(&pk, bitcoin::Network::Regtest).to_string()
}

fn lock_psbt(lock_spk: ScriptBuf, v_lock: Amount) -> Psbt {
    use bitcoin::hashes::Hash;
    use bitcoin::{OutPoint, Sequence, TxIn, TxOut, WPubkeyHash, Witness};
    let tx = Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::from_byte_array([1; 32]), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: v_lock,
            script_pubkey: lock_spk,
        }],
    };
    let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(1_000_000),
        script_pubkey: ScriptBuf::new_p2wpkh(&WPubkeyHash::from_byte_array([0xcc; 20])),
    });
    psbt
}

fn signs_for(ask: &Ask, shared: &Shared) -> bool {
    let sk = reddsa::SigningKey::<SpendAuth>::from_bytes(ask.expose_secret()).unwrap();
    <[u8; 32]>::from(reddsa::VerificationKey::from(&sk)) == shared.ak
}

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

/// A maker that takes the quote and then never answers (S-02).
struct Silent;

#[async_trait]
impl alice::Maker for Silent {
    async fn send(&self, _: &Envelope) -> Result<Option<Envelope>, String> {
        Ok(None)
    }
    async fn next(&self, _: &[u8; 32]) -> Result<Option<Envelope>, String> {
        Ok(None)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn s_02_maker_never_locks_alice_aborts_after_the_timeout() {
    let chain = FakeChain::default();
    let sources: [&dyn BtcSource; 2] = [&chain, &chain];
    let zec = FakeZcash::default();
    let ports = alice::Ports {
        maker: &Silent,
        sources: &sources,
        broadcast: &chain,
        zec: &zec,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("engine.sqlite")).unwrap();
    let a = Core::new(&mut OsRng, quote(), Role::Alice, addr(1)).unwrap();
    let mut alice =
        AliceRunner::start(&mut store, AliceSession::new(a), pass(), "http://maker", 0).unwrap();
    alice
        .feed(&mut store, &ports, AliceInput::Begin, 0)
        .await
        .unwrap();
    alice
        .tick(&mut store, &ports, alice::LOCK_TIMEOUT_SECS - 1)
        .await
        .unwrap();
    assert!(!alice.state.is_terminal());
    alice
        .tick(&mut store, &ports, alice::LOCK_TIMEOUT_SECS)
        .await
        .unwrap();
    assert_eq!(alice.state, AliceState::Aborted(AbortReason::NoLock));
    assert!(alice.payment_request.is_none());
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

const UI: &str = "http://ui";
const TOKEN: &str = "test-token";
const ENGINE_PASS: &str = "engine passphrase";

async fn serve(router: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// One call from the UI's origin with the token; every body is kept for the secret check.
async fn call(
    base: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
    seen: &mut String,
) -> (u16, String) {
    let mut req = reqwest::Client::new()
        .request(method, format!("{base}{path}"))
        .header("origin", UI)
        .bearer_auth(TOKEN);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.unwrap();
    let status = res.status().as_u16();
    let text = res.text().await.unwrap();
    seen.push_str(&text);
    (status, text)
}

type SharedEngine = std::sync::Arc<tokio::sync::Mutex<Engine>>;

fn engine(
    dir: &std::path::Path,
    maker: &'static HttpMaker,
    sources: &'static [&'static dyn BtcSource],
    chain: &'static FakeChain,
    zec: &'static FakeZcash,
) -> SharedEngine {
    let config = EngineConfig {
        network: Network::Regtest,
        maker_url: "http://maker".into(),
        maker_key: maker_public_key(&MakerSecret::from_bytes([0x11; 32])).unwrap(),
    };
    let ports = alice::Ports {
        maker,
        sources,
        broadcast: chain,
        zec,
    };
    let store = Store::open(&dir.join("engine.sqlite")).unwrap();
    std::sync::Arc::new(tokio::sync::Mutex::new(Engine::new(
        config, store, maker, ports,
    )))
}

fn api(engine: SharedEngine) -> axum::Router {
    let access = Access {
        token: TOKEN.into(),
        origins: vec![UI.into()],
    };
    zecsplitd::http::router(engine, access)
}

/// T-034: S-01 through the engine API against the maker service over HTTP; no response carries
/// a secret (INTERFACES §3), and the recovery file restores the swap in a fresh engine (§6.4).
#[tokio::test(flavor = "multi_thread")]
async fn t_034_engine_api_runs_s_01_and_never_returns_a_secret() {
    let chain: &'static FakeChain = Box::leak(Box::default());
    let zec: &'static FakeZcash = Box::leak(Box::default());
    let sources: &'static [&'static dyn BtcSource] =
        Box::leak(Box::new([chain as &dyn BtcSource, chain]));
    let dirs = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );

    let maker_config = MakerConfig {
        network: Network::Regtest,
        sats_per_zec: 500_000,
        min_zec: 1_000_000,
        max_zec: 200_000_000,
        btc_fee_rate: 2,
        endpoint: "http://maker".into(),
        passphrase: pass(),
    };
    let maker_ports = bob::Ports {
        sources,
        broadcast: chain,
        wallet: &FakeWallet,
        zec,
    };
    let maker_store = Store::open(&dirs.0.path().join("maker.sqlite")).unwrap();
    let identity = MakerSecret::from_bytes([0x11; 32]);
    let svc = Service::open(maker_config, identity, maker_store, maker_ports).unwrap();
    let svc = std::sync::Arc::new(tokio::sync::Mutex::new(svc));
    let maker_url = serve(zecsplit_maker::http::router(svc.clone())).await;
    let maker: &'static HttpMaker = Box::leak(Box::new(HttpMaker::new(&maker_url).unwrap()));

    let eng = engine(dirs.1.path(), maker, sources, chain, zec);
    let base = serve(api(eng.clone())).await;
    let mut seen = String::new();
    let (get, post) = (reqwest::Method::GET, reqwest::Method::POST);

    let bare = reqwest::get(format!("{base}/api/health")).await.unwrap();
    assert_eq!(bare.status().as_u16(), 401, "no token");
    let foreign = reqwest::Client::new()
        .get(format!("{base}/api/health"))
        .header("origin", "http://evil")
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status().as_u16(), 403, "foreign origin");

    let short = json!({"passphrase": "short"});
    let (status, _) = call(&base, post.clone(), "/api/unlock", Some(short), &mut seen).await;
    assert_eq!(status, 400);
    let unlock = json!({"passphrase": ENGINE_PASS});
    let (status, _) = call(&base, post.clone(), "/api/unlock", Some(unlock), &mut seen).await;
    assert_eq!(status, 200);
    let ask = json!({"zec_amount": 30_000_000});
    let (status, quote) = call(&base, post.clone(), "/api/quote", Some(ask), &mut seen).await;
    assert_eq!(status, 200, "{quote}");
    let quote: serde_json::Value = serde_json::from_str(&quote).unwrap();
    assert_eq!(quote["btc_amount"], 150_000);
    let create =
        json!({"quote_id": quote["quote_id"], "btc_address": addr(1), "accept_risks": true});
    let (status, created) = call(&base, post.clone(), "/api/swaps", Some(create), &mut seen).await;
    assert_eq!(status, 200, "{created}");
    let created: serde_json::Value = serde_json::from_str(&created).unwrap();
    let id = created["swap_id"].as_str().unwrap().to_string();
    let (view_path, file_path) = (
        format!("/api/swaps/{id}"),
        format!("/api/swaps/{id}/recovery-file"),
    );

    let mut file = None;
    let mut view = serde_json::Value::Null;
    for _ in 0..200 {
        if let Some(failure) = eng.lock().await.tick().await.pop() {
            panic!("engine tick: {failure}");
        }
        if let Some(failure) = svc.lock().await.tick().await.pop() {
            panic!("maker tick: {failure}");
        }
        let (_, body) = call(&base, get.clone(), &view_path, None, &mut seen).await;
        view = serde_json::from_str(&body).unwrap();
        if file.is_none() {
            let (status, text) = call(&base, get.clone(), &file_path, None, &mut seen).await;
            if status == 200 {
                assert!(
                    view["zip321_uri"].is_null(),
                    "UX_SPEC §2.4: file before payment"
                );
                let saved = format!("/api/swaps/{id}/recovery-saved");
                call(&base, post.clone(), &saved, None, &mut seen).await;
                file = Some(text);
            }
        }
        if let Some(uri) = view["zip321_uri"].as_str() {
            assert!(
                uri.starts_with("zcash:") && uri.ends_with("?amount=0.3"),
                "{uri}"
            );
            zec.paid.lock().unwrap().get_or_insert(0);
        }
        if view["outcome"] == "redeemed" {
            break;
        }
        chain.mine();
        zec.mine();
    }
    assert_eq!(view["outcome"], "redeemed", "{view}");
    let steps = view["steps"].as_array().unwrap();
    assert!(steps.iter().all(|s| s["status"] == "confirmed"), "{view}");
    let (_, list) = call(&base, get.clone(), "/api/swaps", None, &mut seen).await;
    assert!(list.contains(&id));

    let file = file.unwrap();
    let sealed = BASE64.decode(file.trim()).unwrap();
    let bundle = open(&sealed, SecretString::from(ENGINE_PASS.to_owned())).unwrap();
    for secret in [
        hex::encode(bundle.share.expose_secret()),
        hex::encode(bundle.btc_key.expose_secret()),
        ENGINE_PASS.to_string(),
    ] {
        assert!(!seen.contains(&secret), "a response leaked a secret");
    }

    let fresh = engine(dirs.2.path(), maker, sources, chain, zec);
    let base = serve(api(fresh.clone())).await;
    let unlock = json!({"passphrase": "another passphrase"});
    call(&base, post.clone(), "/api/unlock", Some(unlock), &mut seen).await;
    let wrong = json!({"recovery_file": file, "passphrase": "not the passphrase"});
    let (status, _) = call(&base, post.clone(), "/api/restore", Some(wrong), &mut seen).await;
    assert_eq!(status, 400);
    let restore = json!({"recovery_file": file, "passphrase": ENGINE_PASS});
    let (status, body) = call(
        &base,
        post.clone(),
        "/api/restore",
        Some(restore.clone()),
        &mut seen,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, _) = call(&base, post, "/api/restore", Some(restore), &mut seen).await;
    assert_eq!(status, 409, "restoring twice");
    fresh.lock().await.tick().await;
    let (_, body) = call(&base, get, &view_path, None, &mut seen).await;
    let restored: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        restored["outcome"], "redeemed",
        "the chain says how it ended: {restored}"
    );
}

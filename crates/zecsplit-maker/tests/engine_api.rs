//! T-034: the engine API end to end against the maker service over HTTP, in its own test
//! binary so the swap tests' CPU load never pushes its maker calls past their timeout.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;
mod fakes;

use bitcoin::base64::Engine as _;
use bitcoin::base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::json;
use zecsplit_btc::sources::BtcSource;
use zecsplit_crypto::{MakerSecret, maker_public_key};
use zecsplit_maker::runner as bob;
use zecsplit_maker::service::{Config as MakerConfig, Service};
use zecsplit_protocol::messages::Network;
use zecsplit_protocol::recovery::{SecretString, open};
use zecsplit_store::Store;
use zecsplitd::engine::{Config as EngineConfig, Engine};
use zecsplitd::http::Access;
use zecsplitd::maker_client::HttpMaker;
use zecsplitd::runner as alice;

use common::pass;
use fakes::{FakeChain, FakeWallet, FakeZcash, addr};

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
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let mut req = CLIENT
        .get_or_init(reqwest::Client::new)
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
    zecsplit_btc::sources::tls();
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

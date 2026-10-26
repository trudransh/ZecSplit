//! Taker engine: localhost HTTP API + SSE, swap runner, resume on unlock.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use zcash_protocol::consensus::Network::TestNetwork;
use zecsplit_btc::sources::{BitcoindRpc, BtcSource, Esplora};
use zecsplit_crypto::rand_core::{OsRng, RngCore};
use zecsplit_protocol::messages::Network;
use zecsplit_store::Store;
use zecsplit_zcash::{LockWallets, regtest};
use zecsplitd::engine::{Config, Engine};
use zecsplitd::http::{Access, router};
use zecsplitd::maker_client::HttpMaker;
use zecsplitd::runner::{Ports, ZecSweep};

/// Seconds between ticks outside regtest: each tick asks the public Esplora about every swap
/// transaction, and 10-minute signet blocks need nothing faster (ADR-053).
const PUBLIC_TICK_SECS: u64 = 15;

fn var(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn var_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

/// Reads `ZECSPLIT_ENGINE_*`, then serves INTERFACES §3 on loopback and ticks every swap.
async fn run() -> Result<(), String> {
    let network = match var("ZECSPLIT_ENGINE_NETWORK")?.as_str() {
        "regtest" => Network::Regtest,
        "test" => Network::Test,
        other => return Err(format!("network {other} is not supported")),
    };
    let listen: SocketAddr = var_or("ZECSPLIT_ENGINE_LISTEN", "127.0.0.1:7780")
        .parse()
        .map_err(|e| format!("ZECSPLIT_ENGINE_LISTEN: {e}"))?;
    if !listen.ip().is_loopback() {
        return Err("INTERFACES §3: the engine binds to 127.0.0.1 only".into());
    }
    let mut maker_key = [0; 32];
    hex::decode_to_slice(var("ZECSPLIT_ENGINE_MAKER_KEY")?, &mut maker_key)
        .map_err(|e| format!("ZECSPLIT_ENGINE_MAKER_KEY: {e}"))?;
    let maker_url = var("ZECSPLIT_ENGINE_MAKER")?;
    let data = PathBuf::from(var("ZECSPLIT_ENGINE_DATA")?);
    let zec_dir = data.join("zec");
    std::fs::create_dir_all(&zec_dir).map_err(|e| e.to_string())?;

    let (node, user, pass) = (
        var("ZECSPLIT_ENGINE_BITCOIND")?,
        var("ZECSPLIT_ENGINE_RPC_USER")?,
        var("ZECSPLIT_ENGINE_RPC_PASS")?,
    );
    let source_a: &'static BitcoindRpc = Box::leak(Box::new(
        BitcoindRpc::new(&node, &user, &pass).map_err(|e| e.to_string())?,
    ));
    let source_b: &'static dyn BtcSource = match std::env::var("ZECSPLIT_ENGINE_ESPLORA") {
        Ok(url) => Box::leak(Box::new(Esplora::new(&url).map_err(|e| e.to_string())?)),
        Err(_) if network == Network::Regtest => source_a,
        Err(_) => return Err("§4.2-R1: set ZECSPLIT_ENGINE_ESPLORA as a second source".into()),
    };
    let sources: &'static [&'static dyn BtcSource] =
        Box::leak(Box::new([source_a as &dyn BtcSource, source_b]));
    let maker: &'static HttpMaker = Box::leak(Box::new(HttpMaker::new(&maker_url)?));
    let (server, refund) = (
        var("ZECSPLIT_ENGINE_LIGHT_SERVER")?,
        var("ZECSPLIT_ENGINE_ZEC_REFUND")?,
    );
    let zec: &'static dyn ZecSweep = match network {
        Network::Regtest => Box::leak(Box::new(LockWallets::new(
            regtest(),
            zec_dir,
            &server,
            &refund,
        ))),
        Network::Test | Network::MainBeta => Box::leak(Box::new(LockWallets::new(
            TestNetwork,
            zec_dir,
            &server,
            &refund,
        ))),
    };
    let ports = Ports {
        maker,
        sources,
        broadcast: source_a,
        zec,
    };
    let store = Store::open(&data.join("engine.sqlite")).map_err(|e| e.to_string())?;
    let config = Config {
        network,
        maker_url,
        maker_key,
    };
    let engine = Arc::new(Mutex::new(Engine::new(config, store, maker, ports)));

    let token = match std::env::var("ZECSPLIT_ENGINE_TOKEN") {
        Ok(t) => t,
        Err(_) => {
            let mut bytes = [0; 32];
            OsRng.fill_bytes(&mut bytes);
            hex::encode(bytes)
        }
    };
    let token_file = data.join("ui-token");
    std::fs::write(&token_file, &token).map_err(|e| e.to_string())?;
    let ui = var_or("ZECSPLIT_ENGINE_UI", "http://localhost:3000");
    let origins = var_or("ZECSPLIT_ENGINE_ORIGINS", &ui)
        .split(',')
        .map(|o| o.trim().trim_end_matches('/').to_string())
        .collect();

    let every = Duration::from_secs(match network {
        Network::Regtest => 1,
        Network::Test | Network::MainBeta => PUBLIC_TICK_SECS,
    });
    let ticker = engine.clone();
    tokio::spawn(async move {
        loop {
            for failure in ticker.lock().await.tick().await {
                eprintln!("tick failed: {failure}");
            }
            tokio::time::sleep(every).await;
        }
    });
    let socket = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|e| format!("{listen}: {e}"))?;
    eprintln!(
        "engine listening on {listen}; UI token in {}\nopen {}/swap#engine=http://{listen}&token={token}",
        token_file.display(),
        ui.trim_end_matches('/'),
    );
    axum::serve(socket, router(engine, Access { token, origins }))
        .await
        .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zecsplitd: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

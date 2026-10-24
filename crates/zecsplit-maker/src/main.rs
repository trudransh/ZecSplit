//! Maker service: signed quotes, limits, Bob runner, ops CLI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use zecsplit_btc::sources::{BitcoindRpc, BtcSource};
use zecsplit_crypto::MakerSecret;
use zecsplit_maker::http::router;
use zecsplit_maker::ports::BitcoindWallet;
use zecsplit_maker::runner::Ports;
use zecsplit_maker::service::{Config, Service, owned_lock_server};
use zecsplit_protocol::messages::Network;
use zecsplit_protocol::recovery::SecretString;
use zecsplit_store::Store;
use zecsplit_zcash::{LockWallets, regtest};

fn var(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn num(name: &str) -> Result<u64, String> {
    var(name)?.parse().map_err(|e| format!("{name}: {e}"))
}

/// Reads `ZECSPLIT_MAKER_*`, resumes stored swaps, then serves INTERFACES §2 and ticks every swap.
async fn run() -> Result<(), String> {
    let network = match var("ZECSPLIT_MAKER_NETWORK")?.as_str() {
        "regtest" => Network::Regtest,
        "test" => Network::Test,
        "main_beta" => Network::MainBeta,
        other => return Err(format!("unknown network {other}")),
    };
    owned_lock_server(network, &var("ZECSPLIT_MAKER_LIGHT_SERVER")?)?;
    if network != Network::Regtest {
        return Err("only regtest is supported yet".into());
    }
    let mut identity = [0; 32];
    hex::decode_to_slice(var("ZECSPLIT_MAKER_IDENTITY")?, &mut identity)
        .map_err(|e| format!("ZECSPLIT_MAKER_IDENTITY: {e}"))?;
    let data = PathBuf::from(var("ZECSPLIT_MAKER_DATA")?);
    let zec_dir = data.join("zec");
    std::fs::create_dir_all(&zec_dir).map_err(|e| e.to_string())?;
    let (node, user, pass) = (
        var("ZECSPLIT_MAKER_BITCOIND")?,
        var("ZECSPLIT_MAKER_RPC_USER")?,
        var("ZECSPLIT_MAKER_RPC_PASS")?,
    );
    let rpc = |path: &str| BitcoindRpc::new(&format!("{node}{path}"), &user, &pass);
    let node_rpc: &'static BitcoindRpc = Box::leak(Box::new(rpc("").map_err(|e| e.to_string())?));
    let sources: &'static [&'static dyn BtcSource] = Box::leak(Box::new([
        node_rpc as &dyn BtcSource,
        node_rpc as &dyn BtcSource,
    ]));
    let wallet = Box::leak(Box::new(BitcoindWallet::new(
        rpc(&format!("/wallet/{}", var("ZECSPLIT_MAKER_WALLET")?)).map_err(|e| e.to_string())?,
        bitcoin::Network::Regtest,
    )));
    let zec = Box::leak(Box::new(LockWallets::new(
        regtest(),
        zec_dir,
        &var("ZECSPLIT_MAKER_LIGHT_SERVER")?,
        &var("ZECSPLIT_MAKER_ZEC_PAYOUT")?,
    )));
    let ports = Ports {
        sources,
        broadcast: node_rpc,
        wallet,
        zec,
    };
    let listen = var("ZECSPLIT_MAKER_LISTEN")?;
    let config = Config {
        network,
        sats_per_zec: num("ZECSPLIT_MAKER_SATS_PER_ZEC")?,
        min_zec: num("ZECSPLIT_MAKER_MIN_ZEC")?,
        max_zec: num("ZECSPLIT_MAKER_MAX_ZEC")?,
        btc_fee_rate: 2,
        endpoint: format!("http://{listen}"),
        passphrase: SecretString::from(var("ZECSPLIT_MAKER_PASSPHRASE")?),
    };
    let store = Store::open(&data.join("maker.sqlite")).map_err(|e| e.to_string())?;
    let svc = Service::open(config, MakerSecret::from_bytes(identity), store, ports)
        .map_err(|e| e.to_string())?;
    let svc = Arc::new(Mutex::new(svc));

    let ticker = svc.clone();
    tokio::spawn(async move {
        loop {
            for failure in ticker.lock().await.tick().await {
                eprintln!("tick failed: {failure}");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    let socket = tokio::net::TcpListener::bind(&listen)
        .await
        .map_err(|e| format!("{listen}: {e}"))?;
    eprintln!("maker listening on {listen}");
    axum::serve(socket, router(svc))
        .await
        .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zecsplit-maker: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

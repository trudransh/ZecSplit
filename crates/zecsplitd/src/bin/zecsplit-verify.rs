//! Checks a finished swap's public record against a public Bitcoin chain, or exports one.

use std::path::Path;
use std::str::FromStr;

use bitcoin::Txid;
use zecsplit_btc::sources::{BtcSource, Esplora};
use zecsplit_protocol::messages::Network;
use zecsplit_protocol::recovery::{SecretString, open};
use zecsplit_protocol::verify::{PublicRecord, record_from_alice, verify};
use zecsplit_store::Store;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage: zecsplit-verify <record.json> [esplora-url]
       zecsplit-verify export <engine.sqlite> <swap-id>   (passphrase in ZECSPLIT_PASSPHRASE)";

/// Writes the public record of one finished swap from the engine's database to stdout.
fn export(db: &str, id_hex: &str) -> Res<()> {
    let id: [u8; 32] = hex::decode(id_hex)?
        .try_into()
        .map_err(|_| "swap id is 32 bytes of hex")?;
    let store = Store::open(Path::new(db))?;
    let sealed = store
        .secret(&id)?
        .ok_or("no sealed session for this swap")?;
    let pass = SecretString::from(std::env::var("ZECSPLIT_PASSPHRASE")?);
    let bundle = open(&sealed, pass)?;
    let redeem = store
        .txs(&id)?
        .into_iter()
        .find(|t| t.name == "tx_redeem")
        .ok_or("this swap has no tx_redeem")?;
    let record = record_from_alice(&bundle, &redeem.txid)?;
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

/// Fetches `tx_redeem` from Esplora and runs every check on the record.
async fn check(path: &str, esplora: Option<&str>) -> Res<()> {
    let record: PublicRecord = serde_json::from_slice(&std::fs::read(path)?)?;
    let base = match (esplora, record.quote.network) {
        (Some(url), _) => url,
        (None, Network::Test) => "https://mempool.space/signet/api",
        (None, _) => return Err("pass an Esplora URL for this network".into()),
    };
    let txid = Txid::from_str(&record.redeem_txid)?;
    let redeem = Esplora::new(base)?
        .tx(&txid)
        .await?
        .ok_or("tx_redeem is not on this chain")?;
    println!("swap      {}", hex::encode(record.quote.swap_id));
    println!("tx_redeem {txid} (block {:?}) from {base}", redeem.height);
    verify(&record, &redeem.tx)?;
    println!("ok  Alice's and Bob's reveals open their commitments");
    println!(
        "ok  both cross-group DLEq proofs: each Zcash share point has the same secret as its Bitcoin twin"
    );
    println!("ok  all six proofs of knowledge");
    println!("ok  tx_redeem pays Alice's committed payout address");
    println!("ok  Bob's signature in tx_redeem, with his adaptor signature, yields s_a;");
    println!("    s_a opens Alice's committed Zcash share point, so Bob could sweep the ZEC");
    Ok(())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["export", db, id] => export(db, id),
        [path] => check(path, None).await,
        [path, url] => check(path, Some(url)).await,
        _ => Err(USAGE.into()),
    };
    if let Err(e) = run {
        eprintln!("FAIL {e}");
        std::process::exit(1);
    }
}

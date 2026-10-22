//! T-020 on the regtest devnet: two sources agree on a real payment (§4.2-R1).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bitcoin::{Amount, ScriptBuf, TxOut, Txid};
use serde_json::json;
use zecsplit_btc::sources::{Agreement, BitcoindRpc, BtcSource, confirmed_on_all, tip_on_all};

const NODE: &str = "http://127.0.0.1:18443";

fn devnet() -> bool {
    let on = std::env::var_os("ZECSPLIT_DEVNET").is_some();
    if !on {
        eprintln!("skipped: set ZECSPLIT_DEVNET=1 with `just devnet-up` running");
    }
    on
}

fn rpc(path: &str) -> BitcoindRpc {
    BitcoindRpc::new(&format!("{NODE}{path}"), "devnet", "devnet").unwrap()
}

#[tokio::test]
async fn spec_4_2_r1_two_rpc_sources_agree_on_regtest() {
    if !devnet() {
        return;
    }
    let (a, b, wallet) = (rpc(""), rpc(""), rpc("/wallet/devnet"));
    let sources: [&dyn BtcSource; 2] = [&a, &b];

    let addr = wallet.call("getnewaddress", json!([])).await.unwrap();
    let info = wallet.call("getaddressinfo", json!([addr])).await.unwrap();
    let expect = TxOut {
        value: Amount::from_sat(50_000),
        script_pubkey: ScriptBuf::from_hex(info["scriptPubKey"].as_str().unwrap()).unwrap(),
    };
    let sent = wallet
        .call("sendtoaddress", json!([addr, expect.value.to_btc()]))
        .await
        .unwrap();
    let txid: Txid = sent.as_str().unwrap().parse().unwrap();

    let got = confirmed_on_all(&sources, &txid, &expect).await.unwrap();
    assert_eq!(got, Agreement::Mempool);

    wallet
        .call("generatetoaddress", json!([3, addr]))
        .await
        .unwrap();
    let tip = tip_on_all(&sources).await.unwrap();
    let got = confirmed_on_all(&sources, &txid, &expect).await.unwrap();
    assert_eq!(got, Agreement::Confirmed { height: tip - 2 });

    let mut wrong = expect.clone();
    wrong.value = Amount::from_sat(1);
    let got = confirmed_on_all(&sources, &txid, &wrong).await.unwrap();
    assert_eq!(got, Agreement::Disagree);

    let unknown = Txid::from_raw_hash(bitcoin::hashes::Hash::all_zeros());
    let got = confirmed_on_all(&sources, &unknown, &expect).await.unwrap();
    assert_eq!(got, Agreement::Unseen);
}

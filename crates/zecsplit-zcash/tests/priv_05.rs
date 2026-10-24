//! PRIV-05: nothing here can ask the light server for a transaction by txid.

const SOURCES: [(&str, &str); 5] = [
    ("lib.rs", include_str!("../src/lib.rs")),
    ("lock.rs", include_str!("../src/lock.rs")),
    ("watch.rs", include_str!("../src/watch.rs")),
    ("sweep.rs", include_str!("../src/sweep.rs")),
    ("wallets.rs", include_str!("../src/wallets.rs")),
];

#[test]
fn priv_05_no_get_transaction_request_is_ever_built() {
    for (name, src) in SOURCES {
        assert!(
            !src.contains("TxFilter"),
            "{name} builds a GetTransaction request"
        );
        assert!(
            !src.contains("client.get_transaction"),
            "{name} calls GetTransaction"
        );
        assert!(
            !src.contains("get_taddress"),
            "{name} queries a transparent address"
        );
    }
}

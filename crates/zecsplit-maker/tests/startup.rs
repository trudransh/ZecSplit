//! The maker binary refuses unsafe configurations before it serves anything.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

#[test]
fn adv_04_maker_will_not_start_on_testnet_with_a_foreign_lock_server() {
    let out = Command::new(env!("CARGO_BIN_EXE_zecsplit-maker"))
        .env_clear()
        .env("ZECSPLIT_MAKER_NETWORK", "test")
        .env("ZECSPLIT_MAKER_LIGHT_SERVER", "https://zec.rocks:443")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("§4.3-R4"));
}

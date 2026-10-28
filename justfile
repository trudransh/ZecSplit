# Once per clone: turn on the commit hooks.
setup:
    git config core.hooksPath .githooks

# Run before every commit. CI runs exactly this.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    @git diff --quiet -- packages/engine-api && test -z "$(git ls-files --others --exclude-standard packages/engine-api)" || { echo "packages/engine-api is stale: commit the TS bindings cargo test regenerated (ADR-023)" >&2; exit 1; }
    cargo deny check
    scripts/secret-scan.sh

# 101 BTC blocks so the bitcoind wallet has a mature coinbase.
devnet-up:
    docker compose -f devnet/docker-compose.yml up -d
    devnet/scripts/mine-zec 1
    devnet/scripts/mine-btc 101
    devnet/scripts/zec-wallet setup

# Integration tests against the running devnet (`just devnet-up` first).
devnet-test:
    ZECSPLIT_DEVNET=1 cargo test -p zecsplit-zcash --test devnet -- --nocapture --test-threads=1
    ZECSPLIT_DEVNET=1 cargo test -p zecsplit-btc --test devnet -- --nocapture --test-threads=1
    ZECSPLIT_DEVNET=1 cargo test -p zecsplit-protocol --test devnet_swap -- --nocapture --test-threads=1
    ZECSPLIT_DEVNET=1 cargo test -p zecsplit-maker --test devnet_runners -- --nocapture --test-threads=1
    ZECSPLIT_DEVNET=1 cargo test -p zecsplit-maker --test devnet_http -- --nocapture --test-threads=1

# Judge kit (`just devnet-up` first): three swaps between real engine and maker processes with
# the privacy checks, then the refund path; prints every transaction id.
demo:
    #!/usr/bin/env bash
    set -euo pipefail
    export ZECSPLIT_DEVNET=1
    echo "Happy path x3 over HTTP, then PRIV-01/02/03 (a few minutes)..."
    cargo test -q -p zecsplit-maker --test devnet_http -- --nocapture 2>&1 | grep -E "^round [0-9]+ swap|test result|panicked"
    echo "Refund path: maker locks, taker pays, nobody redeems, cancel then refund..."
    cargo test -q -p zecsplit-protocol --test devnet_swap t_024_cancel_then_refund -- --nocapture 2>&1 | grep -E "^refund path|test result|panicked"

# Judge kit: private regtest world + maker + engine, driven from the hosted UI; Ctrl-C stops it.
judge:
    devnet/judge.sh

# Pays a QR's zcash: link from the devnet wallet (run while `just judge` is up).
judge-pay uri:
    devnet/judge.sh pay '{{uri}}'

# Checks every testnet swap record against Bitcoin signet (mempool.space): proofs and the recovered share.
verify:
    cargo build -q --release -p zecsplitd --bin zecsplit-verify
    for f in evidence/testnet/records/*.json; do target/release/zecsplit-verify "$f" || exit 1; done

devnet-down:
    docker compose -f devnet/docker-compose.yml down

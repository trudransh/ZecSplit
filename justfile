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
    ZECSPLIT_DEVNET=1 cargo test -p zecsplit-zcash --test devnet_lock -- --nocapture

devnet-down:
    docker compose -f devnet/docker-compose.yml down

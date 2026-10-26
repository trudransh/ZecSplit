#!/usr/bin/env bash
# The taker engine on Zcash testnet + Bitcoin signet for the web UI (ADR-051, ADR-055).
# Needs the SSH tunnel to the maker VM (7781, 9067, 38332) and a release build of zecsplitd.
# Usage: ops/testnet/engine.sh [ui-origin=http://localhost:3000]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
P="$ROOT/private"
UI="${1:-http://localhost:3000}"
# shellcheck disable=SC1091
. "$P/testnet.env"
REFUND="$(docker run --rm -v "$P/taker-wallet:/wallet" devnet-zec-wallet:latest \
    zcash-devtool wallet -w /wallet list-addresses 2>/dev/null | grep -o 'utest1[a-z0-9]*' | head -1)"
MAKER_KEY="$(curl -s http://127.0.0.1:7781/v1/info | python3 -c 'import sys,json;print(json.load(sys.stdin)["maker_key"])')"
mkdir -p "$P/testnet-engine"
ZECSPLIT_ENGINE_NETWORK=test ZECSPLIT_ENGINE_MAKER_KEY="$MAKER_KEY" \
ZECSPLIT_ENGINE_MAKER=http://127.0.0.1:7781 ZECSPLIT_ENGINE_DATA="$P/testnet-engine" \
ZECSPLIT_ENGINE_BITCOIND=http://127.0.0.1:38332 ZECSPLIT_ENGINE_RPC_USER=zecsplit \
ZECSPLIT_ENGINE_RPC_PASS="$BTC_RPC_PASS" ZECSPLIT_ENGINE_ESPLORA=https://mempool.space/signet/api \
ZECSPLIT_ENGINE_LIGHT_SERVER=http://127.0.0.1:9067 ZECSPLIT_ENGINE_ZEC_REFUND="$REFUND" \
ZECSPLIT_ENGINE_UI="$UI" ZECSPLIT_ENGINE_ORIGINS="$UI,http://127.0.0.1:3000,http://localhost:3000" \
    exec "$ROOT/target/release/zecsplitd"

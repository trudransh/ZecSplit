#!/usr/bin/env bash
# Judge kit: a private regtest world on this machine, driven from https://zecsplit.vercel.app.
# Starts the devnet, a maker and a taker engine, and mines a block every few seconds.
# Usage: devnet/judge.sh            start everything, print the UI link, Ctrl-C stops it
#        devnet/judge.sh pay <uri>  pay the QR's zcash: URI from the devnet wallet
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
S="$ROOT/devnet/scripts"
UI="${ZECSPLIT_UI:-https://zecsplit.vercel.app}"

if [ "${1:-}" = pay ]; then
    uri="${2:?usage: judge.sh pay 'zcash:<ua>?amount=<zec>'}"
    ua="${uri#zcash:}"; ua="${ua%%\?*}"
    zec="${uri##*amount=}"; zec="${zec%%&*}"
    zats="$(python3 -c 'import sys,decimal;print(int(decimal.Decimal(sys.argv[1])*100000000))' "$zec")"
    exec "$S/zec-wallet" send "$ua" "$zats"
fi

echo "== devnet (Zebra regtest, bitcoind regtest, wallet)"
docker compose -f "$ROOT/devnet/docker-compose.yml" up -d --build
"$S/mine-zec" 1 >/dev/null
"$S/mine-btc" 101
"$S/zec-wallet" setup

echo "== release build (first run takes a few minutes)"
cargo build --release --locked -q -p zecsplit-maker -p zecsplitd

DATA="$ROOT/target/judge"
rm -rf "$DATA" && mkdir -p "$DATA/maker" "$DATA/engine"
ZEC_UA="$("$S/zec-wallet" address)"
trap 'kill $(jobs -p) 2>/dev/null || true' EXIT

ZECSPLIT_MAKER_NETWORK=regtest ZECSPLIT_MAKER_IDENTITY="$(printf '11%.0s' {1..32})" \
ZECSPLIT_MAKER_DATA="$DATA/maker" ZECSPLIT_MAKER_BITCOIND=http://127.0.0.1:18443 \
ZECSPLIT_MAKER_RPC_USER=devnet ZECSPLIT_MAKER_RPC_PASS=devnet ZECSPLIT_MAKER_WALLET=devnet \
ZECSPLIT_MAKER_LIGHT_SERVER=http://127.0.0.1:9067 ZECSPLIT_MAKER_ZEC_PAYOUT="$ZEC_UA" \
ZECSPLIT_MAKER_LISTEN=127.0.0.1:7781 ZECSPLIT_MAKER_SATS_PER_ZEC=500000 \
ZECSPLIT_MAKER_MIN_ZEC=1000000 ZECSPLIT_MAKER_MAX_ZEC=200000000 \
ZECSPLIT_MAKER_PASSPHRASE="maker passphrase" \
    "$ROOT/target/release/zecsplit-maker" > "$DATA/maker.log" 2>&1 &
until curl -sf http://127.0.0.1:7781/v1/info >/dev/null; do sleep 1; done
MAKER_KEY="$(curl -s http://127.0.0.1:7781/v1/info | python3 -c 'import sys,json;print(json.load(sys.stdin)["maker_key"])')"

ZECSPLIT_ENGINE_NETWORK=regtest ZECSPLIT_ENGINE_MAKER_KEY="$MAKER_KEY" \
ZECSPLIT_ENGINE_MAKER=http://127.0.0.1:7781 ZECSPLIT_ENGINE_DATA="$DATA/engine" \
ZECSPLIT_ENGINE_BITCOIND=http://127.0.0.1:18443 ZECSPLIT_ENGINE_RPC_USER=devnet \
ZECSPLIT_ENGINE_RPC_PASS=devnet ZECSPLIT_ENGINE_LIGHT_SERVER=http://127.0.0.1:9067 \
ZECSPLIT_ENGINE_ZEC_REFUND="$ZEC_UA" ZECSPLIT_ENGINE_UI="$UI" \
ZECSPLIT_ENGINE_ORIGINS="$UI,http://localhost:3000,http://127.0.0.1:3000" \
    "$ROOT/target/release/zecsplitd" > "$DATA/engine.log" 2>&1 &
until grep -q '^open ' "$DATA/engine.log" 2>/dev/null; do sleep 1; done

(n=0; while sleep 10; do
    "$S/mine-zec" 1 >/dev/null; n=$((n + 1))
    [ $((n % 3)) -eq 0 ] && "$S/mine-btc" 1 >/dev/null
done) &

cat <<EOF

Ready. Everything runs on this machine; the web page only talks to 127.0.0.1.
1. Open:            $(sed -n 's/^open //p' "$DATA/engine.log")
2. Passphrase:      anything (it encrypts your recovery file)
3. BTC address:     $("$S/btc-cli" getnewaddress judge bech32 | tr -d '\r')
4. Pay the QR:      just judge-pay '<the zcash: link under the QR>'  (in a second terminal)
Blocks are mined every 10 s (ZEC) and 30 s (BTC): pay within ~7 min of the QR showing. Logs: $DATA. Ctrl-C stops everything.
EOF
wait

#!/usr/bin/env bash
# Live swaps on Zcash testnet + Bitcoin signet (G5 criterion 2, ADR-051).
# The maker runs on the VM, the engine here; both reach the VM's nodes over one SSH tunnel.
# Drives the engine API exactly as the UI does and pays each QR from the taker wallet.
#
# Usage: ops/testnet/swap.sh [swaps=3] [zatoshis=5000000]
# Needs: release binaries (`cargo build --release -p zecsplit-maker -p zecsplitd`), and in
# private/: testnet.env, maker-identity.hex, maker-zec-ua.txt, taker-wallet/, taker-btc-address.txt.
set -euo pipefail

SWAPS="${1:-3}"
ZATS="${2:-5000000}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
P="$ROOT/private"
VM=(--project zecsplit --zone asia-south1-a)
WALLET_IMAGE=devnet-zec-wallet:latest
# shellcheck disable=SC1091
. "$P/testnet.env"
TAKER_ZEC="$(docker run --rm -v "$P/taker-wallet:/wallet" "$WALLET_IMAGE" \
    zcash-devtool wallet -w /wallet list-addresses 2>/dev/null | grep -o 'utest1[a-z0-9]*' | head -1)"
TAKER_BTC="$(cat "$P/taker-btc-address.txt")"
ENGINE_DATA="$P/testnet-engine"
mkdir -p "$ENGINE_DATA"

echo "== maker binary to the VM, started there"
gcloud compute scp "${VM[@]}" "$ROOT/target/release/zecsplit-maker" zecsplit-maker:~/zecsplit-maker.new
timeout 60 gcloud compute ssh zecsplit-maker "${VM[@]}" --command "if systemctl is-enabled -q zecsplit-maker@\$USER 2>/dev/null; then \
  sudo systemctl stop zecsplit-maker@\$USER; mv ~/zecsplit-maker.new ~/zecsplit-maker; \
  sudo systemctl start zecsplit-maker@\$USER; exit 0; fi; \
  pkill -x zecsplit-maker; sleep 1; \
  mv ~/zecsplit-maker.new ~/zecsplit-maker && mkdir -p ~/maker-data && \
  set -a && . ~/testnet/.env && set +a && \
  ZECSPLIT_MAKER_NETWORK=test ZECSPLIT_MAKER_IDENTITY=$(cat "$P/maker-identity.hex") \
  ZECSPLIT_MAKER_DATA=\$HOME/maker-data ZECSPLIT_MAKER_BITCOIND=http://127.0.0.1:38332 \
  ZECSPLIT_MAKER_RPC_USER=zecsplit ZECSPLIT_MAKER_RPC_PASS=\$BTC_RPC_PASS ZECSPLIT_MAKER_WALLET=maker \
  ZECSPLIT_MAKER_LIGHT_SERVER=http://127.0.0.1:9067 ZECSPLIT_MAKER_ZEC_PAYOUT=$(cat "$P/maker-zec-ua.txt") \
  ZECSPLIT_MAKER_LISTEN=127.0.0.1:7781 ZECSPLIT_MAKER_SATS_PER_ZEC=500000 \
  ZECSPLIT_MAKER_MIN_ZEC=1000000 ZECSPLIT_MAKER_MAX_ZEC=200000000 \
  ZECSPLIT_MAKER_PASSPHRASE=\$BTC_RPC_PASS setsid nohup ~/zecsplit-maker < /dev/null > ~/maker-data/maker.log 2>&1 &" || true

echo "== tunnel: maker 7781, light server 9067, bitcoind 38332"
gcloud compute ssh zecsplit-maker "${VM[@]}" -- -N -o ExitOnForwardFailure=yes \
    -L 7781:127.0.0.1:7781 -L 9067:127.0.0.1:9067 -L 38332:127.0.0.1:38332 &
TUNNEL=$!
trap 'kill $TUNNEL ${ENGINE:-} 2>/dev/null || true' EXIT
until curl -sf http://127.0.0.1:7781/v1/info >/dev/null; do sleep 2; done
MAKER_KEY="$(curl -s http://127.0.0.1:7781/v1/info | python3 -c 'import sys,json;print(json.load(sys.stdin)["maker_key"])')"
echo "maker key $MAKER_KEY"

echo "== engine on 127.0.0.1:7780"
ZECSPLIT_ENGINE_NETWORK=test ZECSPLIT_ENGINE_MAKER_KEY="$MAKER_KEY" \
ZECSPLIT_ENGINE_MAKER=http://127.0.0.1:7781 ZECSPLIT_ENGINE_DATA="$ENGINE_DATA" \
ZECSPLIT_ENGINE_BITCOIND=http://127.0.0.1:38332 ZECSPLIT_ENGINE_RPC_USER=zecsplit \
ZECSPLIT_ENGINE_RPC_PASS="$BTC_RPC_PASS" ZECSPLIT_ENGINE_ESPLORA=https://mempool.space/signet/api \
ZECSPLIT_ENGINE_LIGHT_SERVER=http://127.0.0.1:9067 ZECSPLIT_ENGINE_ZEC_REFUND="$TAKER_ZEC" \
    "$ROOT/target/release/zecsplitd" >> "$ENGINE_DATA/engine.log" 2>&1 &
ENGINE=$!
until [ -s "$ENGINE_DATA/ui-token" ] && curl -s -o /dev/null http://127.0.0.1:7780/api/health; do sleep 1; done
TOKEN="$(cat "$ENGINE_DATA/ui-token")"
api() { curl -sf -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' "$@"; }
field() { python3 -c "import sys,json;v=json.load(sys.stdin);print(eval(sys.argv[1]) or '')" "$1"; }
api -X POST http://127.0.0.1:7780/api/unlock -d "{\"passphrase\":\"$BTC_RPC_PASS\"}" >/dev/null

for n in $(seq 1 "$SWAPS"); do
    echo "== swap $n: $ZATS zatoshis"
    QUOTE="$(api -X POST http://127.0.0.1:7780/api/quote -d "{\"zec_amount\":$ZATS}")"
    QID="$(echo "$QUOTE" | field 'v["quote_id"]')"
    echo "quote $(echo "$QUOTE" | field 'v["btc_amount"]') sats"
    api -X POST http://127.0.0.1:7780/api/swaps \
        -d "{\"quote_id\":\"$QID\",\"btc_address\":\"$TAKER_BTC\",\"accept_risks\":true}" >/dev/null
    PAID=""
    while :; do
        VIEW="$(api "http://127.0.0.1:7780/api/swaps/$QID")"
        OUT="$(echo "$VIEW" | field 'v.get("outcome")')"
        [ -n "$OUT" ] && { echo "outcome $OUT"; break; }
        if [ "$(echo "$VIEW" | field 'v["recovery_saved"]')" != "True" ] &&
            api -o "$ENGINE_DATA/$QID.recovery" "http://127.0.0.1:7780/api/swaps/$QID/recovery-file"; then
            api -X POST "http://127.0.0.1:7780/api/swaps/$QID/recovery-saved" >/dev/null
        fi
        URI="$(echo "$VIEW" | field 'v.get("zip321_uri")')"
        if [ -n "$URI" ] && [ -z "$PAID" ]; then
            UA="${URI#zcash:}"; UA="${UA%%\?*}"
            echo "paying $ZATS to the lock address"
            docker run --rm -v "$P/taker-wallet:/wallet" "$WALLET_IMAGE" sh -c \
                "zcash-devtool wallet -w /wallet sync -s zecrocks >/dev/null 2>&1; \
                 zcash-devtool wallet -w /wallet send -i /wallet/identity.txt --address $UA --value $ZATS -s zecrocks 2>&1 | tail -1"
            PAID=1
        fi
        echo "  $(echo "$VIEW" | field '" ".join(s["kind"]+":"+s["status"] for s in v["steps"])')"
        sleep 20
    done
    sqlite3 "$ENGINE_DATA/engine.sqlite" "select name, txid from txs where hex(swap_id)=upper('$QID')" 2>/dev/null ||
        echo "$VIEW" | field '[s.get("txid") for s in v["steps"] if s.get("txid")]'
done

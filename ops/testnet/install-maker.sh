#!/usr/bin/env bash
# Turns the testnet maker on the VM into a systemd service (`zecsplit-maker@<user>`).
# Run locally after `ops/testnet/swap.sh` has put the binary on the VM.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
P="$ROOT/private"
VM=(--project zecsplit --zone asia-south1-a)
gcloud compute scp "${VM[@]}" "$ROOT/ops/testnet/zecsplit-maker.service" zecsplit-maker:~/zecsplit-maker@.service
timeout 120 gcloud compute ssh zecsplit-maker "${VM[@]}" --command "set -e; . ~/testnet/.env; umask 077
cat > ~/maker.env <<CONF
ZECSPLIT_MAKER_NETWORK=test
ZECSPLIT_MAKER_IDENTITY=$(cat "$P/maker-identity.hex")
ZECSPLIT_MAKER_DATA=\$HOME/maker-data
ZECSPLIT_MAKER_BITCOIND=http://127.0.0.1:38332
ZECSPLIT_MAKER_RPC_USER=zecsplit
ZECSPLIT_MAKER_RPC_PASS=\$BTC_RPC_PASS
ZECSPLIT_MAKER_WALLET=maker
ZECSPLIT_MAKER_LIGHT_SERVER=http://127.0.0.1:9067
ZECSPLIT_MAKER_ZEC_PAYOUT=$(cat "$P/maker-zec-ua.txt")
ZECSPLIT_MAKER_LISTEN=127.0.0.1:7781
ZECSPLIT_MAKER_SATS_PER_ZEC=500000
ZECSPLIT_MAKER_MIN_ZEC=1000000
ZECSPLIT_MAKER_MAX_ZEC=200000000
ZECSPLIT_MAKER_PASSPHRASE=\$BTC_RPC_PASS
CONF
pkill -x zecsplit-maker || true
sudo mv ~/zecsplit-maker@.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now zecsplit-maker@\$USER
sleep 3; systemctl is-active zecsplit-maker@\$USER; curl -s 127.0.0.1:7781/v1/info" < /dev/null || true

#!/usr/bin/env bash
# T-001 baseline for PRIV-03 / ADV-13: any 64-hex-char run in a log counts as a leaked secret.
# ponytail: txids are 64 hex too; once logs carry txids, switch to grepping for the actual
# secret values the scenario suite produced (PRIV-03 proper).
set -euo pipefail
cd "$(dirname "$0")/.."

hits=$(find . \( -path ./target -o -path ./private -o -path ./.git -o -name node_modules \) -prune -o \
    -type f \( -path '*/logs/*' -o -name '*.log' \) -print0 |
    xargs -0 -r grep -nHE '\b[0-9a-fA-F]{64}\b' || true)

if [ -n "$hits" ]; then
    echo "secret-scan: 64-hex strings found in logs:" >&2
    echo "$hits" >&2
    exit 1
fi

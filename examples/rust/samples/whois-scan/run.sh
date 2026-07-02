#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/target/release/whois-scan"

if [[ ! -x "$BIN" ]]; then
  echo "Building $BIN ..."
  (cd "$ROOT" && cargo build --release)
fi

exec "$BIN" \
  --interface "${BACNET_BIND_ADDRESS:-192.168.204.55}" \
  --broadcast "${BACNET_BROADCAST:-192.168.204.255}" \
  --timeout "${BACNET_SCAN_TIMEOUT:-3}" \
  "$@"

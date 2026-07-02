#!/usr/bin/env bash
# RPM demo on device 5007 — OA-T, STAT ZN-T, DUCT-T in one request.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/target/release/rpm-read"

if [[ ! -x "$BIN" ]]; then
  echo "Building $BIN ..."
  (cd "$ROOT" && cargo build --release)
fi

exec "$BIN" \
  --device "${BACNET_DEVICE_INSTANCE:-5007}" \
  --interface "${BACNET_BIND_ADDRESS:-192.168.204.55}" \
  --broadcast "${BACNET_BROADCAST:-192.168.204.255}" \
  --points "${BACNET_RPM_POINTS:-analog-input:1173,analog-input:10014,analog-input:1192}" \
  "$@"

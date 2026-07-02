#!/usr/bin/env bash
# WriteProperty demo on device 5007 (C06-0-10VDC-O), verify, relinquish.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/target/release/bacnet-write"

if [[ ! -x "$BIN" ]]; then
  echo "Building $BIN ..."
  (cd "$ROOT" && cargo build --release)
fi

exec "$BIN" \
  --device "${BACNET_DEVICE_INSTANCE:-5007}" \
  --interface "${BACNET_BIND_ADDRESS:-192.168.204.55}" \
  --broadcast "${BACNET_BROADCAST:-192.168.204.255}" \
  --point "${BACNET_WRITE_POINT:-analog-output:10035}" \
  --value "${BACNET_WRITE_VALUE:-5}" \
  --priority "${BACNET_WRITE_PRIORITY:-8}" \
  "$@"

#!/usr/bin/env bash
# Discover device 5007 via Who-Is, enumerate points, exit.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/target/release/point-discover"

if [[ ! -x "$BIN" ]]; then
  echo "Building $BIN ..."
  (cd "$ROOT" && cargo build --release)
fi

ARGS=(
  --device "${BACNET_DEVICE_INSTANCE:-5007}"
  --interface "${BACNET_BIND_ADDRESS:-192.168.204.55}"
  --broadcast "${BACNET_BROADCAST:-192.168.204.255}"
)

if [[ -n "${BACNET_DEVICE_ADDRESS:-}" ]]; then
  ARGS+=(--address "$BACNET_DEVICE_ADDRESS")
fi

exec "$BIN" "${ARGS[@]}" "$@"

#!/usr/bin/env bash
# Foreground mini-device with discovery-friendly settings (run from any directory).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/mini-device-revisited/target/release/mini-device-revisited"

if [[ ! -x "$BIN" ]]; then
  echo "Building $BIN ..."
  (cd "$ROOT/mini-device-revisited" && cargo build --release)
fi

export RUST_LOG="${RUST_LOG:-debug,mini_device_revisited=debug,bacnet_server=debug,bacnet_transport=debug,bacnet_network=debug}"

exec "$BIN" \
  --name "${BACNET_DEVICE_NAME:-BensServerTest}" \
  --instance "${BACNET_DEVICE_INSTANCE:-3456}" \
  --address "${BACNET_BIND_ADDRESS:-192.168.204.55}" \
  --broadcast "${BACNET_BROADCAST:-192.168.204.255}" \
  --debug \
  "$@"

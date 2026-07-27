#!/usr/bin/env bash
#
# Check the published crates at the workspace MSRV.
#
# The MSRV is a promise to people who depend on this workspace from crates.io,
# so the gate covers exactly the crates they can depend on. The list is derived
# from `cargo metadata` rather than written here: a hardcoded list drifts in the
# dangerous direction silently, because a newly published member that nobody
# remembers to add is simply never checked.
#
# Run with RUSTUP_TOOLCHAIN set to the MSRV, which overrides rust-toolchain.toml:
#   RUSTUP_TOOLCHAIN=1.93 bash .github/scripts/check-msrv.sh
set -euo pipefail

cd "$(dirname "$0")/../.."

PKGS=$(
  cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json,sys
for p in json.load(sys.stdin)["packages"]:
    if p.get("publish") != []:
        print(p["name"])' |
    sort
)

if [ -z "$PKGS" ]; then
  echo "error: no publishable crates found in cargo metadata" >&2
  exit 1
fi

ARGS=""
for p in $PKGS; do
  ARGS="$ARGS -p $p"
done

# Optional features a consumer can enable. Without them the gate sees only
# default features, which leaves the BACnet/SC and IPv6 module trees — and the
# MSRVs of rustls, tokio-rustls and tokio-tungstenite — unchecked.
#
# `serial`, `serial-gpio`, `ethernet` and `pcap` are deliberately absent: they
# need Linux system packages this job does not install. Extending the gate to
# them is tracked as #196.
FEATURES="bacnet-transport/sc-tls,bacnet-transport/ipv6"
FEATURES="$FEATURES,bacnet-client/sc-tls,bacnet-client/ipv6"
FEATURES="$FEATURES,bacnet-server/sc-tls"
FEATURES="$FEATURES,bacnet-cli/sc-tls"

echo "MSRV gate covers:"
for p in $PKGS; do echo "  - $p"; done
echo "with features: $FEATURES"
echo

# shellcheck disable=SC2086
exec cargo check --locked $ARGS --features "$FEATURES"

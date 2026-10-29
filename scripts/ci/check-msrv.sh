#!/usr/bin/env bash
#
# Check metadata-eligible crates at the workspace MSRV.
#
# The MSRV is a promise to people who depend on this workspace from crates.io,
# so the gate covers members whose metadata permits publication. This is not
# proof that every eligible member has been published (see #195). The list is derived
# from `cargo metadata` rather than written here: a hardcoded list drifts in the
# dangerous direction silently, because a newly published member that nobody
# remembers to add is simply never checked.
#
# Run with RUSTUP_TOOLCHAIN set to the MSRV, which overrides rust-toolchain.toml:
#   RUSTUP_TOOLCHAIN=1.95 bash scripts/ci/check-msrv.sh
# Add --linux-native for the required local Linux GNU optional-feature/link gate.
set -euo pipefail

cd "$(dirname "$0")/../.."

fail() { echo "error: MSRV: $*" >&2; exit 1; }
native=false
case "$#:${1:-}" in
  0:) ;;
  1:--linux-native) native=true ;;
  *) fail 'usage: check-msrv.sh [--linux-native]' ;;
esac

cargo_bin=cargo
target_args=()
if "$native"; then
  [ "$(uname -s)" = Linux ] || fail '--linux-native requires a native Linux GNU environment'
  for tool in rustup python3 cc pkg-config cmake perl file ldd; do
    command -v "$tool" >/dev/null || fail "missing prerequisite: $tool"
  done
  # Resolve only an installed toolchain; never install or update one here.
  toolchain=${RUSTUP_TOOLCHAIN:-1.95}
  installed=false
  while read -r name _; do
    case "$name" in "$toolchain"|"$toolchain"-*) installed=true ;; esac
  done <<< "$(rustup toolchain list)"
  "$installed" || fail "toolchain $toolchain is not installed; provide Rust/Cargo 1.95"
  compiler=$(rustup which --toolchain "$toolchain" rustc) || fail 'cannot resolve installed rustc'
  cargo_bin=$(rustup which --toolchain "$toolchain" cargo) || fail 'cannot resolve installed cargo'
  compiler_info=$("$compiler" -vV)
  cargo_version=$("$cargo_bin" --version)
  grep -Eq '^release: 1[.]95[.][0-9]+$' <<< "$compiler_info" || fail 'rustc must be stable 1.95.x'
  grep -Eq '^cargo 1[.]95[.][0-9]+ ' <<< "$cargo_version" || fail 'cargo must be stable 1.95.x'
  host=$(sed -n 's/^host: //p' <<< "$compiler_info")
  [ "$host" = "$(uname -m)-unknown-linux-gnu" ] || fail "unsupported or non-native compiler host: $host"
  for key in RUSTC CARGO_BUILD_RUSTC; do
    [ -z "${!key:-}" ] || [ "${!key}" = "$compiler" ] || fail "conflicting $key; use the resolved 1.95 compiler"
  done
  for key in RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER CARGO_BUILD_RUSTC_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER; do
    [ -z "${!key:-}" ] || fail "compiler wrapper $key is not allowed in --linux-native"
  done
  [ -z "${CARGO_BUILD_TARGET:-}" ] || [ "$CARGO_BUILD_TARGET" = "$host" ] || fail 'CARGO_BUILD_TARGET conflicts with native host'
  # Environment beats build.rustc/configured wrappers; explicit --target beats
  # build.target. Pin the same compiler for both target and build-script work.
  export RUSTC="$compiler" CARGO_BUILD_RUSTC="$compiler"
  export RUSTC_WRAPPER='' RUSTC_WORKSPACE_WRAPPER=''
  export CARGO_BUILD_RUSTC_WRAPPER='' CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER=''
  export LC_ALL=C
  target_args=(--target "$host")
  pkg-config --exists libpcap || fail 'libpcap development package is required (Debian: libpcap-dev)'
  echo "$compiler_info"
  echo "$cargo_version"
  echo "Native target: $host; compiler: $compiler"
  pkg-config --modversion libpcap
fi

PKGS=$(
  "$cargo_bin" metadata --locked --no-deps --format-version 1 |
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

ARGS=()
for p in $PKGS; do
  ARGS+=(-p "$p")
done

# Optional features a consumer can enable. Without them the gate sees only
# default features, which leaves the BACnet/SC and IPv6 module trees — and the
# MSRVs of rustls, tokio-rustls and tokio-tungstenite — unchecked.
#
# Linux native features need extra local prerequisites. Keep them separate from
# this baseline, which is also used by the retained GitHub invocation.
FEATURES="bacnet-transport/sc-tls,bacnet-transport/ipv6"
FEATURES="$FEATURES,bacnet-client/sc-tls,bacnet-client/ipv6"
FEATURES="$FEATURES,bacnet-server/sc-tls"
FEATURES="$FEATURES,bacnet-cli/sc-tls,bacnet-cli/tui"
FEATURES="$FEATURES,bacnet-mstp-codec/bytes,bacnet-mstp-codec/heapless"
FEATURES="$FEATURES,bacnet-mstp-core/alloc"
FEATURES="$FEATURES,bacnet-bip-codec/bytes,bacnet-bip-codec/heapless"
FEATURES="$FEATURES,bacnet-router-core/bip,bacnet-router-core/mstp"

# The no_std builds (std off, alloc on) of the crates that have one (#1613).
# In the run above, the hosted crates turn std back on for them, so these get
# a run of their own.
NO_STD_ARGS=(-p bacnet-types -p bacnet-encoding -p bacnet-services -p bacnet-objects --no-default-features --features alloc)

echo "MSRV gate covers:"
for p in $PKGS; do echo "  - $p"; done
echo "with features: $FEATURES"
echo "and bacnet-types, bacnet-encoding, bacnet-services and bacnet-objects without std, with alloc"
echo

run() { printf '+ ' >&2; printf '%q ' "$@" >&2; printf '\n' >&2; "$@"; }
if "$native"; then
  run "$cargo_bin" check --locked "${ARGS[@]}" --features "$FEATURES" "${target_args[@]}"
  run "$cargo_bin" check --locked "${NO_STD_ARGS[@]}" "${target_args[@]}"
else
  # Bash 3.2 with nounset cannot expand an empty array here.
  run "$cargo_bin" check --locked "${ARGS[@]}" --features "$FEATURES"
  run "$cargo_bin" check --locked "${NO_STD_ARGS[@]}"
  exit 0
fi

for feature in ethernet serial serial-gpio; do
  run "$cargo_bin" check --locked -p bacnet-transport --features "$feature" "${target_args[@]}"
done

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
# Render diagnostics on stderr while keeping artifact messages for exact output
# selection. Do not assume target/debug or ignore a failed Cargo exit.
run "$cargo_bin" build --locked -p bacnet-cli --bin bacnet --features pcap \
  "${target_args[@]}" --message-format=json-render-diagnostics > "$scratch/build.json"
executable=$(python3 - "$PWD/crates/bacnet-cli/Cargo.toml" "$scratch/build.json" <<'PY'
import json, os, sys
manifest, output = sys.argv[1:]
artifacts, finished = [], []
with open(output) as stream:
    for line in stream:
        message = json.loads(line)
        if message.get('reason') == 'build-finished':
            finished.append(message.get('success'))
        if (message.get('reason') == 'compiler-artifact'
                and os.path.realpath(message['manifest_path']) == os.path.realpath(manifest)
                and 'bin' in message['target']['kind']
                and message['target']['name'] == 'bacnet'
                and message.get('executable')):
            artifacts.append(message['executable'])
if finished != [True] or len(artifacts) != 1:
    sys.exit('error: MSRV: expected one successful bacnet-cli executable artifact')
if not os.path.isfile(artifacts[0]) or not os.access(artifacts[0], os.X_OK):
    sys.exit('error: MSRV: Cargo executable artifact is missing or not executable')
print(artifacts[0])
PY
)
echo "Cargo executable: $executable"
description=$(file -b "$executable")
echo "$description"
[[ "$description" == ELF*executable* ]] || fail 'Cargo executable is not a native ELF executable'
ldd "$executable" > "$scratch/ldd.txt"
cat "$scratch/ldd.txt"
python3 - "$scratch/ldd.txt" <<'PY'
import re, sys
with open(sys.argv[1]) as stream:
    links = stream.read()
if 'not found' in links:
    sys.exit('error: MSRV: unresolved dynamic library (not found)')
if not re.search(r'^\s*libpcap\.so\S*\s+=>\s+/\S+\s+\(', links, re.MULTILINE):
    sys.exit('error: MSRV: no resolved dynamic libpcap.so dependency')
PY
echo 'OK: Linux native MSRV baseline, no_std alloc, ethernet, serial, serial-gpio and pcap build/link passed.'

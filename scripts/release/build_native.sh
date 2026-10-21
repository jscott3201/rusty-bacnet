#!/usr/bin/env bash
# Build the macOS or Windows release artifacts on a runner of that platform
# and architecture (#1472):
#
#   scripts/release/build_native.sh <target> <cli asset name> <python>...
#
# With the runner's own C toolchain and linker (Xcode's on macOS, MSVC on
# Windows) and the Rust toolchain that rust-toolchain.toml pins, which the job
# installs first, it builds:
# 1. a wheel for each <python> (an interpreter's path) into dist/, with the
#    maturin wheel that maturin-requirements.txt pins by version and sha256,
#    installed into a venv of the first one;
# 2. the CLI with sc-tls into out/<cli asset name>.
# Both use the `dist` profile (fat LTO, #973; Cargo.toml).
#
# macOS: MACOSX_DEPLOYMENT_TARGET must be set. It is the binaries' minimum
# macOS, for the C code too, and maturin takes the wheel tag from it.
# Windows: both link with -DEBUG:NONE (no PDB, whose path and build ID change
# with every build) and -Brepro (a hash of the output where the timestamps
# go), and the CLI links the C runtime statically, so it needs no Visual C++
# Redistributable. The wheels link it dynamically, like Python itself.
# SOURCE_DATE_EPOCH, when set, pins the wheels' timestamps. Bash on Windows is
# Git Bash.
set -euo pipefail

fail() { echo "error: build_native.sh: $*" >&2; exit 1; }
[ $# -ge 3 ] || fail "usage: build_native.sh <target> <cli asset name> <python>..."
target=$1 cli=$2
shift 2
root=$(cd "$(dirname "$0")/../.." && pwd)

# D:/a/... paths on Windows, which both bash and native programs accept.
native_path() { if command -v cygpath >/dev/null; then cygpath -m "$1"; else printf '%s\n' "$1"; fi; }

exe='' wheel_flags='' cli_flags=''
case $target in
  *-apple-darwin)
    [[ ${MACOSX_DEPLOYMENT_TARGET:-} =~ ^[0-9]+\.[0-9]+$ ]] \
      || fail "set MACOSX_DEPLOYMENT_TARGET (major.minor) to build $target"
    echo "minimum macOS $MACOSX_DEPLOYMENT_TARGET; $(xcrun --show-sdk-path) ($(xcrun --show-sdk-version))"
    ;;
  *-pc-windows-msvc)
    exe=.exe
    # link.exe takes options with - as well as /. The - form keeps Git Bash
    # from taking /DEBUG:NONE for a path list and converting it.
    wheel_flags="-C link-arg=-DEBUG:NONE -C link-arg=-Brepro"
    cli_flags="-C target-feature=+crt-static $wheel_flags"
    ;;
  *) fail "build_native.sh builds macOS and Windows targets, not $target" ;;
esac
cd "$root"
host=$(rustc -vV | sed -n 's/^host: //p')
[ "$host" = "$target" ] || fail "build $target on a $target runner, not $host"

interpreters=()
for python in "$@"; do interpreters+=("$(native_path "$python")"); done
tmp=$(native_path "${RUNNER_TEMP:-$(mktemp -d)}")
venv=$tmp/maturin-venv
"${interpreters[0]}" -m venv "$venv"
bin=$venv/bin
if [ -d "$venv/Scripts" ]; then bin=$venv/Scripts; fi
"$bin/python" -m pip install -q --disable-pip-version-check --require-hashes --only-binary :all: \
  -r scripts/release/maturin-requirements.txt
"$bin/maturin" --version

echo "::group::Wheels"
RUSTFLAGS=$wheel_flags "$bin/maturin" build --profile dist --locked --target "$target" -i "${interpreters[@]}" \
  -m crates/rusty-bacnet/Cargo.toml --out dist
echo "::endgroup::"

# No `serial` (MS/TP) here yet: it ships in the Linux CLI only until a USB
# RS-485 adapter has been tested on macOS and on Windows (#969). On macOS it
# would link IOKit and CoreFoundation for the port listing, as the wheels do.
# The default features add the terminal UI (`tui`, #974).
echo "::group::CLI"
RUSTFLAGS=$cli_flags cargo build --profile dist --locked -p bacnet-cli --features sc-tls --target "$target"
mkdir -p out
cp "target/$target/dist/bacnet$exe" "out/$cli"
echo "::endgroup::"
ls -l dist out

#!/usr/bin/env bash
# Installs the CI tools named on the command line, at the versions pinned in
# .github/ci-pins.env, from their prebuilt Linux x86_64 release archives. Each
# download must match the pinned SHA-256 before anything is unpacked. The
# binaries go to $CARGO_HOME/bin (default ~/.cargo/bin), which is on the
# runner's PATH. ci.yml's and fuzz.yml's jobs run it on GitHub's Ubuntu
# runners:
#
#   bash scripts/ci/install-tools.sh cargo-nextest maturin
#
# Tools: cargo-nextest, cargo-deny, cargo-audit, maturin, cargo-fuzz.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
pins=$root/.github/ci-pins.env

fail() { echo "error: install-tools: $*" >&2; exit 1; }
[ "$#" -gt 0 ] || fail 'usage: install-tools.sh <tool>...'
[ "$(uname -sm)" = "Linux x86_64" ] || fail "only for Linux x86_64, not $(uname -sm)"

# pin NAME: the value of NAME= in the pins file, which must be there once.
pin() {
  local value
  value=$(sed -n "s/^$1=//p" "$pins")
  [[ $value =~ ^[0-9A-Za-z.]+$ ]] || fail "no single $1 in $pins"
  printf '%s\n' "$value"
}

bin=${CARGO_HOME:-$HOME/.cargo}/bin
mkdir -p "$bin"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# fetch URL SHA256: download to $tmp/archive and check it.
fetch() {
  curl -sSfL --retry 3 -o "$tmp/archive" "$1"
  echo "$2  $tmp/archive" | sha256sum -c --quiet - || fail "$1 does not match its pinned SHA-256"
}

for tool in "$@"; do
  case $tool in
    cargo-nextest)
      version=$(pin NEXTEST_VERSION)
      fetch "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$version/cargo-nextest-$version-x86_64-unknown-linux-gnu.tar.gz" \
        "$(pin NEXTEST_SHA256)"
      tar xzf "$tmp/archive" -C "$bin" cargo-nextest
      cargo nextest --version | sed -n 1p ;;
    cargo-deny)
      version=$(pin CARGO_DENY_VERSION)
      dir=cargo-deny-$version-x86_64-unknown-linux-musl
      fetch "https://github.com/EmbarkStudios/cargo-deny/releases/download/$version/$dir.tar.gz" \
        "$(pin CARGO_DENY_SHA256)"
      tar xzf "$tmp/archive" -C "$bin" --strip-components=1 "$dir/cargo-deny"
      cargo deny --version ;;
    cargo-audit)
      version=$(pin CARGO_AUDIT_VERSION)
      dir=cargo-audit-x86_64-unknown-linux-musl-v$version
      fetch "https://github.com/rustsec/rustsec/releases/download/cargo-audit%2Fv$version/$dir.tgz" \
        "$(pin CARGO_AUDIT_SHA256)"
      tar xzf "$tmp/archive" -C "$bin" --strip-components=1 "$dir/cargo-audit"
      cargo audit --version ;;
    maturin)
      version=$(pin MATURIN_VERSION)
      fetch "https://github.com/PyO3/maturin/releases/download/v$version/maturin-x86_64-unknown-linux-musl.tar.gz" \
        "$(pin MATURIN_SHA256)"
      tar xzf "$tmp/archive" -C "$bin" maturin
      maturin --version ;;
    cargo-fuzz)
      version=$(pin CARGO_FUZZ_VERSION)
      fetch "https://github.com/rust-fuzz/cargo-fuzz/releases/download/$version/cargo-fuzz-$version-x86_64-unknown-linux-musl.tar.gz" \
        "$(pin CARGO_FUZZ_SHA256)"
      tar xzf "$tmp/archive" -C "$bin" cargo-fuzz
      cargo fuzz --version ;;
    *) fail "unknown tool $tool" ;;
  esac
done

#!/usr/bin/env bash
# Clippy and rustdoc for each publishable crate on its own, with default
# features and only its lib/bin targets, denying warnings. Also covers the
# feature-off builds of bacnet-types, bacnet-encoding and bacnet-services
# (no_std with alloc) and of bacnet-cli (no TUI), and the rustdoc of the
# MS/TP and B/IP crates with each storage feature alone. Every rustdoc run documents
# private items, so a broken intra-doc link on an item that exists only in one
# feature configuration fails here (#1192).
#
#   check-default-features.sh               for the host
#   check-default-features.sh <triple>...   for those targets; cargo checks
#                                           them side by side, in one run
#
# The workspace runs cannot catch code or docs that only work with a feature
# on: cargo unifies features across the selected members, and --all-targets
# pulls in dev-dependencies that turn optional features (such as sc-tls) back
# on. A published crate built alone with defaults hit exactly that (#908), and
# docs.rs builds with default features too. Crates come from `cargo metadata`,
# so a new publishable crate is covered without editing this list.
#
# Linux CI runs it for Linux, Windows and macOS (#981), whose platform cfgs
# compile different code. Neither clippy nor rustdoc links, and no C code
# builds with default features, so another target needs only its standard
# library (`rustup target add`), not cargo-xwin or zig. A default-feature
# dependency with a C build script would change that.
set -euo pipefail

targets=()
for triple in "$@"; do
  case $triple in -*|'') echo "usage: $0 [<target triple>...]" >&2; exit 2 ;; esac
  targets+=(--target "$triple")
done
label=${*:+; $*}
# cargo <subcommand> with the --target options. macOS's bash 3.2 treats an
# empty array as unset under `set -u`, hence the ${...+...}.
cargo_for() { local sub=$1; shift; cargo "$sub" ${targets[@]+"${targets[@]}"} "$@"; }

crates=$(cargo metadata --no-deps --format-version 1 --locked | python3 -c '
import json, sys
for p in json.load(sys.stdin)["packages"]:
    if p.get("publish") != []:
        print(p["name"])
')
[ -n "$crates" ] || { echo "error: cargo metadata listed no publishable crates" >&2; exit 1; }

export RUSTDOCFLAGS="-D warnings"
for crate in $crates; do
  echo "==> $crate (default features$label)"
  cargo_for clippy -p "$crate" --locked -- -D warnings
  cargo_for doc -p "$crate" --no-deps --locked --document-private-items
done

# The no_std builds (std off, alloc on) of the crates that have one. Without
# either feature they refuse to build (#1604).
for crate in bacnet-types bacnet-encoding bacnet-services; do
  echo "==> $crate (no_std with alloc$label)"
  cargo_for clippy -p "$crate" --no-default-features --features alloc --locked -- -D warnings
  cargo_for doc -p "$crate" --no-default-features --features alloc --no-deps --locked --document-private-items
done

# The rustdoc of the MS/TP and B/IP crates with each storage feature alone,
# and of the router core with each glue feature alone. Their defaults have
# none, and the workspace runs turn on all of them, so an intra-doc link to
# an item behind the other feature breaks only here (#1681, #1679).
for spec in "bacnet-mstp-core alloc" \
  "bacnet-mstp-codec alloc" "bacnet-mstp-codec heapless" \
  "bacnet-bip-codec alloc" "bacnet-bip-codec heapless" \
  "bacnet-router-core bip" "bacnet-router-core mstp"; do
  read -r crate features <<<"$spec"
  echo "==> $crate (rustdoc, features: $features$label)"
  cargo_for doc -p "$crate" --no-default-features --features "$features" --no-deps --locked --document-private-items
done

echo "==> bacnet-cli (rustdoc, no default features, without the TUI$label)"
cargo_for doc -p bacnet-cli --no-default-features --no-deps --locked --document-private-items

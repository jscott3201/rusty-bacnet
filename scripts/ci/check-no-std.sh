#!/usr/bin/env bash
# Clippy for the no_std crates on bare-metal targets, denying warnings, once
# per feature set that changes what they compile, and a check of each build's
# dependency graph (#1604, #1605, #1613).
#
#   check-no-std.sh               for the default targets below
#   check-no-std.sh <triple>...   for those targets; cargo checks them side by
#                                 side, in one run per crate and feature set
#
# A host build can't prove a crate is no_std: the host always has std and an
# allocator, so a stray `std::` path or a dependency that turns on its std
# feature still compiles. A bare-metal target has no std, and no allocator
# unless a crate asks for alloc, so it fails there. The default targets are
# ARMv6-M (no atomic compare-and-swap, so no alloc::sync), ARMv7E-M with an
# FPU, and RISC-V with atomics. Only lib targets build: tests and their
# dev-dependencies need std. Each target needs only `rustup target add`.
#
# On ARMv6-M, bytes takes compare-and-swap from portable-atomic, whose
# backend only the final firmware may choose. So a build there with bytes in
# its graph brings in `harness` below (firmware/nostd-harness), which plays
# the firmware: on that target alone it turns on bytes' extra-platforms
# feature and portable-atomic's critical-section backend. Cargo unifies
# features across all the targets of one run, so that build runs on its own,
# and bytes on the other targets builds as their firmware would build it.
# Clippy doesn't link, so nothing implements the critical section. The
# allocation-free builds of bacnet-types, bacnet-encoding and
# bacnet-services (`-` below, #1614) have no bytes in their graph, so they
# need no harness on any target; check_alloc_free fails one that has bytes
# or smallvec.
#
# The graph checks read `cargo tree` for each crate, feature set and target:
# the crate's own graph, and on ARMv6-M the graph with the harness too. No
# package in either may have a `std` feature on, and none may be one of
# `host_only` below. That names the package when std leaks in (through a
# workspace entry with default features, say), and it catches a leak that
# still compiles, such as tracing on a target with atomics. The crate's own
# graph, on every target, may not choose atomics for the firmware either: no
# portable-atomic or critical-section in it, and no bytes with
# extra-platforms. The harness would hide such a choice on ARMv6-M.
#
# A crate in `cas_only` needs atomic compare-and-swap (alloc::sync::Arc), so
# it is checked only on the targets that have it: ARMv6-M is passed over
# (#1615).
#
# To cover another crate or feature set, add a line to `checks`.
set -euo pipefail

# One line each: the crate, then its features as a comma-separated list, or
# `-` for none. Every line builds with --no-default-features.
checks=(
  "bacnet-time -"
  "bacnet-npdu -"
  "bacnet-mstp-codec -"
  "bacnet-mstp-codec alloc"
  "bacnet-mstp-codec heapless"
  "bacnet-mstp-codec bytes,heapless"
  "bacnet-mstp-codec crc32k-table"
  "bacnet-mstp-core -"
  "bacnet-mstp-core alloc"
  "bacnet-bip-codec -"
  "bacnet-bip-codec alloc"
  "bacnet-bip-codec heapless"
  "bacnet-bip-codec bytes,heapless"
  "bacnet-bip-core -"
  "bacnet-router-core -"
  "bacnet-router-core bip"
  "bacnet-router-core mstp"
  "bacnet-router-core bip,mstp"
  "bacnet-types -"
  "bacnet-types alloc"
  "bacnet-types serde"
  "bacnet-encoding -"
  "bacnet-encoding heapless"
  "bacnet-encoding alloc"
  "bacnet-encoding alloc,heapless"
  "bacnet-services -"
  "bacnet-services alloc"
  "bacnet-objects alloc"
)
default_targets=(thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imac-unknown-none-elf)
# Crates that need std (or, for tracing, atomics the no_std crates don't
# assume). None may appear in a no_std build's graph.
host_only=(tracing tracing-core tokio socket2)
# The firmware stand-in for ARMv6-M builds that use bytes; see above.
harness=nostd-harness
# The crates that need compare-and-swap, and the targets that lack it.
cas_only=(bacnet-objects)
no_cas_target_re='^thumbv6m-'
# The crates whose builds without `alloc` (or `std` or `serde`, which turn it
# on) are their allocation-free profile (#1614), and the packages that
# allocate, which none of those builds' graphs may hold.
alloc_free=(bacnet-types bacnet-encoding bacnet-services)
allocating=(bytes smallvec)

# One line per package of the graph for <cargo tree args...>: `name vX.Y.Z
# [feature,...]`. Proc macros run on the host, so they're left out.
graph() {
  cargo tree --locked "$@" -e normal,no-proc-macro --prefix none --format '{p} [{f}]'
}

# Fails if <tree>, a graph of <crate> with <features> on <target>, turns on a
# std feature or holds a host-only crate. <label> names anything else in the
# graph (the harness), for the message.
check_std() {
  local crate=$1 features=$2 triple=$3 label=$4 tree=$5 bad host_re
  host_re=$(IFS='|'; echo "${host_only[*]}")
  bad=$(grep -E "\[([^]]*,)?std(,[^]]*)?\]|^($host_re) v" <<<"$tree" || true)
  if [ -n "$bad" ]; then
    echo "error: the $crate graph (features: $features; $triple$label) has std in it:" >&2
    sort -u <<<"$bad" >&2
    return 1
  fi
}

# Fails if <tree>, the graph of <crate> alone with <features> on <target>,
# chooses atomics that only firmware may choose: portable-atomic or
# critical-section in it, or bytes with extra-platforms.
check_backend() {
  local crate=$1 features=$2 triple=$3 tree=$4 bad
  bad=$(grep -E "^(portable-atomic|critical-section) v|^bytes v.*\[([^]]*,)?extra-platforms(,[^]]*)?\]$" <<<"$tree" || true)
  if [ -n "$bad" ]; then
    echo "error: $crate (features: $features; $triple) chooses its atomics: only the firmware may" \
      "bring in portable-atomic or critical-section, or turn on bytes' extra-platforms feature:" >&2
    sort -u <<<"$bad" >&2
    return 1
  fi
}

# Fails if <tree>, the graph of <crate> with <features> on <target>, is an
# allocation-free profile's (a crate in `alloc_free` without `alloc`, `std`
# or `serde`, as `-` and `heapless` are) and holds a package that allocates.
# Clippy doesn't link, so a dependency that allocates would otherwise
# compile there unseen.
check_alloc_free() {
  local crate=$1 features=$2 triple=$3 tree=$4 bad alloc_re
  case ",$features," in *,alloc,* | *,std,* | *,serde,*) return 0 ;; esac
  case " ${alloc_free[*]} " in *" $crate "*) ;; *) return 0 ;; esac
  alloc_re=$(IFS='|'; echo "${allocating[*]}")
  bad=$(grep -E "^($alloc_re) v" <<<"$tree" || true)
  if [ -n "$bad" ]; then
    echo "error: $crate (features: $features; $triple) is the allocation-free profile," \
      "but its graph holds a package that allocates:" >&2
    sort -u <<<"$bad" >&2
    return 1
  fi
}

# Whether a build on <target>, whose own graph is <tree>, brings in the
# harness: on ARMv6-M, when bytes is in the graph. This keys on thumbv6m and
# bytes, as the harness's [target.thumbv6m-none-eabi] table does. A library
# that ever needs portable-atomic directly, or another target without
# compare-and-swap, needs both generalised (on target_has_atomic = "ptr", say),
# and check_backend revisited.
uses_harness() {
  local triple=$1 tree=$2
  case $triple in thumbv6m-*) grep -q '^bytes v' <<<"$tree" ;; *) return 1 ;; esac
}

# Runs clippy for <crate> with <features> on each <target>, in one cargo run;
# with --harness first, with the harness in the build too.
lint() {
  local with_harness=() with=
  if [ "$1" = --harness ]; then
    with_harness=(-p "$harness")
    with="; with $harness"
    shift
  fi
  local crate=$1 features=$2 args=(--no-default-features) triple
  shift 2
  [ "$features" = - ] || args+=(--features "$features")
  echo "==> $crate (features: $features; $*$with)"
  for triple in "$@"; do args+=(--target "$triple"); done
  # macOS's bash 3.2 treats an empty array as unset under `set -u`, hence
  # the ${...+...}.
  cargo clippy -p "$crate" ${with_harness[@]+"${with_harness[@]}"} --locked "${args[@]}" -- -D warnings
}

if [ $# -gt 0 ]; then targets=("$@"); else targets=("${default_targets[@]}"); fi
for triple in "${targets[@]}"; do
  case $triple in -*|'') echo "usage: $0 [<target triple>...]" >&2; exit 2 ;; esac
done

if command -v rustup >/dev/null; then
  installed=$(rustup target list --installed)
  for triple in "${targets[@]}"; do
    grep -qxF "$triple" <<<"$installed" \
      || { echo "error: target $triple is not installed; rustup target add $triple" >&2; exit 1; }
  done
fi

for check in "${checks[@]}"; do
  read -r crate features <<<"$check"
  feature_args=(--no-default-features)
  [ "$features" = - ] || feature_args+=(--features "$features")
  together=()
  for triple in "${targets[@]}"; do
    case " ${cas_only[*]} " in
      *" $crate "*) if grep -qE "$no_cas_target_re" <<<"$triple"; then
          echo "==> $crate (features: $features; $triple): passed over, no compare-and-swap"
          continue
        fi ;;
    esac
    tree=$(graph -p "$crate" "${feature_args[@]}" --target "$triple")
    check_std "$crate" "$features" "$triple" "" "$tree"
    check_backend "$crate" "$features" "$triple" "$tree"
    check_alloc_free "$crate" "$features" "$triple" "$tree"
    if uses_harness "$triple" "$tree"; then
      tree=$(graph -p "$crate" -p "$harness" "${feature_args[@]}" --target "$triple")
      check_std "$crate" "$features" "$triple" "; with $harness" "$tree"
      lint --harness "$crate" "$features" "$triple"
    else
      together+=("$triple")
    fi
  done
  # Checked before expanding: macOS's bash 3.2 treats an empty array as
  # unset under `set -u`.
  if [ ${#together[@]} -gt 0 ]; then lint "$crate" "$features" "${together[@]}"; fi
done

#!/usr/bin/env bash
# The firmware link test (#1626, #1635, #1664, #1667, #1679): build
# firmware/link's images, one per stack, as bare-metal binaries for each
# target, and check their sizes against firmware/link/size-budget.toml.
# mstp (the MS/TP codec and core), mstp-extended (the same with extended
# frames on), bip (the BVLL codec), node (bacnet-bip-core's foreign-device
# node), bbmd (its BBMD tables) and router (bacnet-router-core between a
# BBMD's B/IP port and an MS/TP port with extended frames) have no global
# allocator; apdu (bacnet-encoding and bacnet-services, no_std with alloc)
# has one, over a fixed arena.
#
#   check-firmware-link.sh [<triple>...]             check the default targets
#                                                    below, or these
#   check-firmware-link.sh --suggest [<triple>...]   print budget tables for
#                                                    the images as they build
#                                                    now, each size plus 12%
#
# check-no-std.sh only runs clippy, which never links. Linking is what shows
# that nothing in a stack needs std or a symbol only a hosted target or the
# firmware provides, such as an atomic library call or a critical section,
# and that every image but apdu needs no allocator: its build fails with "no
# global memory allocator found" if anything in it uses alloc. CI never runs
# the images; the scenarios they link are checked natively by
# firmware/link/tests, and scripts/emu/emulate-firmware.sh runs them under
# QEMU by hand.
#
# The crate's `alloc` feature, on by default, links alloc into its lib, so
# the other images build with --no-default-features and apdu with it on. Per
# target: clippy on the images' code, which a host build compiles out, once
# per feature set; the builds, with the root Cargo.toml's `firmware` profile
# (size-optimized, LTO, panic=abort); then, per image,
# scripts/ci/firmware_size.py, which prints the loaded sections, the four
# main sections and the flash and RAM totals against the budget, and any
# core::fmt code, 64-bit division routine or memmove routine, and fails over
# budget. The
# targets need only `rustup target add`: they link with the rust-lld that
# rustc ships.
set -euo pipefail

cd "$(dirname "$0")/../.."
default_targets=(thumbv7m-none-eabi thumbv6m-none-eabi riscv32imc-unknown-none-elf thumbv7em-none-eabihf)
images=(mstp mstp-extended bip node bbmd router apdu)
# One cargo run per feature set, as its arguments: every image but apdu
# without the crate's default `alloc` feature, which would link alloc into
# them, then the apdu image with it.
builds=("--no-default-features" "--lib --bin apdu")
budget=firmware/link/size-budget.toml

suggest=false
if [ "${1:-}" = --suggest ]; then suggest=true; shift; fi
if [ $# -gt 0 ]; then targets=("$@"); else targets=("${default_targets[@]}"); fi
for triple in "${targets[@]}"; do
  case $triple in -*|'') echo "usage: $0 [--suggest] [<target triple>...]" >&2; exit 2 ;; esac
done

if command -v rustup >/dev/null; then
  installed=$(rustup target list --installed)
  for triple in "${targets[@]}"; do
    grep -qxF "$triple" <<<"$installed" \
      || { echo "error: target $triple is not installed; rustup target add $triple" >&2; exit 1; }
  done
fi

target_dir=$(cargo metadata --format-version 1 --no-deps --locked \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')

status=0
for triple in "${targets[@]}"; do
  for build in "${builds[@]}"; do
    read -ra args <<<"$build"
    if "$suggest"; then
      cargo build -q -p firmware-link --locked --target "$triple" --profile firmware "${args[@]}"
    else
      echo "==> firmware-link ($triple; $build)"
      cargo clippy -p firmware-link --locked --target "$triple" "${args[@]}" -- -D warnings
      cargo build -p firmware-link --locked --target "$triple" --profile firmware "${args[@]}"
    fi
  done
  for image in "${images[@]}"; do
    elf=$target_dir/$triple/firmware/$image
    if "$suggest"; then
      python3 scripts/ci/firmware_size.py --suggest "$elf" "$triple.$image"
      echo
    else
      echo "--> $image"
      python3 scripts/ci/firmware_size.py "$elf" "$budget" "$triple.$image" || status=1
    fi
  done
done
exit $status

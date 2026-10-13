#!/usr/bin/env bash
# Runs the firmware link test's images on emulated boards (docs/ci.md,
# "Emulation (manual)"). A manual check, run at intervals: nothing in CI
# calls it but .github/workflows/emulation.yml, which runs only when
# someone starts it.
#
#   emulate-firmware.sh [--native] [--suggest] [--timeout <seconds>] [<board>...]
#
# For each board (all four unless some are named), this builds the
# firmware link test's seven images (mstp, mstp-extended, bip, node, bbmd,
# router and apdu) for the board's target with the `emulate` feature and
# the board's, with the pinned toolchain, on the host: clippy first, as
# check-firmware-link.sh does, then the `emulate` profile, which is the link
# test's `firmware` profile under its own name. Then scripts/emu/run-qemu.sh
# runs each image on the board's QEMU machine, in the Docker image that
# scripts/emu/Dockerfile pins, built on first use and tagged with the
# Dockerfile's hash; with --native, on the QEMU on PATH instead (Homebrew's
# `qemu`, say), whose version may differ. Each image checks its scenario's
# summary against the one its host test asserts, reports its stack's
# high-water mark, and exits through semihosting; an image whose statics
# leave a board too little RAM for its stack, the router on the two
# 16 KiB boards, is built for it but not run. Then
# scripts/ci/firmware_size.py --stack checks each passing image's peak
# against its budget in firmware/link/stack-budget.toml (#1660). The script
# exits non-zero if anything fails to build, an image fails or runs past
# the timeout (default 30 s per image), or a peak is over its budget or has
# none; a run of every board also fails on a table for a board it doesn't
# have. With --suggest it prints stack budget tables for the images as they
# run now instead, each peak plus 25%, and checks none.
#
# The boards, their targets and QEMU machines (firmware/link/boards/ holds
# their memory layouts):
#
#   microbit        thumbv6m-none-eabi           Cortex-M0, nRF51822
#   lm3s6965evb     thumbv7m-none-eabi           Cortex-M3, LM3S6965
#   netduinoplus2   thumbv7em-none-eabihf        Cortex-M4F, STM32F405
#   sifive_e        riscv32imc-unknown-none-elf  SiFive E31 (RV32IMAC)
set -euo pipefail

cd "$(dirname "$0")/../.."

# board, target, QEMU system (qemu-system-<it>), the board's cargo feature,
# and the images that don't fit its RAM, comma-separated ("-" for none).
# The router image's statics, 15.5 KB, leave the 16 KiB boards less than
# 1 KB of stack, under a quarter of what it needs: it builds for them, but
# doesn't run there (docs/ci.md, "Emulation (manual)").
all_boards=(
  "microbit thumbv6m-none-eabi arm board-microbit router"
  "lm3s6965evb thumbv7m-none-eabi arm board-lm3s6965evb -"
  "netduinoplus2 thumbv7em-none-eabihf arm board-netduinoplus2 -"
  "sifive_e riscv32imc-unknown-none-elf riscv32 board-sifive-e router"
)
# One cargo run per feature set, as in check-firmware-link.sh: every image
# but apdu without the crate's default `alloc` feature, then apdu with it.
builds=("--no-default-features" "--lib --bin apdu")
images=(mstp mstp-extended bip node bbmd router apdu)
stack_budget=firmware/link/stack-budget.toml

usage() {
  echo "usage: $0 [--native] [--suggest] [--timeout <seconds>] [<board>...]" >&2
  echo "boards: microbit lm3s6965evb netduinoplus2 sifive_e" >&2
  exit 2
}

native=false
suggest=false
timeout=30
wanted=()
while [ $# -gt 0 ]; do
  case $1 in
    --native) native=true ;;
    --suggest) suggest=true ;;
    --timeout)
      [ $# -ge 2 ] || usage
      timeout=$2
      shift
      ;;
    -*) usage ;;
    *) wanted+=("$1") ;;
  esac
  shift
done
# A positive whole number, without leading zeros (bash would read 08 as octal).
case $timeout in '' | *[!0-9]* | 0*) usage ;; esac

boards=()
if [ ${#wanted[@]} -eq 0 ]; then
  boards=("${all_boards[@]}")
else
  for name in "${wanted[@]}"; do
    found=
    for board in "${all_boards[@]}"; do
      [ "${board%% *}" = "$name" ] && found=$board
    done
    [ -n "$found" ] || { echo "error: no board named $name" >&2; usage; }
    boards+=("$found")
  done
fi

# Fail early on what the run needs: the targets, and Docker or QEMU.
if command -v rustup >/dev/null; then
  installed=$(rustup target list --installed)
  for board in "${boards[@]}"; do
    read -r _ triple _ _ <<<"$board"
    grep -qxF "$triple" <<<"$installed" \
      || { echo "error: target $triple is not installed; rustup target add $triple" >&2; exit 1; }
  done
fi
if "$native"; then
  for board in "${boards[@]}"; do
    read -r _ _ qemu _ <<<"$board"
    command -v "qemu-system-$qemu" >/dev/null \
      || { echo "error: qemu-system-$qemu is not on PATH (brew install qemu, or drop --native)" >&2; exit 1; }
  done
else
  docker info >/dev/null 2>&1 \
    || { echo "error: Docker isn't running; start it, or use --native with a local QEMU" >&2; exit 1; }
fi

target_dir=$(cargo metadata --format-version 1 --no-deps --locked \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')

jobs=()
machines=()
for board in "${boards[@]}"; do
  read -r machine triple qemu feature unfit <<<"$board"
  machines+=("$machine")
  for build in "${builds[@]}"; do
    read -ra args <<<"$build"
    echo "==> firmware-link for $machine ($triple; --features $feature $build)"
    cargo clippy -p firmware-link --locked --target "$triple" --features "$feature" "${args[@]}" -- -D warnings
    cargo build -p firmware-link --locked --target "$triple" --profile emulate --features "$feature" "${args[@]}"
  done
  for image in "${images[@]}"; do
    case ",$unfit," in
      *",$image,"*)
        echo "--> $image skipped on $machine: its statics leave too little RAM for its stack"
        continue
        ;;
    esac
    jobs+=("$image:$triple:$qemu:$machine:$triple/emulate/$image")
  done
done

echo
if "$native"; then
  echo "==> QEMU on the host"
  run() { bash scripts/emu/run-qemu.sh "$timeout" "$target_dir" "${jobs[@]}"; }
else
  # The image's tag is the Dockerfile's hash, so an edit to it builds a new
  # image instead of reusing a stale one.
  if command -v sha256sum >/dev/null; then
    hash=$(sha256sum scripts/emu/Dockerfile)
  else
    hash=$(shasum -a 256 scripts/emu/Dockerfile)
  fi
  tag=rusty-bacnet-emu:${hash:0:12}
  if ! docker image inspect "$tag" >/dev/null 2>&1; then
    echo "==> building $tag from scripts/emu/Dockerfile (once; under a minute)"
    docker build --quiet --tag "$tag" - <scripts/emu/Dockerfile
  fi
  echo "==> QEMU in $tag"
  # Read-only mounts and no network: the container only runs QEMU on the
  # images and prints what they report.
  run() {
    docker run --rm --network none \
      --volume "$target_dir:/images:ro" \
      --volume "$PWD/scripts/emu:/emu:ro" \
      "$tag" bash /emu/run-qemu.sh "$timeout" /images "${jobs[@]}"
  }
fi

# A copy of what the runs print, for the stack peaks: each image that
# passes prints its peak on its PASS line, on stdout.
log=$(mktemp)
trap 'rm -f "$log"' EXIT
status=0
run | tee "$log" || status=$?

echo
stack_status=0
if "$suggest"; then
  echo "==> stack budgets for $stack_budget, each peak plus 25%"
  python3 scripts/ci/firmware_size.py --stack-suggest "$log" || stack_status=1
else
  args=("$log" "$stack_budget" "${machines[@]}")
  # A run of every board also fails on a table for a board it doesn't have.
  if [ ${#wanted[@]} -eq 0 ]; then args=(--all-boards "${args[@]}"); fi
  python3 scripts/ci/firmware_size.py --stack "${args[@]}" || stack_status=1
fi
# A failed stack check fails the script with 1, unless the run already
# failed: its own status (run-qemu.sh's 1, or Docker's 125, say) stays.
[ "$status" -ne 0 ] || status=$stack_status
exit "$status"

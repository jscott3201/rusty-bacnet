#!/usr/bin/env bash
# Runs the firmware link test's emulation images under QEMU, one at a time,
# and reports how each went (docs/ci.md, "Emulation (manual)").
# scripts/emu/emulate-firmware.sh builds the images and calls this, in the
# pinned Docker image (scripts/emu/Dockerfile) or, with --native, on the
# host:
#
#   run-qemu.sh <timeout> <dir> <image>:<target>:<qemu>:<machine>:<path>...
#
# <timeout> is in seconds, per image. Each image is <dir>/<path>, run on
# qemu-system-<qemu> -machine <machine> with semihosting on, so its output
# reaches this script's stdout and stderr and its exit code becomes QEMU's:
# 0 when the summary matches the expected one with stack paint to spare, and
# otherwise what firmware/link/src/bin/emulate/mod.rs documents. An image
# still running at the timeout is killed and fails as 124, as a hang or a
# fault the image couldn't report would. The script exits 1 if any image
# failed. Besides QEMU it needs only bash, sleep and head, which the Docker
# image's base already has.
set -uo pipefail

if [ $# -lt 3 ]; then
  echo "usage: $0 <timeout> <dir> <image>:<target>:<qemu>:<machine>:<path>..." >&2
  exit 2
fi
timeout=$1 dir=$2
shift 2
# A positive whole number, without leading zeros (bash would read 08 as octal).
case $timeout in '' | *[!0-9]* | 0*)
  echo "error: the timeout is a positive whole number of seconds, not '$timeout'" >&2
  exit 2
  ;;
esac

# Run "$@" with stdin closed, polling every tenth of a second, and kill it
# when $timeout seconds have passed: its exit status, or 124 when it ran out
# of time. Polling needs nothing beyond bash and sleep, unlike timeout(1),
# which macOS lacks.
bounded() {
  "$@" </dev/null &
  local pid=$! ticks=$((timeout * 10))
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$ticks" -le 0 ]; then
      kill -KILL "$pid" 2>/dev/null
      wait "$pid" 2>/dev/null
      return 124
    fi
    sleep 0.1
    ticks=$((ticks - 1))
  done
  wait "$pid"
}

# The QEMU binaries the images need, and their versions. A plain list, not
# an associative array: macOS's bash is 3.2.
seen=" "
for job in "$@"; do
  IFS=: read -r _ _ qemu _ _ <<<"$job"
  case $seen in *" $qemu "*) continue ;; esac
  seen="$seen$qemu "
  binary=qemu-system-$qemu
  command -v "$binary" >/dev/null || { echo "error: $binary is not on PATH" >&2; exit 2; }
  echo "$binary: $("$binary" --version | head -n 1)"
done

results=()
failed=0
for job in "$@"; do
  IFS=: read -r image target qemu machine path <<<"$job"
  echo
  echo "==> $image on $machine ($target)"
  # No display, monitor or serial port: the images talk only through
  # semihosting. (lm3s6965evb's on-chip Ethernet controller warns that it
  # "has no peer"; nothing here uses it.)
  bounded "qemu-system-$qemu" -machine "$machine" -nodefaults -display none \
    -monitor none -serial none \
    -semihosting-config enable=on,target=native -kernel "$dir/$path"
  status=$?
  case $status in
    0) result=pass ;;
    124) result="FAIL: still running after ${timeout}s (a hang, or a fault the image couldn't report)" ;;
    *) result="FAIL: exit status $status" ;;
  esac
  [ "$status" -eq 0 ] || failed=1
  echo "--> $result"
  results+=("$(printf '%-13s %-14s %-28s %s' "$image" "$machine" "$target" "$result")")
done

echo
echo "Emulation results:"
printf '  %s\n' "${results[@]}"
exit $failed

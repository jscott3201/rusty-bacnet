#!/usr/bin/env bash
# Checks, builds and runs the cargo-fuzz targets of the codecs (#1610) and
# the MS/TP master core (#1640) with the nightly toolchain that
# .github/ci-pins.env pins as RUST_FUZZ_NIGHTLY. .github/workflows/fuzz.yml
# runs it, and it runs the same way locally (see docs/ci.md, "Fuzzing"):
#
#   fuzz.sh list                     one "<crate> <target>" line per target
#   fuzz.sh matrix                   the same as a JSON array, for fuzz.yml
#   fuzz.sh check                    each fuzz crate's lockfile, rustfmt, clippy,
#                                    and its tests, which replay saved
#                                    regression inputs
#   fuzz.sh build [<target>]         build every target, or one
#   fuzz.sh run <target> <seconds> [<libFuzzer flag>...]
#                                    fuzz one target for that many seconds
#   fuzz.sh tmin <target> [<input>...]
#                                    minimize failing inputs, by default each
#                                    crash or timeout the target has saved
#   fuzz.sh cmin <target>            minimize the target's corpus
#   fuzz.sh lock [--check]           bring each fuzz Cargo.lock up to date
#                                    with its manifests, changing as little as
#                                    possible; --check fails instead
#
# The fuzz crates are crates/*/fuzz, each a workspace of its own with its own
# Cargo.lock, which records the workspace version of the crate it fuzzes and
# the workspace's requirements for its dependencies: a version bump or a moved
# requirement leaves it stale until `fuzz.sh lock`. Every build checks it
# first, and so does ci.yml's Lint job, with `lock --check`. `lock` needs no
# nightly: any cargo will do.
#
# With FUZZ_WORK set, corpora go in $FUZZ_WORK/corpus/<target> and failing
# inputs in $FUZZ_WORK/artifacts/<target>; without it, in the fuzz crate's
# corpus/ and artifacts/, cargo-fuzz's defaults, which git ignores.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
pins=$root/.github/ci-pins.env

fail() { echo "error: fuzz.sh: $*" >&2; exit 1; }
# The comment above, as the usage message.
usage() { awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0" >&2; exit 2; }

[ "$#" -gt 0 ] || usage
command=$1; shift

nightly=$(sed -n 's/^RUST_FUZZ_NIGHTLY=//p' "$pins")
[[ $nightly =~ ^nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] \
  || fail "no single RUST_FUZZ_NIGHTLY=nightly-YYYY-MM-DD in $pins"

# The crates with a fuzz crate, by directory name.
crates=()
for manifest in "$root"/crates/*/fuzz/Cargo.toml; do
  [ -f "$manifest" ] || continue
  dir=${manifest%/fuzz/Cargo.toml}
  crates+=("${dir##*/}")
done
[ ${#crates[@]} -gt 0 ] || fail "no fuzz crates in crates/*/fuzz"

# targets CRATE: the [[bin]] names in CRATE's fuzz manifest.
targets() {
  awk '/^\[\[bin\]\]/ { bin = 1; next }
       /^\[/ { bin = 0 }
       bin && /^name *= *"/ { sub(/^name *= *"/, ""); sub(/".*$/, ""); print }' \
    "$root/crates/$1/fuzz/Cargo.toml"
}

# crate_of TARGET: the crate whose fuzz crate has TARGET.
crate_of() {
  local crate
  for crate in "${crates[@]}"; do
    if targets "$crate" | grep -xF "$1" >/dev/null; then echo "$crate"; return; fi
  done
  fail "no fuzz target $1; fuzz.sh list names them"
}

# Fail unless the pinned nightly is installed.
need_nightly() {
  rustup toolchain list | grep "^$nightly-" >/dev/null \
    || fail "$nightly is not installed: rustup toolchain install $nightly --profile minimal"
}

# cargo with the pinned nightly.
nightly_cargo() {
  need_nightly
  cargo "+$nightly" "$@"
}

# cargo_fuzz SUBCOMMAND ARG...: cargo-fuzz on the pinned nightly, for the
# nightly's host triple. cargo-fuzz defaults --target to the triple it was
# built for itself, and the release archive CI installs is a musl build:
# AddressSanitizer can't take a statically linked libc, and the nightly has
# no musl standard library.
cargo_fuzz() {
  local subcommand=$1 host
  shift
  need_nightly
  host=$(rustc "+$nightly" -vV | sed -n 's/^host: //p')
  [ -n "$host" ] || fail "rustc +$nightly -vV names no host triple"
  cargo "+$nightly" fuzz "$subcommand" --target "$host" "$@"
}

# locked CRATE: fail unless CRATE's fuzz lockfile is current. Any cargo can
# tell, so this runs whichever the caller has.
locked() {
  cargo fetch --locked --quiet --manifest-path "$root/crates/$1/fuzz/Cargo.toml" \
    || fail "crates/$1/fuzz/Cargo.lock doesn't match its manifests (a workspace version bump or a moved dependency requirement): run bash scripts/ci/fuzz.sh lock and commit the lockfiles"
}

# work KIND TARGET: the corpus or artifact directory for TARGET under
# FUZZ_WORK, created, or nothing without FUZZ_WORK.
work() {
  [ -n "${FUZZ_WORK:-}" ] || return 0
  mkdir -p "$FUZZ_WORK/$1/$2"
  (cd "$FUZZ_WORK/$1/$2" && pwd)
}

# in_crate CRATE TARGET COMMAND...: run COMMAND in CRATE's directory, where
# cargo-fuzz finds the fuzz crate. cargo-fuzz creates its own artifacts
# directory even when FUZZ_WORK takes its place; that one is removed after,
# if it is still empty.
in_crate() {
  local crate=$1 target=$2 status=0
  shift 2
  (cd "$root/crates/$crate" && "$@") || status=$?
  if [ -n "${FUZZ_WORK:-}" ]; then
    rmdir "$root/crates/$crate/fuzz/artifacts/$target" "$root/crates/$crate/fuzz/artifacts" \
      2>/dev/null || true
  fi
  return "$status"
}

case $command in
  list)
    for crate in "${crates[@]}"; do
      targets "$crate" | sed "s/^/$crate /"
    done ;;
  matrix)
    entries=()
    for crate in "${crates[@]}"; do
      for target in $(targets "$crate"); do
        [[ $crate$target =~ ^[a-z0-9_-]+$ ]] || fail "unexpected characters in $crate $target"
        entries+=("{\"crate\":\"$crate\",\"target\":\"$target\"}")
      done
    done
    (IFS=,; echo "[${entries[*]}]") ;;
  check)
    for crate in "${crates[@]}"; do
      manifest=$root/crates/$crate/fuzz/Cargo.toml
      echo "==> $crate"
      locked "$crate"
      nightly_cargo fmt --manifest-path "$manifest" --check
      nightly_cargo clippy --locked --manifest-path "$manifest" --all-targets -- -D warnings
      nightly_cargo test --locked --manifest-path "$manifest" --quiet
    done ;;
  build)
    [ "$#" -le 1 ] || usage
    if [ "$#" -eq 1 ]; then
      crate=$(crate_of "$1")
      locked "$crate"
      (cd "$root/crates/$crate" && cargo_fuzz build "$1")
    else
      for crate in "${crates[@]}"; do
        echo "==> $crate"
        locked "$crate"
        (cd "$root/crates/$crate" && cargo_fuzz build)
      done
    fi ;;
  run)
    [ "$#" -ge 2 ] || usage
    target=$1 seconds=$2; shift 2
    [[ $seconds =~ ^[1-9][0-9]*$ ]] || fail "seconds must be a positive whole number, not $seconds"
    crate=$(crate_of "$target")
    locked "$crate"
    corpus=$(work corpus "$target")
    artifacts=$(work artifacts "$target")
    # A positional corpus directory replaces cargo-fuzz's default one; a
    # later -artifact_prefix overrides the one cargo-fuzz passes. A single
    # input that runs 10 seconds is a hang.
    in_crate "$crate" "$target" cargo_fuzz run "$target" ${corpus:+"$corpus"} -- \
      -max_total_time="$seconds" -timeout=10 -print_final_stats=1 \
      ${artifacts:+"-artifact_prefix=$artifacts/"} "$@" ;;
  tmin)
    [ "$#" -ge 1 ] || usage
    target=$1; shift
    crate=$(crate_of "$target")
    artifacts=$(work artifacts "$target")
    dir=${artifacts:-$root/crates/$crate/fuzz/artifacts/$target}
    inputs=("$@")
    if [ ${#inputs[@]} -eq 0 ]; then
      for input in "$dir"/crash-* "$dir"/timeout-*; do
        [ -f "$input" ] && inputs+=("$input")
      done
    fi
    [ ${#inputs[@]} -gt 0 ] || fail "no failing input for $target in $dir"
    locked "$crate"
    status=0
    for input in "${inputs[@]}"; do
      input=$(cd "$(dirname "$input")" && pwd)/$(basename "$input")
      echo "==> minimizing $input"
      in_crate "$crate" "$target" cargo_fuzz tmin "$target" "$input" -- -timeout=10 \
        ${artifacts:+"-artifact_prefix=$artifacts/"} || status=1
    done
    exit "$status" ;;
  cmin)
    # Keeps the smallest set of inputs that covers what the whole corpus
    # does, so a corpus saved run after run doesn't grow without bound.
    [ "$#" -eq 1 ] || usage
    target=$1
    crate=$(crate_of "$target")
    locked "$crate"
    corpus=$(work corpus "$target")
    before=$(find "${corpus:-$root/crates/$crate/fuzz/corpus/$target}" -type f | wc -l)
    in_crate "$crate" "$target" cargo_fuzz cmin "$target" ${corpus:+"$corpus"} -- -timeout=10
    after=$(find "${corpus:-$root/crates/$crate/fuzz/corpus/$target}" -type f | wc -l)
    echo "corpus of $target: $((before)) inputs before, $((after)) after" ;;
  lock)
    [ "$#" -le 1 ] || usage
    case ${1:-} in
      --check) for crate in "${crates[@]}"; do locked "$crate"; done ;;
      '')
        # Without --locked, cargo re-resolves only the entries the manifests
        # no longer allow, the fuzzed crate's own version included, and
        # keeps every other dependency where the lockfile has it. That fails
        # when a moved requirement lands on a dependency the fuzz crate names
        # itself too (bytes, heapless): its locked version satisfies the fuzz
        # crate, so cargo keeps it. Then unlock the fuzzed crate's
        # dependencies and resolve them again.
        for crate in "${crates[@]}"; do
          manifest=$root/crates/$crate/fuzz/Cargo.toml
          if ! cargo fetch --quiet --manifest-path "$manifest" 2>/dev/null; then
            echo "crates/$crate/fuzz/Cargo.lock: resolving $crate's dependencies again"
            cargo update --manifest-path "$manifest" -p "$crate" --recursive
          fi
          locked "$crate"
          echo "crates/$crate/fuzz/Cargo.lock is up to date"
        done ;;
      *) usage ;;
    esac ;;
  *) usage ;;
esac

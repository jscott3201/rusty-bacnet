#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/bench-local.sh <suite> [options]

Suites:
  smoke                  Compile benchmark crate tests without running benches
  codec|encoding         Criterion encoding/APDU/NPDU suite
  bip                    Criterion BIP latency and throughput suites
  bip-latency            Criterion BIP latency suite
  bip-throughput         Criterion BIP throughput suite
  sc                     Criterion SC latency and throughput suites
  bbmd                   Stress BBMD foreign-device suite
  router                 Stress router forwarding suite
  segmentation           Stress segmentation suite
  clients                Stress concurrent-client suite
  objects                Stress object-scale suite
  whois                  Stress Who-Is/device-scan suite

Options:
  --repo <path>          Repository checkout to run in (default: current repo)
  --quick                Reduce Criterion sample/warmup/measurement time
  --noplot               Disable Criterion plots
  --duration <seconds>   Stress duration where supported (default: 5)
  --steps <csv>          Stress step list override
  --json                 Accepted for callers; stress suites already emit JSON
  -h, --help             Show this help
USAGE
}

if [[ $# -eq 0 ]]; then
  usage >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
default_repo="$(cd "$script_dir/.." && pwd)"
repo="$default_repo"
suite="$1"
shift

quick=false
noplot=false
duration=5
steps=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      repo="${2:?--repo requires a path}"
      shift 2
      ;;
    --quick)
      quick=true
      shift
      ;;
    --noplot)
      noplot=true
      shift
      ;;
    --duration)
      duration="${2:?--duration requires seconds}"
      shift 2
      ;;
    --steps)
      steps="${2:?--steps requires a comma-separated list}"
      shift 2
      ;;
    --json)
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

repo="$(cd "$repo" && pwd)"

criterion_args=()
if [[ "$quick" == true ]]; then
  criterion_args+=(--sample-size 10 --warm-up-time 1 --measurement-time 2)
fi
if [[ "$noplot" == true ]]; then
  criterion_args+=(--noplot)
fi

run_cargo() {
  (cd "$repo" && cargo "$@")
}

run_criterion() {
  local bench_name="$1"
  run_cargo bench -p bacnet-benchmarks --bench "$bench_name" -- "${criterion_args[@]}"
}

run_stress() {
  local command="$1"
  local default_steps="$2"
  local selected_steps="${steps:-$default_steps}"
  run_cargo run --release -p bacnet-benchmarks --bin stress-test -- "$command" \
    --duration "$duration" \
    --steps "$selected_steps"
}

case "$suite" in
  smoke)
    run_cargo test -p bacnet-benchmarks --locked --no-run
    ;;
  codec|encoding|apdu|npdu|bvll)
    run_criterion encoding
    ;;
  bip)
    run_criterion bip_latency
    run_criterion bip_throughput
    ;;
  bip-latency|bip_latency)
    run_criterion bip_latency
    ;;
  bip-throughput|bip_throughput)
    run_criterion bip_throughput
    ;;
  sc)
    run_criterion sc_latency
    run_criterion sc_throughput
    ;;
  bbmd)
    run_stress bbmd "1,3"
    ;;
  router)
    run_stress router "1,3,5"
    ;;
  segmentation)
    selected_steps="${steps:-10,25,50}"
    run_cargo run --release -p bacnet-benchmarks --bin stress-test -- segmentation \
      --steps "$selected_steps"
    ;;
  clients)
    run_stress clients "1,5,10,25,50"
    ;;
  objects)
    run_stress objects "100,500,1000,2500,5000"
    ;;
  whois)
    run_stress whois "3,10,25"
    ;;
  *)
    echo "unknown benchmark suite: $suite" >&2
    usage >&2
    exit 2
    ;;
esac

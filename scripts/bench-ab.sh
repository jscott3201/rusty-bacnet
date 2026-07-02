#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/bench-ab.sh --base <ref> --head <ref> --suites <csv> --output-dir <dir> [options]

Options:
  --duration <seconds>   Stress-suite duration where supported (default: 5)
  --allow-dirty          Allow running with local uncommitted changes
  --quick                Pass quick Criterion settings to bench-local
  --noplot               Disable Criterion plots
  -h, --help             Show this help

Example:
  scripts/bench-ab.sh --base origin/dev --head HEAD --suites bip,bbmd \
    --duration 5 --output-dir bench-output/ab-annex-j
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

base_ref=""
head_ref=""
suites_csv=""
output_dir=""
duration=5
allow_dirty=false
quick=false
noplot=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      base_ref="${2:?--base requires a ref}"
      shift 2
      ;;
    --head)
      head_ref="${2:?--head requires a ref}"
      shift 2
      ;;
    --suites)
      suites_csv="${2:?--suites requires a comma-separated list}"
      shift 2
      ;;
    --output-dir)
      output_dir="${2:?--output-dir requires a path}"
      shift 2
      ;;
    --duration)
      duration="${2:?--duration requires seconds}"
      shift 2
      ;;
    --allow-dirty)
      allow_dirty=true
      shift
      ;;
    --quick)
      quick=true
      shift
      ;;
    --noplot)
      noplot=true
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

if [[ -z "$base_ref" || -z "$head_ref" || -z "$suites_csv" || -z "$output_dir" ]]; then
  usage >&2
  exit 2
fi

if [[ "$allow_dirty" != true ]]; then
  if ! git -C "$repo_root" diff --quiet || ! git -C "$repo_root" diff --cached --quiet; then
    echo "working tree is dirty; commit/stash changes or pass --allow-dirty" >&2
    exit 1
  fi
fi

base_sha="$(git -C "$repo_root" rev-parse "$base_ref")"
head_sha="$(git -C "$repo_root" rev-parse "$head_ref")"
output_dir="$(mkdir -p "$output_dir" && cd "$output_dir" && pwd)"

worktree_root="$output_dir/worktrees"
mkdir -p "$worktree_root" "$output_dir/base" "$output_dir/head"

cleanup() {
  git -C "$repo_root" worktree remove --force "$worktree_root/base" >/dev/null 2>&1 || true
  git -C "$repo_root" worktree remove --force "$worktree_root/head" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
git -C "$repo_root" worktree add --detach "$worktree_root/base" "$base_sha" >/dev/null
git -C "$repo_root" worktree add --detach "$worktree_root/head" "$head_sha" >/dev/null

IFS=',' read -r -a suites <<< "$suites_csv"
bench_flags=(--duration "$duration" --json)
if [[ "$quick" == true ]]; then
  bench_flags+=(--quick)
fi
if [[ "$noplot" == true ]]; then
  bench_flags+=(--noplot)
fi

write_env() {
  local file="$1"
  python3 - "$file" "$base_ref" "$base_sha" "$head_ref" "$head_sha" "$suites_csv" "$duration" <<'PY'
import json
import os
import platform
import subprocess
import sys

file, base_ref, base_sha, head_ref, head_sha, suites, duration = sys.argv[1:]

def cmd(args):
    try:
        return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT).strip()
    except Exception as exc:
        return f"unavailable: {exc}"

data = {
    "base": {"ref": base_ref, "sha": base_sha},
    "head": {"ref": head_ref, "sha": head_sha},
    "suites": [s for s in suites.split(",") if s],
    "duration_secs": int(duration),
    "environment": {
        "os": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "python": platform.python_version(),
        "rustc": cmd(["rustc", "--version"]),
        "cargo": cmd(["cargo", "--version"]),
    },
}
with open(file, "w", encoding="utf-8") as fh:
    json.dump(data, fh, indent=2)
    fh.write("\n")
PY
}

run_side() {
  local side="$1"
  local checkout="$2"
  local suite
  for suite in "${suites[@]}"; do
    suite="${suite//[[:space:]]/}"
    [[ -z "$suite" ]] && continue
    echo "[$side] $suite" >&2
    set +e
    "$repo_root/scripts/bench-local.sh" "$suite" --repo "$checkout" "${bench_flags[@]}" \
      >"$output_dir/$side/$suite.out" \
      2>"$output_dir/$side/$suite.err"
    status=$?
    set -e
    printf '%s\n' "$status" >"$output_dir/$side/$suite.status"
    if [[ "$status" -ne 0 ]]; then
      echo "suite $suite failed on $side with status $status" >&2
    fi
  done
}

write_env "$output_dir/environment.json"
run_side base "$worktree_root/base"
run_side head "$worktree_root/head"

python3 - "$output_dir" <<'PY'
import json
import sys
from pathlib import Path

out = Path(sys.argv[1])
env = json.loads((out / "environment.json").read_text())
suites = env["suites"]

def load(side, suite):
    status_file = out / side / f"{suite}.status"
    status = int(status_file.read_text().strip()) if status_file.exists() else 127
    raw = out / side / f"{suite}.out"
    parsed = None
    if raw.exists():
        text = raw.read_text(errors="replace").strip()
        if text.startswith("{"):
            try:
                parsed = json.loads(text)
            except json.JSONDecodeError:
                parsed = None
    return status, parsed

def metric(parsed):
    if not parsed:
        return None
    results = parsed.get("results", {})
    latency = results.get("latency_us", {})
    return {
        "throughput": results.get("throughput_ops_sec"),
        "p50": latency.get("p50"),
        "p99": latency.get("p99"),
        "errors": results.get("failed"),
    }

def pct(base, head):
    if base in (None, 0) or head is None:
        return ""
    return f"{((head - base) / base) * 100:+.1f}%"

lines = [
    "# Benchmark A/B Summary",
    "",
    f"- Base: `{env['base']['ref']}` / `{env['base']['sha']}`",
    f"- Head: `{env['head']['ref']}` / `{env['head']['sha']}`",
    f"- Duration: {env['duration_secs']}s",
    f"- Rust: {env['environment']['rustc']}",
    f"- Cargo: {env['environment']['cargo']}",
    f"- OS: {env['environment']['os']}",
    "",
    "| Suite | Base status | Head status | Base throughput | Head throughput | Throughput delta | Base p99 | Head p99 | p99 delta | Errors | Result |",
    "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|",
]

for suite in suites:
    base_status, base_parsed = load("base", suite)
    head_status, head_parsed = load("head", suite)
    b = metric(base_parsed)
    h = metric(head_parsed)
    if b and h:
        errors = f"{b['errors']} -> {h['errors']}"
        result = "needs-review" if base_status or head_status or h["errors"] else "ok"
        lines.append(
            "| {suite} | {bs} | {hs} | {bt:.2f} | {ht:.2f} | {td} | {bp99} | {hp99} | {pd} | {errors} | {result} |".format(
                suite=suite,
                bs=base_status,
                hs=head_status,
                bt=b["throughput"] or 0.0,
                ht=h["throughput"] or 0.0,
                td=pct(b["throughput"], h["throughput"]),
                bp99=b["p99"] if b["p99"] is not None else "",
                hp99=h["p99"] if h["p99"] is not None else "",
                pd=pct(b["p99"], h["p99"]),
                errors=errors,
                result=result,
            )
        )
    else:
        result = "see-logs" if base_status == 0 and head_status == 0 else "needs-review"
        lines.append(f"| {suite} | {base_status} | {head_status} |  |  |  |  |  |  |  | {result} |")

lines.extend([
    "",
    "Raw artifacts:",
    "- `environment.json`",
    "- `base/<suite>.out`, `base/<suite>.err`, `base/<suite>.status`",
    "- `head/<suite>.out`, `head/<suite>.err`, `head/<suite>.status`",
    "",
    "Criterion suites write textual output in `.out` files; stress suites emit JSON and are summarized above.",
])

(out / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
print(out / "summary.md")
PY

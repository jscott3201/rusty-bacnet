#!/usr/bin/env python3
"""The cost of a cheap Rust mutation policy per WriteProperty, asked on
Tokio's blocking pool or inline (#2049), measured as the counting harness
measures (Benchmarks.md section 10.4).

  measure.py build SHA --out DIR        export SHA, refresh the harness lock, build in Docker
  measure.py counts --out DIR [--blocks 3]  callgrind: none/pool/inline x ct/mt x 1,000/10,000
  measure.py wall --out DIR [--blocks 3]    the same binary without Valgrind, 20,000 requests, wall time
  measure.py analyze --out DIR              DIR/summary.json and DIR/summary.md
  measure.py clean --out DIR                remove the build's Docker volumes

It runs `benchmarks/counts/examples/policy_counts.rs`, which isn't one of the
harness's binaries, so `scripts/perf/perf_counts.py` can't run it; this
borrows that script's image, lock refresh, build and run settings and
count_analysis.py's parsing: the image rb-perf-counts:<tag> perf_counts.py
derives, --locked, CARGO_INCREMENTAL=0, GLIBC_TUNABLES, --network none
for runs, the binary's hash checked before each run, the counting
allocator's cost subtracted. One build, run in the order none pool inline
inline pool none per block, so each policy gets two runs per block. With
one build there is no A' floor: the range of each row's six values stands
in for it.
"""

import argparse
import json
import statistics
import subprocess
import sys
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "scripts/perf"))
import count_analysis as ca  # noqa: E402
import perf_counts as pc  # noqa: E402

BINARY = "/target/release/examples/policy_counts"
VARIANTS = ("none", "pool", "inline")
ORDER = ("none", "pool", "inline", "inline", "pool", "none")
RUNTIMES = ("ct", "mt")
LENGTHS = (1_000, 10_000)
WALL_N = 20_000
ENV = {"GLIBC_TUNABLES": pc.GLIBC_TUNABLES}


def sh(cmd, **kw):
    print("+", " ".join(cmd)[:200], flush=True)
    return subprocess.run(cmd, check=True, text=True, **kw)


def build(sha, out):
    tree = out / "tree"
    tree.mkdir(parents=True)
    archive = subprocess.run(["git", "-C", str(REPO), "archive", sha], check=True, capture_output=True).stdout
    subprocess.run(["tar", "-x", "-C", str(tree)], input=archive, check=True)
    image = pc.ensure_image()
    run_id = uuid.uuid4().hex[:8]
    registry, target = f"rb-perf-2049-{run_id}-registry", f"rb-perf-2049-{run_id}-target"
    for volume in (registry, target):
        sh(["docker", "volume", "create", volume], stdout=subprocess.DEVNULL)
    lock = tree / "benchmarks/counts/Cargo.lock"
    before = lock.read_text()
    sh(["docker", "run", "--rm", "-v", f"{tree}:/wt", "-v", f"{registry}:/opt/cargo/registry", image["id"],
        "sh", "-c", "cd /wt/benchmarks/counts && cargo update --workspace --quiet"])
    added, dropped = pc.lock_drift(before, lock.read_text())
    if added:
        raise SystemExit(f"the lock refresh adds or changes third-party packages: {added}")
    script = ("set -e; cd /wt/benchmarks/counts; rustc -vV; "
              "cargo build --release --locked --example policy_counts; "
              f"sha256sum {BINARY}")
    built = sh(["docker", "run", "--rm", "-v", f"{tree}:/wt:ro", "-v", f"{target}:/target",
                "-v", f"{registry}:/opt/cargo/registry", "-e", "CARGO_TARGET_DIR=/target",
                "-e", "CARGO_INCREMENTAL=0", image["id"], "sh", "-c", script], stdout=subprocess.PIPE).stdout
    state = {
        "sha": pc.resolve(sha),
        "image": {k: image[k] for k in ("tag", "id", "packages")},
        "volumes": [registry, target],
        "binary_sha256": next(line.split()[0] for line in built.splitlines() if line.endswith(BINARY)),
        "rustc": next(line.split()[1] for line in built.splitlines() if line.startswith("release:")),
        "lock_dropped": [f"{name} {version}" for name, version, _ in dropped],
        "environment": ENV,
    }
    (out / "state.json").write_text(json.dumps(state, indent=2))


def run_one(state, out_dir, name, args, valgrind):
    tool = (" ".join(pc.VALGRIND) + f" --callgrind-out-file=/out/{name}.cg ") if valgrind else ""
    script = (f'test "$(sha256sum {BINARY} | cut -d " " -f 1)" = "{state["binary_sha256"]}" '
              f"|| {{ echo 'binary hash mismatch' >&2; exit 97; }}; "
              f"{tool}{BINARY} {' '.join(args)}; status=$?; chmod a+r /out/{name}.cg 2>/dev/null; exit \"$status\"")
    env = [arg for key, value in sorted(ENV.items()) for arg in ("-e", f"{key}={value}")]
    result = subprocess.run(["docker", "run", "--rm", "--network", "none", *env,
                             "-v", f"{state['volumes'][1]}:/target:ro", "-v", f"{out_dir}:/out",
                             state["image"]["id"], "sh", "-c", script], text=True, capture_output=True)
    (out_dir / f"{name}.stderr").write_text(result.stderr)
    if result.returncode != 0:
        raise SystemExit(f"{name} failed ({result.returncode}):\n{result.stderr[-2000:]}")


def matrix(out, kind, blocks):
    state = json.loads((out / "state.json").read_text())
    out_dir = out / kind
    out_dir.mkdir(exist_ok=True)
    lengths = LENGTHS if kind == "raw" else (WALL_N,)
    seen = {}
    for _block in range(blocks):
        for variant in ORDER:
            seen[variant] = seen.get(variant, 0) + 1
            for runtime in RUNTIMES:
                for n in lengths:
                    name = f"{variant}-{runtime}-{seen[variant]:02}-{n}"
                    print("RUN", name, flush=True)
                    run_one(state, out_dir, name, [variant, runtime, str(n)], kind == "raw")


def wall_ns(text):
    for line in text.splitlines():
        if line.startswith("WALL "):
            return float(dict(part.split("=") for part in line.split()[1:])["ns_per_iter"])
    raise ValueError("no WALL line")


def analyze(out):
    raw, wall = out / "raw", out / "wall"
    rows, calls = {}, {}
    for variant in VARIANTS:
        for runtime in RUNTIMES:
            per = {"instructions": [], "with_harness": [], "allocs": [], "bytes": []}
            call_slopes = []
            ordinal = 1
            while (raw / f"{variant}-{runtime}-{ordinal:02}-{LENGTHS[0]}.cg").exists():
                parsed = {}
                for n in LENGTHS:
                    stem = raw / f"{variant}-{runtime}-{ordinal:02}-{n}"
                    text = stem.with_suffix(".cg").read_text()
                    _lines, functions, edges = ca.parse_callgrind(text)
                    parsed[n] = {"ir": ca.parse_summary(text), "harness": ca.harness_ir(functions, edges),
                                 "calls": ca.calls_in(edges),
                                 **ca.parse_result(stem.with_suffix(".stderr").read_text())}
                short, long = parsed[LENGTHS[0]], parsed[LENGTHS[1]]
                per["with_harness"].append(ca.slope(short["ir"], long["ir"], *LENGTHS))
                per["instructions"].append(ca.slope(short["ir"] - short["harness"],
                                                    long["ir"] - long["harness"], *LENGTHS))
                per["allocs"].append(long["allocs"] / long["n"])
                per["bytes"].append(long["bytes"] / long["n"])
                call_slopes.append({f: ca.slope(short["calls"].get(f, 0), long["calls"].get(f, 0), *LENGTHS)
                                    for f in set(short["calls"]) | set(long["calls"])})
                ordinal += 1
            walls = sorted(wall_ns(p.read_text()) for p in wall.glob(f"{variant}-{runtime}-*.stderr"))
            rows[f"{variant}/{runtime}"] = {
                "values": per["instructions"],
                "median": statistics.median(per["instructions"]),
                "range": max(per["instructions"]) - min(per["instructions"]),
                "median_with_harness": statistics.median(per["with_harness"]),
                "alloc_calls": statistics.median(per["allocs"]),
                "alloc_bytes": statistics.median(per["bytes"]),
                "wall_ns_per_request": walls,
                "wall_median": statistics.median(walls) if walls else None,
            }
            functions = set().union(*call_slopes)
            calls[f"{variant}/{runtime}"] = {f: statistics.median([c.get(f, 0.0) for c in call_slopes])
                                             for f in functions}
    diffs = {}
    for runtime in RUNTIMES:
        pool, inline = calls[f"pool/{runtime}"], calls[f"inline/{runtime}"]
        moved = ((pool.get(f, 0) - inline.get(f, 0), f) for f in set(pool) | set(inline))
        diffs[runtime] = sorted((m for m in moved if abs(m[0]) >= 0.5), key=lambda m: (-abs(m[0]), m[1]))[:25]
    state = json.loads((out / "state.json").read_text())
    summary = {"state": state, "rows": rows, "call_differences_pool_minus_inline": diffs}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    md = ["| Policy | Runtime | Instructions / request | Range of 6 | With counting allocator "
          "| Allocator calls | Bytes | Wall ns / request |",
          "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for key, row in rows.items():
        variant, runtime = key.split("/")
        wall_median = f"{row['wall_median']:,.0f}" if row["wall_median"] else "-"
        md.append(f"| {variant} | {runtime} | {row['median']:,.1f} | {row['range']:,.1f} | "
                  f"{row['median_with_harness']:,.1f} | {row['alloc_calls']:.4f} | {row['alloc_bytes']:,.2f} | "
                  f"{wall_median} |")
    for runtime in RUNTIMES:
        md += ["", f"Calls per request, pool minus inline, {runtime}:", ""]
        md += [f"- {d:+.2f} `{f}`" for d, f in diffs[runtime]]
    (out / "summary.md").write_text("\n".join(md) + "\n")
    print("\n".join(md))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("build", "counts", "wall", "analyze", "clean"))
    parser.add_argument("sha", nargs="?")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--blocks", type=int, default=3)
    args = parser.parse_args()
    out = args.out.resolve()
    if args.command == "build":
        if not args.sha:
            parser.error("build takes a SHA")
        build(args.sha, out)
    elif args.command == "counts":
        matrix(out, "raw", args.blocks)
    elif args.command == "wall":
        matrix(out, "wall", args.blocks)
    elif args.command == "clean":
        for volume in json.loads((out / "state.json").read_text())["volumes"]:
            sh(["docker", "volume", "rm", volume], stdout=subprocess.DEVNULL)
    else:
        analyze(out)


if __name__ == "__main__":
    main()

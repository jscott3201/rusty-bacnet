#!/usr/bin/env python3
"""Runs the counting harness's matrix in Docker and applies its gates
(Benchmarks.md section 10.4; the rules are in count_analysis.py).

  perf_counts.py run --baseline REF --candidate REF [options]   # A, A' and B
  perf_counts.py run --noise REF [options]                      # A and A'
  perf_counts.py analyze OUT [--aim rp,...]

`run`:
1. builds the image from scripts/perf/Dockerfile (tag rb-perf-counts:<hash
   of the Dockerfile and its build arguments>) unless it exists, and records
   its libc6, valgrind and gcc package versions;
2. exports each ref with `git archive` into OUT/trees/<label>, and replaces
   its benchmarks/counts/ with one harness (the candidate's, or --harness);
   in each tree, `cargo update --workspace` refreshes only the stack's path
   packages in the harness lock, and the run stops if any third-party
   package would change;
3. builds each tree in a fresh Docker volume at the same paths (/wt, /target)
   with --locked and CARGO_INCREMENTAL=0; a comparison builds the baseline
   twice (A and A'), so its noise floor comes from the same matrix;
4. runs each workload and runtime in the order A B A' A' B A (A A' A' A for
   a noise run), --blocks times, each position at the short and the long
   loop length under callgrind; every run first checks its binary's SHA-256
   and runs without a network;
5. writes callgrind_annotate's per-function view of every run, then
   analyzes: OUT/summary.json, OUT/summary.md (the verdict table),
   OUT/profiles/*.md (per-function and per-line slopes) and OUT/items.md;
6. removes the volumes and trees (unless --keep).

A comparison exits 1 when it isn't accepted, 2 on a usage error. Quick mode
(--noise-floors FILE) skips the A' build and takes the floors from an
earlier noise run on the same machine; use it only for local iteration.
"""

import argparse
import datetime
import gzip
import hashlib
import json
import re
import shutil
import subprocess
import sys
import uuid
from pathlib import Path

import count_analysis as ca

REPO = Path(__file__).resolve().parents[2]
DOCKERFILE = REPO / "scripts/perf/Dockerfile"
HARNESS_FILES = ("Cargo.toml", "Cargo.lock", "src")
BINARIES = ("counts", "codec-counts")
VALGRIND = ["valgrind", "--tool=callgrind", "--compress-pos=no"]


def log(*parts):
    print(*parts, flush=True)


def sh(command, **kwargs):
    return subprocess.run(command, check=True, text=True, **kwargs)


def out_of(command):
    return sh(command, stdout=subprocess.PIPE).stdout.strip()


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


# ---------------------------------------------------------------- inputs


def image_build_args():
    pins = {}
    for line in (REPO / ".github/ci-pins.env").read_text().splitlines():
        if "=" in line and not line.startswith("#"):
            key, _, value = line.partition("=")
            pins[key] = value
    channel = None
    for line in (REPO / "rust-toolchain.toml").read_text().splitlines():
        if line.startswith("channel = "):
            channel = line.split('"')[1]
    return {
        "RUSTUP_VERSION": pins["RUSTUP_VERSION"],
        "RUSTUP_AARCH64_SHA256": pins["RUSTUP_AARCH64_SHA256"],
        "RUST_TOOLCHAIN": channel,
    }


def ensure_image(tag=None):
    args = image_build_args()
    digest = hashlib.sha256(DOCKERFILE.read_bytes() + json.dumps(args, sort_keys=True).encode()).hexdigest()
    tag = tag or f"rb-perf-counts:{digest[:12]}"
    probe = subprocess.run(["docker", "image", "inspect", tag], capture_output=True)
    if probe.returncode != 0:
        log("BUILD IMAGE", tag)
        command = ["docker", "build", "--platform", "linux/arm64", "-t", tag, "-f", str(DOCKERFILE)]
        for key, value in args.items():
            command += ["--build-arg", f"{key}={value}"]
        sh(command + [str(DOCKERFILE.parent)])
    image_id = out_of(["docker", "image", "inspect", tag, "--format", "{{.Id}}"])
    packages = out_of(["docker", "run", "--rm", "--network", "none", image_id,
                       "dpkg-query", "-W", "-f", "${Package} ${Version}\\n", "libc6", "valgrind", "gcc"])
    return {"tag": tag, "id": image_id, "build_args": args, "dockerfile_sha256": sha256(DOCKERFILE),
            "packages": dict(line.split(" ", 1) for line in packages.splitlines() if " " in line)}


def resolve(ref):
    return out_of(["git", "-C", str(REPO), "rev-parse", "--verify", f"{ref}^{{commit}}"])


def export_tree(sha, dest):
    dest.mkdir(parents=True)
    archive = subprocess.Popen(["git", "-C", str(REPO), "archive", "--format=tar", sha], stdout=subprocess.PIPE)
    sh(["tar", "-x", "-C", str(dest)], stdin=archive.stdout)
    if archive.wait() != 0:
        raise RuntimeError(f"git archive {sha} failed")


def harness_hashes(directory):
    out = {}
    for path in sorted(directory.rglob("*")):
        if path.is_file() and "target" not in path.relative_to(directory).parts:
            out[str(path.relative_to(directory))] = sha256(path)
    return out


def install_harness(source, tree):
    target = tree / "benchmarks/counts"
    if target.exists():
        shutil.rmtree(target)
    target.mkdir(parents=True)
    for name in HARNESS_FILES:
        path = source / name
        if path.is_dir():
            shutil.copytree(path, target / name)
        else:
            shutil.copy2(path, target / name)
    return harness_hashes(target)


# ---------------------------------------------------------------- docker


def third_party(lock_text):
    """{(name, version, source)} of the registry packages in a Cargo.lock."""
    out = set()
    for block in lock_text.split("[[package]]")[1:]:
        fields = dict(re.findall(r'^(name|version|source) = "([^"]*)"', block, re.M))
        if "source" in fields:
            out.add((fields["name"], fields["version"], fields["source"]))
    return out


def refresh_lock(image, variant, registry):
    """Re-resolves the tree's path packages in its copy of the harness lock
    and refuses any third-party change, so both trees build the same
    third-party versions."""
    lock = Path(variant["root"]) / "benchmarks/counts/Cargo.lock"
    before = lock.read_text()
    sh([
        "docker", "run", "--rm", "-v", f"{variant['root']}:/wt", "-v", f"{registry}:/opt/cargo/registry",
        image["id"], "sh", "-c", "cd /wt/benchmarks/counts && cargo update --workspace --quiet",
    ])
    after = lock.read_text()
    added, removed = third_party(after) - third_party(before), third_party(before) - third_party(after)
    if added or removed:
        raise RuntimeError(
            f"{variant['label']}: refreshing the harness lock for {variant['sha'][:12]} would change third-party "
            f"packages (added {sorted(added)}, removed {sorted(removed)}); update benchmarks/counts/Cargo.lock "
            "so it serves both trees"
        )
    variant["lock_sha256"] = sha256(lock)
    variant["lock_refreshed"] = after != before


def build_variant(image, variant, registry):
    """Builds the harness in a fresh volume; returns the binaries' hashes."""
    sh(["docker", "volume", "create", variant["volume"]], stdout=subprocess.DEVNULL)
    script = (
        "set -e; cd /wt/benchmarks/counts; rustc -vV; "
        "cargo build --release --locked --bins; "
        "cd /target/release && sha256sum " + " ".join(BINARIES)
    )
    result = sh(
        [
            "docker", "run", "--rm",
            "-v", f"{variant['root']}:/wt:ro",
            "-v", f"{variant['volume']}:/target",
            "-v", f"{registry}:/opt/cargo/registry",
            "-e", "CARGO_TARGET_DIR=/target", "-e", "CARGO_INCREMENTAL=0",
            image["id"], "sh", "-c", script,
        ],
        stdout=subprocess.PIPE,
    ).stdout
    hashes = {}
    for line in result.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1] in BINARIES:
            hashes[parts[1]] = parts[0]
        elif line.startswith("release:"):
            variant["rustc"] = line.split()[1]
    if set(hashes) != set(BINARIES):
        raise RuntimeError(f"build of {variant['label']} reported no binary hashes")
    return hashes


def run_command(image, variant, raw, name, binary, args):
    expected = variant["binaries"][binary]
    script = (
        f'test "$(sha256sum /target/release/{binary} | cut -d " " -f 1)" = "{expected}" '
        f"|| {{ echo 'binary hash mismatch' >&2; exit 97; }}; "
        f"exec {' '.join(VALGRIND)} --callgrind-out-file=/out/{name}.cg /target/release/{binary} {' '.join(args)}"
    )
    return [
        "docker", "run", "--rm", "--network", "none",
        "-v", f"{variant['volume']}:/target:ro", "-v", f"{raw}:/out",
        image["id"], "sh", "-c", script,
    ]


def workload_command(workload, runtime, n):
    if workload in ca.ROUND_TRIP:
        return "counts", [workload, runtime, str(n)]
    return "codec-counts", [workload, str(n)]


def run_matrix(manifest, out, save):
    raw = out / "raw"
    raw.mkdir(exist_ok=True)
    image = manifest["image"]
    order = list(manifest["order"])
    for workload in manifest["workloads"]:
        runtimes = manifest["runtimes"] if workload in ca.ROUND_TRIP else ["none"]
        for runtime in runtimes:
            counts = {label: 0 for label in manifest["variants"]}
            for position, label in enumerate(order, 1):
                counts[label] += 1
                variant = manifest["variants"][label]
                for n in ca.lengths(workload):
                    name = f"{workload}-{runtime}-{position:02}-{label}{counts[label]}-{n}"
                    binary, args = workload_command(workload, runtime, n)
                    record = {
                        "name": name, "workload": workload, "runtime": runtime, "position": position,
                        "label": label, "ordinal": counts[label], "n": n, "started": now(),
                    }
                    manifest["runs"].append(record)
                    command = run_command(image, variant, raw, name, binary, args)
                    with open(raw / f"{name}.out", "w") as stdout, open(raw / f"{name}.err", "w") as stderr:
                        status = subprocess.run(command, stdout=stdout, stderr=stderr).returncode
                    record.update(exit_code=status, ended=now())
                    if status:
                        save()
                        raise RuntimeError(f"{name} exited {status}; see {raw / (name + '.err')} (no retry)")
                    result = ca.parse_result((raw / f"{name}.err").read_text())
                    if (result["workload"], result["runtime"], result["n"]) != (workload, runtime, n):
                        raise RuntimeError(f"{name}: RESULT line doesn't match the run")
                    cg_text = (raw / f"{name}.cg").read_text()
                    _lines, functions, edges = ca.parse_callgrind(cg_text)
                    record.update(
                        instructions=ca.parse_summary(cg_text),
                        harness_ir=ca.harness_ir(functions, edges),
                        allocs=result["allocs"],
                        bytes=result["bytes"],
                    )
                    save()
                    log("RUN", name, record["instructions"], record["allocs"], record["bytes"])


def annotate(manifest, out):
    """callgrind_annotate's per-function self Ir for every run."""
    script = (
        "set -e; for f in /out/*.cg; do "
        'callgrind_annotate --auto=no --inclusive=no --threshold=100 "$f" > "${f%.cg}.sym.txt"; done'
    )
    sh(["docker", "run", "--rm", "--network", "none", "-v", f"{out / 'raw'}:/out", manifest["image"]["id"],
        "sh", "-c", script])


# ---------------------------------------------------------------- analysis


def write_profiles(manifest, out, summary):
    """Per-symbol and per-line slopes, and the #1978 items, per workload."""
    runs = [r for r in manifest["runs"] if r.get("instructions") is not None]
    raw = out / "raw"
    profiles = out / "profiles"
    profiles.mkdir(exist_ok=True)
    items_md = ["# #1978 items by self instructions per request (variant A)", ""]
    items_json = {}
    for workload, runtime in sorted({(r["workload"], r["runtime"]) for r in runs}):
        selected = [r for r in runs if (r["workload"], r["runtime"]) == (workload, runtime)]
        symbols, lines, calls = {}, {}, {}
        for run in selected:
            sym_path = raw / f"{run['name']}.sym.txt"
            if sym_path.exists():
                symbols[run["name"]] = ca.parse_annotate(sym_path.read_text())
            cg_text = read_cg(raw, run["name"])
            if cg_text is not None:
                line_costs, _functions, edges = ca.parse_callgrind(cg_text)
                lines[run["name"]] = line_costs
                calls[run["name"]] = ca.calls_in(edges)
        other = "B" if "B" in manifest["variants"] else "P"
        per = {}
        for label in ("A", other):
            per[label] = {
                "symbols": ca.symbol_slopes(symbols, selected, workload, runtime, label),
                "lines": ca.line_slopes(lines, selected, workload, runtime, label),
                "calls": ca.symbol_slopes(calls, selected, workload, runtime, label),
            }
        unit = "request" if workload in ca.ROUND_TRIP else "iteration"
        md = [f"# {workload} / {runtime}: self instructions per {unit}", ""]
        md += [f"Median slope over the ordinals; the second column is {ca.LABELS[other]}.",
               "Calls in counts calls from other functions (a recursion level calling itself isn't one).", ""]
        sym_a, sym_b = per["A"]["symbols"], per[other]["symbols"]
        top = sorted(sym_a, key=lambda s: -abs(sym_a[s]))[:60]
        md += ["## Functions (callgrind_annotate, hash suffixes stripped)", ""]
        md.append(ca.table(["A", "B", "B - A", "Calls in (A)", "Function"], [
            [ca.fmt(sym_a[s]), ca.fmt(sym_b.get(s, 0)), ca.fmt(sym_b.get(s, 0) - sym_a[s]),
             f"{per['A']['calls'].get(s, 0):.2f}", f"`{s[:160]}`"] for s in top]))
        moved = sorted(set(sym_a) | set(sym_b), key=lambda s: -abs(sym_b.get(s, 0) - sym_a.get(s, 0)))[:30]
        md += ["", "## Largest function changes, B - A", ""]
        md.append(ca.table(["A", "B", "B - A", "Function"], [
            [ca.fmt(sym_a.get(s, 0)), ca.fmt(sym_b.get(s, 0)), ca.fmt(sym_b.get(s, 0) - sym_a.get(s, 0)),
             f"`{s[:160]}`"] for s in moved]))
        line_a, line_b = per["A"]["lines"], per[other]["lines"]
        top_lines = sorted(line_a, key=lambda k: -abs(line_a[k]))[:60]
        md += ["", "## Source lines (line tables; inlined code counts at its own line)", ""]
        md.append(ca.table(["A", "B", "B - A", "File:line"], [
            [ca.fmt(line_a[k]), ca.fmt(line_b.get(k, 0)), ca.fmt(line_b.get(k, 0) - line_a[k]),
             f"`{short_path(k[0])}:{k[1]}`"] for k in top_lines]))
        (profiles / f"{workload}-{runtime}.md").write_text("\n".join(md) + "\n")
        if workload in ca.ROUND_TRIP:
            items = ca.item_costs(sym_a, line_a)
            items_json[f"{workload}/{runtime}"] = items
            items_md += [f"## {workload} / {runtime}", ""]
            ranked = sorted(items.items(), key=lambda kv: -kv[1]["by_symbol"])
            items_md.append(ca.table(["Item", "By function", "By source file", "What"], [
                [item, ca.fmt(v["by_symbol"]), ca.fmt(v["by_file"]), v["title"]] for item, v in ranked]))
            for item, v in ranked:
                items_md += ["", f"Item {item}, largest functions:", ""]
                items_md += [f"- {ca.fmt(cost)}: `{symbol[:150]}`" for symbol, cost in v["top_symbols"]]
            items_md.append("")
    (out / "items.md").write_text("\n".join(items_md) + "\n")
    (out / "items.json").write_text(json.dumps(items_json, indent=2) + "\n")


def read_cg(raw, name):
    path = raw / f"{name}.cg"
    if path.exists():
        return path.read_text()
    if path.with_suffix(".cg.gz").exists():
        with gzip.open(path.with_suffix(".cg.gz"), "rt") as compressed:
            return compressed.read()
    return None


def short_path(path):
    if not path:
        return "???"
    for marker in ("/library/", "index.crates.io-", "/crates/"):
        if marker in path:
            return path[path.index(marker) + (1 if marker != "index.crates.io-" else 0):]
    return path


def summary_markdown(summary):
    variants = summary["variants"]
    lines = [f"# Counting harness: {summary['mode']} run", ""]
    for label, variant in variants.items():
        binaries = ", ".join(f"{k} `{v[:16]}`" for k, v in sorted((variant.get("binaries") or {}).items()))
        lines.append(f"- {ca.LABELS[label]}: `{variant['sha']}` ({variant['ref']}); binaries {binaries}")
    lines += [
        "",
        "Instructions are callgrind instruction reads per request (per iteration for the codec",
        "workloads), without the harness's counting allocator: the median over six ordinals of the",
        "slope between the short and the long run. Calls are allocator calls per request (gated);",
        "bytes are requested bytes per request (reported only).",
        "",
        ca.verdict_table(summary),
        "",
    ]
    if "baseline_builds_identical" in summary:
        lines.append(f"Baseline builds (A and A') byte-identical: {summary['baseline_builds_identical']}")
    if summary["mode"] != "noise":
        lines.append(f"Gates aimed at: {', '.join(summary['aimed'])}. Accepted: {summary['accepted']}")
        lines += [f"- Not acceptable as run: {problem}" for problem in summary["coverage_problems"]]
    return "\n".join(lines) + "\n"


def analyze(out, floors_paths=None, aim=("rp",)):
    manifest = ca.load_json(out / "manifest.json")
    if not manifest.get("completed"):
        raise RuntimeError("the matrix didn't complete; no verdict")
    floors = dict(manifest.get("recorded_floors") or {})
    for path in floors_paths or []:
        floors.update(ca.load_json(path)["noise_floors"])
    summary = ca.analyze(manifest, floors, aim)
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (out / "summary.md").write_text(summary_markdown(summary))
    write_profiles(manifest, out, summary)
    print(summary_markdown(summary))
    return summary


# ---------------------------------------------------------------- main


def expand_workloads(spec):
    out = []
    for part in spec.split(","):
        part = part.strip()
        if part == "all":
            out += list(ca.ROUND_TRIP) + list(ca.CODEC)
        elif part == "codec":
            out += list(ca.CODEC)
        elif part in ca.ROUND_TRIP or part in ca.CODEC:
            out.append(part)
        elif part:
            raise ValueError(f"unknown workload {part!r}")
    return list(dict.fromkeys(out))


def plan(args):
    """Mode, refs per label, matrix order and floors, checked before any build."""
    workloads = expand_workloads(args.workloads)
    runtimes = [r.strip() for r in args.runtimes.split(",") if r.strip()]
    bad = [r for r in runtimes if r not in ca.RUNTIMES]
    if bad or not runtimes:
        raise ValueError(f"runtimes must be ct and/or mt, not {args.runtimes!r}")
    aim = tuple(a.strip() for a in args.aim.split(",") if a.strip())
    floors = {}
    if args.noise:
        if args.baseline or args.candidate or args.noise_floors:
            raise ValueError("--noise takes no --baseline, --candidate or --noise-floors")
        refs, order = {"A": args.noise, "P": args.noise}, "APPA"
        mode = "noise"
    else:
        if not (args.baseline and args.candidate):
            raise ValueError("give --noise REF, or --baseline and --candidate")
        ca.validate_aim(aim, workloads)
        mode = "compare"
        if args.noise_floors:
            refs, order = {"A": args.baseline, "B": args.candidate}, "ABBA"
            for path in args.noise_floors:
                floors.update(ca.load_json(path)["noise_floors"])
            missing = [f"{w}/{r}" for w in workloads for r in (runtimes if w in ca.ROUND_TRIP else ["none"])
                       if f"{w}/{r}" not in floors]
            if missing:
                raise ValueError(f"no recorded floor for {', '.join(missing)}")
        else:
            refs, order = {"A": args.baseline, "P": args.baseline, "B": args.candidate}, "ABPPBA"
    return mode, refs, order * args.blocks, workloads, runtimes, aim, floors


def run(args):
    mode, refs, order, workloads, runtimes, aim, floors = plan(args)
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=False)
    run_id = uuid.uuid4().hex[:8]
    manifest = {
        "mode": mode,
        "started": now(),
        "workloads": workloads,
        "runtimes": runtimes,
        "order": order,
        "lengths": ca.LENGTHS,
        "recorded_floors": floors,
        "variants": {},
        "runs": [],
    }

    def save():
        (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

    registry = f"rb-perf-{run_id}-registry"
    volumes = [registry]
    try:
        manifest["image"] = ensure_image(args.image)
        for label, ref in refs.items():
            sha = resolve(ref)
            root = out / "trees" / label
            export_tree(sha, root)
            manifest["variants"][label] = {
                "label": label, "ref": ref, "sha": sha, "root": str(root), "volume": f"rb-perf-{run_id}-{label}",
            }
        harness = Path(args.harness).resolve() if args.harness else None
        if harness is None:
            last = list(manifest["variants"])[-1]
            candidate = Path(manifest["variants"][last]["root"]) / "benchmarks/counts"
            harness = candidate if (candidate / "Cargo.lock").exists() else REPO / "benchmarks/counts"
        staged = out / "harness"
        staged.mkdir()
        for name in HARNESS_FILES:
            (shutil.copytree if (harness / name).is_dir() else shutil.copy2)(harness / name, staged / name)
        manifest["harness"] = {"source": str(harness), "files": harness_hashes(staged)}
        for variant in manifest["variants"].values():
            if install_harness(staged, Path(variant["root"])) != manifest["harness"]["files"]:
                raise RuntimeError("harness copy differs")
        save()
        sh(["docker", "volume", "create", registry], stdout=subprocess.DEVNULL)
        for variant in manifest["variants"].values():
            refresh_lock(manifest["image"], variant, registry)
            log("BUILD", variant["label"], variant["sha"])
            volumes.append(variant["volume"])
            variant["binaries"] = build_variant(manifest["image"], variant, registry)
            save()
        run_matrix(manifest, out, save)
        manifest["completed"] = True
        manifest["ended"] = now()
        save()
        annotate(manifest, out)
        summary = analyze(out, None, aim)
        if args.gzip_raw:
            for path in (out / "raw").glob("*.cg"):
                with open(path, "rb") as src, gzip.open(f"{path}.gz", "wb") as dst:
                    shutil.copyfileobj(src, dst)
                path.unlink()
    except BaseException as error:
        manifest["failure"] = repr(error)
        save()
        raise
    finally:
        if not args.keep:
            subprocess.run(["docker", "volume", "rm", *volumes], capture_output=True)
            shutil.rmtree(out / "trees", ignore_errors=True)
    return summary


def exit_code(summary):
    """0 for a noise run or an accepted comparison, 1 otherwise."""
    return 0 if summary["mode"] == "noise" or summary["accepted"] else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    run_parser = commands.add_parser("run", help="build the trees and run the matrix")
    run_parser.add_argument("--baseline")
    run_parser.add_argument("--candidate")
    run_parser.add_argument("--noise", metavar="REF", help="A vs A': two clean builds of REF")
    run_parser.add_argument("--workloads", default="rp,wp,rpm", help="rp,wp,rpm, codec workload names, codec or all")
    run_parser.add_argument("--runtimes", default="ct,mt")
    run_parser.add_argument("--blocks", type=int, default=3, help="order blocks (default 3: six runs per build)")
    run_parser.add_argument("--out", required=True, help="a new directory for the outputs")
    run_parser.add_argument("--harness", help="benchmarks/counts directory to build (default: the candidate's)")
    run_parser.add_argument("--image", help="image tag to use or build (default: from the Dockerfile's hash)")
    run_parser.add_argument("--noise-floors", action="append",
                            help="quick mode, local only: skip the A' build and use this noise run's floors "
                                 "(summary.json; repeatable)")
    run_parser.add_argument("--aim", default="rp", help="workloads judged on the saving gate (default rp)")
    run_parser.add_argument("--gzip-raw", action="store_true", help="gzip the callgrind files afterwards")
    run_parser.add_argument("--keep", action="store_true", help="keep the build volumes and trees")
    analyze_parser = commands.add_parser("analyze", help="re-analyze a completed run")
    analyze_parser.add_argument("out")
    analyze_parser.add_argument("--aim", default="rp")
    args = parser.parse_args(argv)
    try:
        if args.command == "run":
            summary = run(args)
        else:
            aim = tuple(a.strip() for a in args.aim.split(",") if a.strip())
            summary = analyze(Path(args.out).resolve(), None, aim)
    except ValueError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    return exit_code(summary)


if __name__ == "__main__":
    sys.exit(main())

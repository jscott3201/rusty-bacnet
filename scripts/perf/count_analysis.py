#!/usr/bin/env python3
"""Analysis for the counting harness (benchmarks/counts/, Benchmarks.md
"Counting harness"): per-iteration slopes, the noise floor, the declared
gates and the per-symbol, per-line and per-item views.

perf_counts.py runs the matrix and calls `analyze`; this module has no
Docker or git calls, so test_count_analysis.py covers it with fixtures.

Tests: python3 -m unittest discover -s scripts/perf

Definitions (all per request, or per iteration for the codec workloads):

- slope: (Ir at the long run - Ir at the short run) / (long - short), for
  one ordinal (the i-th run of a variant in the matrix order). Setup,
  oracles and shutdown cancel out. The gated figure excludes the harness's
  counting allocator (its functions' own instructions and the atomic
  helpers they call); the figure with it is reported beside it.
- allocations: the counted loop's allocator calls and requested bytes at the
  long run, divided by its length (the harness's RESULT line).
- noise floor: the range (max - min) of the twelve slopes of two clean
  builds of the baseline (A and A') in the same matrix as the verdict.
  Counts differ between machines and sessions, so a floor from another run
  is used only when asked for (quick mode), and each row names its source.

Gates (#1979 decision, 2026-11-02), for a comparison (A, A' and B):

- aimed round trips (rp by default): the median slope falls by at least 100
  instructions and by more than 3 times the noise floor, and every A and B
  ordinal pair falls; an aimed codec workload has the same rule without the
  100-instruction minimum;
- every other row: the median slope rises by no more than the noise floor;
- every row: allocation calls per request rise by no more than 0.01;
  requested bytes are reported, not gated;
- acceptance needs every row to pass, rp, wp and rpm on both runtimes when
  any round trip runs, and only known, requested aimed workloads.
"""

import json
import re
import statistics
from pathlib import Path

ROUND_TRIP = ("rp", "wp", "rpm")
CODEC = (
    "encode_read_property_request",
    "decode_read_property_request",
    "encode_npdu_100b_payload",
    "decode_npdu_100b_payload",
    "encode_apdu_confirmed_request",
    "decode_apdu_confirmed_request",
    "full_stack_encode_rp",
)
# Loop lengths: the short and the long run of each ordinal.
LENGTHS = {"round_trip": (1_000, 10_000), "codec": (100_000, 1_000_000)}

MIN_SAVING = 100.0  # instructions per request
FLOOR_FACTOR = 3.0  # the saving must exceed this many noise floors
ALLOC_TOLERANCE = 0.01  # calls or bytes per request


def lengths(workload):
    return LENGTHS["round_trip" if workload in ROUND_TRIP else "codec"]


# ---------------------------------------------------------------- parsing


def parse_summary(cg_text):
    """The total Ir of a callgrind output file."""
    match = re.search(r"^(?:summary|totals): (\d+)", cg_text, re.M)
    if not match:
        raise ValueError("no summary or totals line in the callgrind output")
    return int(match[1])


RESULT_RE = re.compile(r"^RESULT workload=(\S+) runtime=(\S+) n=(\d+) warmup=(\d+) allocs=(\d+) bytes=(\d+)", re.M)


def parse_result(stderr_text):
    """The harness's RESULT line: workload, runtime, n, allocs, bytes."""
    match = RESULT_RE.search(stderr_text)
    if not match:
        raise ValueError("no RESULT line in the harness's stderr")
    return {
        "workload": match[1],
        "runtime": match[2],
        "n": int(match[3]),
        "warmup": int(match[4]),
        "allocs": int(match[5]),
        "bytes": int(match[6]),
    }


# A Rust legacy-mangling hash, a v0 crate disambiguator, and callgrind's
# recursion-depth suffix ('2): none of them name different code.
_HASH_RE = re.compile(r"::h[0-9a-f]{16}\b|\[[0-9a-f]{8,}\]|'\d+$")


def normalize_symbol(name):
    return _HASH_RE.sub("", name.strip())


_ANNOTATE_RE = re.compile(r"^\s*([\d,]+)\s+\(\s*[\d.]+%\)\s+(.*)$")
_OBJECT_RE = re.compile(r" \[(/[^\]]*|\?\?\?)\]$")


def parse_annotate(text):
    """Self Ir per function from `callgrind_annotate --inclusive=no`.

    Its rows are `Ir (pct)  file:function [object]`; one function can have
    rows for several files (code inlined from them), which are summed.
    Returns {normalized function: Ir}.
    """
    out = {}
    in_table = False
    for line in text.splitlines():
        if line.startswith("Ir") and "file:function" in line:
            in_table = True
            continue
        if not in_table:
            continue
        match = _ANNOTATE_RE.match(line)
        if not match:
            if line.startswith("-----") or not line.strip():
                if out:
                    # The table ends at the first rule or blank line after it.
                    break
                continue
            continue
        ir = int(match[1].replace(",", ""))
        rest = _OBJECT_RE.sub("", match[2])
        # file:function, split at the first single colon.
        split = re.match(r"(.*?)(?<!:):(?!:)(.*)$", rest)
        function = split[2] if split else rest
        key = normalize_symbol(function)
        out[key] = out.get(key, 0) + ir
    return out


def parse_callgrind(text):
    """Self Ir per source line and per function, and call edges, from a
    callgrind output file (positions: line).

    Returns (lines, functions, edges): lines is {(file, line): Ir}, functions
    {normalized function: Ir}, edges {(caller, callee): [calls, inclusive Ir]}.
    A call's inclusive cost (the line after `calls=`) isn't self cost.
    """
    names = {"fl": {}, "fn": {}, "ob": {}}
    lines, functions, edges = {}, {}, {}
    file = sub_file = function = callee = None
    last_line = 0
    pending_calls = None

    def name(kind, value):
        match = re.match(r"\((\d+)\)(?: (.*))?$", value)
        if not match:
            return value
        table = names[kind]
        if match[2] is not None:
            table[match[1]] = match[2]
        return table.get(match[1], value)

    def position(token):
        nonlocal last_line
        if token.startswith(("+", "-")):
            last_line += int(token)
        elif token == "*":
            pass
        else:
            last_line = int(token)
        return last_line

    for raw in text.splitlines():
        if not raw or raw[0] == "#":
            continue
        if "=" in raw.split(" ", 1)[0]:
            key, _, value = raw.partition("=")
            if key == "fl":
                file = sub_file = name("fl", value)
            elif key in ("fi", "fe"):
                sub_file = name("fl", value)
            elif key == "fn":
                function = normalize_symbol(name("fn", value))
                sub_file = file
            elif key in ("cfl", "cfi"):
                name("fl", value)
            elif key == "cfn":
                callee = normalize_symbol(name("fn", value))
            elif key == "calls":
                pending_calls = int(value.split()[0])
            elif key in ("ob", "cob"):
                name("ob", value)
            continue
        if not (raw[0].isdigit() or raw[0] in "+-*"):
            continue  # headers: version, events, summary, totals...
        parts = raw.split()
        line_no = position(parts[0])
        cost = int(parts[1]) if len(parts) > 1 else 0
        if pending_calls is not None:
            edge = edges.setdefault((function, callee), [0, 0])
            edge[0] += pending_calls
            edge[1] += cost
            pending_calls = None
            continue
        key = (sub_file, line_no)
        lines[key] = lines.get(key, 0) + cost
        functions[function] = functions.get(function, 0) + cost
    return lines, functions, edges


_HARNESS_RE = re.compile(r"bacnet_counts::CountingAlloc")


def harness_ir(functions, edges):
    """The harness's own counting cost in one run: the counting allocator's
    self Ir plus the outline atomic helpers (`__aarch64_*`) it calls. The
    system allocator it defers to isn't included: that cost is the
    program's."""
    own = sum(v for f, v in functions.items() if _HARNESS_RE.search(f))
    helpers = sum(inclusive for (caller, callee), (_calls, inclusive) in edges.items()
                  if _HARNESS_RE.search(caller) and callee.startswith("__aarch64_"))
    return own + helpers


def calls_in(edges):
    """Calls into each function from other functions; a recursion level
    calling itself ("free'2" into "free") isn't a call in."""
    out = {}
    for (caller, callee), (count, _inclusive) in edges.items():
        if caller != callee:
            out[callee] = out.get(callee, 0) + count
    return out


# ---------------------------------------------------------------- slopes

# Variant labels: A is the baseline, P a second clean build of it (A'), B the
# candidate. A noise run has A and P; a comparison has all three, so its
# noise floor comes from the same matrix as its verdict.
LABELS = {"A": "A", "P": "A'", "B": "B"}


def median(values):
    return statistics.median(values)


def describe(values):
    return {"median": median(values), "min": min(values), "max": max(values), "values": values}


def ordinal_runs(runs, workload, runtime, label):
    """{ordinal: {n: run}} for one variant."""
    out = {}
    for run in runs:
        if (run["workload"], run["runtime"], run["label"]) == (workload, runtime, label):
            out.setdefault(run["ordinal"], {})[run["n"]] = run
    return out


def slope(short, long, short_n, long_n):
    return (long - short) / (long_n - short_n)


def check_harness_ir(run):
    """A run that allocated must have its counting allocator's share: a
    renamed or inlined `CountingAlloc` would otherwise quietly turn the gate
    back into the raw count."""
    if run.get("harness_ir") is None or (run["allocs"] > 0 and run["harness_ir"] <= 0):
        raise ValueError(f"{run['name']}: {run['allocs']} allocations but no counting-allocator cost found")


def check_shape(manifest, runs):
    """Every workload and runtime has, per build, exactly as many ordinals as
    the matrix order gives it, and no run appears twice."""
    names = [r["name"] for r in runs]
    duplicates = sorted({n for n in names if names.count(n) > 1})
    if duplicates:
        raise ValueError(f"duplicate run records: {', '.join(duplicates)}")
    order = manifest.get("order") or ""
    expected = {label: order.count(label) for label in manifest["variants"]}
    if min(expected.values(), default=0) < 6:
        raise ValueError(f"the order {order!r} gives a build fewer than six runs")
    for workload, runtime in {(r["workload"], r["runtime"]) for r in runs}:
        for label, count in expected.items():
            ordinals = ordinal_runs(runs, workload, runtime, label)
            if sorted(ordinals) != list(range(1, count + 1)):
                raise ValueError(f"{workload}/{runtime}/{label}: ordinals {sorted(ordinals)}, expected 1 to {count}")


def variant_metrics(runs, workload, runtime, label):
    """Slopes and allocation rates of one variant's ordinals.

    `instructions` excludes the harness's own counting allocator (each run's
    `harness_ir`, from its profile); `instructions_with_harness` doesn't.
    """
    short_n, long_n = lengths(workload)
    by_ordinal = ordinal_runs(runs, workload, runtime, label)
    slopes, raw, calls, bytes_, call_slopes = [], [], [], [], []
    for ordinal in sorted(by_ordinal):
        pair = by_ordinal[ordinal]
        if short_n not in pair or long_n not in pair:
            raise ValueError(f"{workload}/{runtime}/{label}{ordinal}: missing a run length")
        short, long = pair[short_n], pair[long_n]
        for run in (short, long):
            check_harness_ir(run)
        raw.append(slope(short["instructions"], long["instructions"], short_n, long_n))
        slopes.append(slope(short["instructions"] - short["harness_ir"],
                            long["instructions"] - long["harness_ir"], short_n, long_n))
        calls.append(long["allocs"] / long_n)
        bytes_.append(long["bytes"] / long_n)
        call_slopes.append(slope(short["allocs"], long["allocs"], short_n, long_n))
    if not slopes:
        raise ValueError(f"{workload}/{runtime}/{label}: no runs")
    return {
        "instructions": describe(slopes),
        "instructions_with_harness": describe(raw),
        "alloc_calls": describe(calls),
        "alloc_bytes": describe(bytes_),
        "alloc_call_slopes": describe(call_slopes),
    }


def noise_floor(metrics_a, metrics_p):
    """The range of all slopes of the two builds of the baseline."""
    values = metrics_a["instructions"]["values"] + metrics_p["instructions"]["values"]
    return max(values) - min(values)


def validate_aim(aimed, workloads):
    """Refuses aimed workloads that aren't known or weren't requested."""
    known = set(ROUND_TRIP) | set(CODEC)
    unknown = [w for w in aimed if w not in known]
    if unknown:
        raise ValueError(f"unknown aimed workload(s): {', '.join(unknown)}")
    if not aimed:
        raise ValueError("no aimed workload")
    missing = [w for w in aimed if w not in workloads]
    if missing:
        raise ValueError(f"aimed workload(s) not in the run: {', '.join(missing)}")


def judge(workload, runtime, metrics_a, metrics_b, floor, floor_source, aimed):
    """One row of a comparison's verdict table.

    Round trips aimed at: a saving of at least MIN_SAVING and more than
    FLOOR_FACTOR floors, every pair lower. Codec workloads aimed at: the
    same without the 100-instruction minimum, which is a per-request bar.
    Others: no rise beyond the floor. All: allocation calls up by at most
    ALLOC_TOLERANCE. Requested bytes are reported, not gated.
    """
    a, b = metrics_a["instructions"], metrics_b["instructions"]
    delta = b["median"] - a["median"]
    pairs = [vb - va for va, vb in zip(a["values"], b["values"])]
    row = {
        "workload": workload,
        "runtime": runtime,
        "a_median": a["median"],
        "b_median": b["median"],
        "a_median_with_harness": metrics_a["instructions_with_harness"]["median"],
        "b_median_with_harness": metrics_b["instructions_with_harness"]["median"],
        "delta": delta,
        "pair_deltas": pairs,
        "noise_floor": floor,
        "noise_floor_source": floor_source,
        "a_calls": metrics_a["alloc_calls"]["median"],
        "b_calls": metrics_b["alloc_calls"]["median"],
        "a_bytes": metrics_a["alloc_bytes"]["median"],
        "b_bytes": metrics_b["alloc_bytes"]["median"],
    }
    saving = -delta
    every_pair_lower = bool(pairs) and all(p < 0 for p in pairs)
    if workload in aimed:
        minimum = MIN_SAVING if workload in ROUND_TRIP else 0.0
        instr_ok = saving >= minimum and saving > FLOOR_FACTOR * floor and every_pair_lower
        row["gate"] = f"saving >= {minimum:g} and > {FLOOR_FACTOR:g} x floor, every pair lower"
    else:
        instr_ok = delta <= floor
        row["gate"] = "not higher than the noise floor"
    row["instructions_ok"] = instr_ok
    row["allocations_ok"] = row["b_calls"] - row["a_calls"] <= ALLOC_TOLERANCE
    row["verdict"] = "PASS" if instr_ok and row["allocations_ok"] else "FAIL"
    return row


def coverage_problems(rows, aimed):
    """Why a comparison's rows can't support an acceptance, if they can't."""
    present = {(r["workload"], r["runtime"]) for r in rows}
    problems = []
    if any(w in ROUND_TRIP for w, _ in present):
        for workload in ROUND_TRIP:
            for runtime in RUNTIMES:
                if (workload, runtime) not in present:
                    problems.append(f"round trips need rp, wp and rpm on ct and mt; {workload}/{runtime} is missing")
    for workload in aimed:
        if not any(w == workload for w, _ in present):
            problems.append(f"aimed workload {workload} has no rows")
    return problems


RUNTIMES = ("ct", "mt")


def analyze(manifest, recorded_floors=None, aimed=("rp",)):
    """The summary of a completed matrix manifest.

    A noise run (A and A') reports floors. A comparison (A, A' and B) takes
    its floors from its own A and A'; a quick comparison without A' needs
    `recorded_floors` ({"workload/runtime": floor}) from an earlier noise
    run, and its rows say so.
    """
    if not manifest.get("completed"):
        raise ValueError("the matrix didn't complete; no verdict")
    noise = manifest["mode"] == "noise"
    runs = [r for r in manifest["runs"] if r.get("instructions") is not None]
    check_shape(manifest, runs)
    pairs = sorted({(r["workload"], r["runtime"]) for r in runs})
    if not noise:
        validate_aim(aimed, {w for w, _ in pairs})
    rows = []
    for workload, runtime in pairs:
        labels = sorted({r["label"] for r in runs if (r["workload"], r["runtime"]) == (workload, runtime)})
        metrics = {label: variant_metrics(runs, workload, runtime, label) for label in labels}
        key = f"{workload}/{runtime}"
        if "P" in metrics:
            floor, source = noise_floor(metrics["A"], metrics["P"]), "A vs A' in this matrix"
        elif recorded_floors and key in recorded_floors:
            floor, source = recorded_floors[key], "recorded noise run (quick mode)"
        else:
            raise ValueError(f"{key}: no A' build in the matrix and no recorded floor")
        if noise:
            row = {
                "workload": workload, "runtime": runtime, "noise_floor": floor, "noise_floor_source": source,
                "a_median": metrics["A"]["instructions"]["median"],
                "b_median": metrics["P"]["instructions"]["median"],
                "a_median_with_harness": metrics["A"]["instructions_with_harness"]["median"],
                "b_median_with_harness": metrics["P"]["instructions_with_harness"]["median"],
                "delta": metrics["P"]["instructions"]["median"] - metrics["A"]["instructions"]["median"],
                "pair_deltas": [p - a for a, p in zip(metrics["A"]["instructions"]["values"],
                                                      metrics["P"]["instructions"]["values"])],
                "a_calls": metrics["A"]["alloc_calls"]["median"], "b_calls": metrics["P"]["alloc_calls"]["median"],
                "a_bytes": metrics["A"]["alloc_bytes"]["median"], "b_bytes": metrics["P"]["alloc_bytes"]["median"],
                "gate": "none: measures the noise floor", "verdict": "floor",
            }
        else:
            row = judge(workload, runtime, metrics["A"], metrics["B"], floor, source, aimed)
        row["metrics"] = metrics
        rows.append(row)
    summary = {
        "mode": manifest["mode"],
        "variants": {label: {k: v for k, v in variant.items() if k != "root"}
                     for label, variant in manifest["variants"].items()},
        "rows": rows,
        "noise_floors": {f"{r['workload']}/{r['runtime']}": r["noise_floor"] for r in rows},
    }
    binaries = [manifest["variants"][label].get("binaries") for label in ("A", "P") if label in manifest["variants"]]
    if len(binaries) == 2:
        summary["baseline_builds_identical"] = binaries[0] == binaries[1]
    if not noise:
        problems = coverage_problems(rows, aimed)
        if summary.get("baseline_builds_identical") is False:
            problems.append("the baseline's two builds (A and A') differ, so the build isn't reproducible")
        summary["aimed"] = list(aimed)
        summary["coverage_problems"] = problems
        summary["accepted"] = not problems and bool(rows) and all(r["verdict"] == "PASS" for r in rows)
    return summary


# ---------------------------------------------------------------- profiles


def symbol_slopes(per_run_symbols, runs, workload, runtime, label):
    """Median self-Ir slope per symbol for one variant.

    `per_run_symbols` is {run name: {symbol: Ir}}. Returns {symbol: median}.
    """
    short_n, long_n = lengths(workload)
    by_ordinal = ordinal_runs(runs, workload, runtime, label)
    collected = {}
    for pair in by_ordinal.values():
        short = per_run_symbols.get(pair[short_n]["name"], {})
        long = per_run_symbols.get(pair[long_n]["name"], {})
        for symbol in set(short) | set(long):
            collected.setdefault(symbol, []).append(slope(short.get(symbol, 0), long.get(symbol, 0), short_n, long_n))
    count = len(by_ordinal)
    # A symbol missing from an ordinal's runs cost nothing there.
    return {symbol: median(values + [0.0] * (count - len(values))) for symbol, values in collected.items()}


def table(headers, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(" --- " for _ in headers) + "|"]
    out += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return "\n".join(out)


def fmt(value, places=1):
    return f"{value:,.{places}f}"


def verdict_table(summary):
    """Instructions exclude the harness's counting allocator; the floor's
    source is on every row. Requested bytes are reported, not gated."""
    other = "A'" if summary["mode"] == "noise" else "B"
    headers = ["Workload", "Runtime", "A median", f"{other} median", f"{other} - A", "Noise floor (source)",
               "Pairs lower", "Calls A / " + other, "Bytes A / " + other + " (report only)", "Verdict"]
    rows = []
    for row in summary["rows"]:
        lower = sum(1 for p in row["pair_deltas"] if p < 0)
        rows.append([
            row["workload"], row["runtime"], fmt(row["a_median"]), fmt(row["b_median"]), fmt(row["delta"]),
            f"{fmt(row['noise_floor'])} ({row['noise_floor_source']})", f"{lower} of {len(row['pair_deltas'])}",
            f"{row['a_calls']:.4f} / {row['b_calls']:.4f}", f"{row['a_bytes']:.2f} / {row['b_bytes']:.2f}",
            row["verdict"],
        ])
    return table(headers, rows)


# Items #1978 ranks for the S0 profile: regular expressions over normalized
# function names (symbols) and over source files (lines, which see inlined
# code). "Self" means instructions executed in the matching code itself.
ITEMS = {
    "2a": {
        "title": "Tokio lock acquire and release (Mutex, RwLock, Semaphore)",
        "symbols": r"tokio::sync::(batch_semaphore|mutex|rwlock)\b",
        "files": r"tokio-[\d.]+/src/sync/(batch_semaphore|mutex|rwlock)(/[a-z_]+)?\.rs$",
    },
    "6": {
        "title": "Clock reads (Instant::now, Timespec arithmetic, clock_gettime)",
        "symbols": r"(Timespec>::(now|sub_timespec)|std::time::Instant>::(now|duration_since|elapsed|checked_)"
        r"|^clock_gettime|__kernel_clock_gettime|tokio::time::instant::Instant>::(now|far_future)"
        r"|tokio::runtime::time::source::TimeSource>::(now|instant_to_tick))",
        "files": r"(library/std/src/sys/pal/unix/time\.rs|library/std/src/time\.rs|clock_gettime\.c"
        r"|tokio-[\d.]+/src/time/instant\.rs|tokio-[\d.]+/src/runtime/time/source\.rs)$",
    },
    "8": {
        "title": "Client dispatch loop select!: the loop and its idle branches (purge Interval, cleanup and "
        "network-control channels; the last is shared with the server's loop)",
        "symbols": r"(BACnetClient<[^>]*>>::start_with_options::\{closure#0\}::\{closure#0\}"
        r"|Interval>::poll_tick|Rx<bacnet_client::client::transaction_cleanup::TransactionCleanup"
        r"|Rx<bacnet_network::layer::ReceivedNetworkControl)",
        "files": r"(crates/bacnet-client/src/client/lifecycle\.rs|tokio-[\d.]+/src/time/interval\.rs)$",
    },
}


def item_costs(symbol_medians, line_medians):
    """Self Ir per request of each #1978 item, by symbol and by source file."""
    out = {}
    for item, spec in ITEMS.items():
        sym_re, file_re = re.compile(spec["symbols"]), re.compile(spec["files"])
        symbols = {s: v for s, v in symbol_medians.items() if sym_re.search(s)}
        files = {}
        for (file, _line), value in line_medians.items():
            if file and file_re.search(file):
                files[file] = files.get(file, 0.0) + value
        out[item] = {
            "title": spec["title"],
            "by_symbol": sum(symbols.values()),
            "by_file": sum(files.values()),
            "top_symbols": sorted(symbols.items(), key=lambda kv: -kv[1])[:8],
        }
    return out


def line_slopes(per_run_lines, runs, workload, runtime, label):
    """Median self-Ir slope per (file, line) for one variant."""
    return symbol_slopes(per_run_lines, runs, workload, runtime, label)


def load_json(path):
    return json.loads(Path(path).read_text())

#!/usr/bin/env python3
"""Analysis for the counting harness (benchmarks/counts/, Benchmarks.md
"Counting harness"): per-iteration slopes, the noise floor, the declared
gates and the per-symbol, per-line and per-item views.

perf_counts.py runs the matrix and calls `analyze`; this module has no
Docker or git calls, so test_count_analysis.py covers it with fixtures.

Definitions (all per request, or per iteration for the codec workloads):

- slope: (Ir at the long run - Ir at the short run) / (long - short), for
  one ordinal (the i-th run of a variant in the A B B A order). Setup,
  oracles and shutdown cancel out.
- allocations: the counted loop's allocator calls and requested bytes at the
  long run, divided by its length (the harness's RESULT line).
- noise floor: in an A vs A' run (two clean builds of one commit), the
  range (max - min) of all twelve slopes of a workload and runtime. In an A
  vs B run without a recorded floor, the range of A's six slopes.

Gates (#1979 decision, 2026-11-02), on every runtime:

- the aimed workloads (rp by default): the median slope falls by at least
  100 instructions and by more than 3 times the noise floor, and every
  ordinal pair falls;
- the other round-trip workloads: the median slope rises by no more than the
  noise floor;
- every round-trip workload: allocation calls and requested bytes per
  request rise by no more than 0.01;
- codec workloads are reported, not gated.
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


# ---------------------------------------------------------------- slopes


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


def variant_metrics(runs, workload, runtime, label):
    """Slopes and allocation rates of one variant's ordinals."""
    short_n, long_n = lengths(workload)
    by_ordinal = ordinal_runs(runs, workload, runtime, label)
    slopes, calls, bytes_, call_slopes = [], [], [], []
    for ordinal in sorted(by_ordinal):
        pair = by_ordinal[ordinal]
        if short_n not in pair or long_n not in pair:
            raise ValueError(f"{workload}/{runtime}/{label}{ordinal}: missing a run length")
        short, long = pair[short_n], pair[long_n]
        slopes.append(slope(short["instructions"], long["instructions"], short_n, long_n))
        calls.append(long["allocs"] / long_n)
        bytes_.append(long["bytes"] / long_n)
        call_slopes.append(slope(short["allocs"], long["allocs"], short_n, long_n))
    if not slopes:
        raise ValueError(f"{workload}/{runtime}/{label}: no runs")
    return {
        "instructions": describe(slopes),
        "alloc_calls": describe(calls),
        "alloc_bytes": describe(bytes_),
        "alloc_call_slopes": describe(call_slopes),
    }


def noise_floor(metrics_a, metrics_b):
    """A vs A': the range of all slopes of both builds."""
    values = metrics_a["instructions"]["values"] + metrics_b["instructions"]["values"]
    return max(values) - min(values)


def judge(workload, runtime, metrics_a, metrics_b, floor, floor_source, aimed, noise=False):
    """One row of the verdict table. A noise run (A vs A') has no verdict:
    it measures the floor the gates use."""
    a, b = metrics_a["instructions"], metrics_b["instructions"]
    delta = b["median"] - a["median"]
    pairs = [vb - va for va, vb in zip(a["values"], b["values"])]
    row = {
        "workload": workload,
        "runtime": runtime,
        "a_median": a["median"],
        "b_median": b["median"],
        "delta": delta,
        "pair_deltas": pairs,
        "noise_floor": floor,
        "noise_floor_source": floor_source,
        "a_calls": metrics_a["alloc_calls"]["median"],
        "b_calls": metrics_b["alloc_calls"]["median"],
        "a_bytes": metrics_a["alloc_bytes"]["median"],
        "b_bytes": metrics_b["alloc_bytes"]["median"],
    }
    if noise:
        row["gate"] = "none: measures the noise floor"
        row["verdict"] = "floor"
        return row
    if workload not in ROUND_TRIP:
        row["gate"] = "report only"
        row["verdict"] = "info"
        return row
    calls_ok = row["b_calls"] - row["a_calls"] <= ALLOC_TOLERANCE
    bytes_ok = row["b_bytes"] - row["a_bytes"] <= ALLOC_TOLERANCE
    if workload in aimed:
        saving = -delta
        instr_ok = saving >= MIN_SAVING and saving > FLOOR_FACTOR * floor and all(p < 0 for p in pairs)
        row["gate"] = f"saving >= {MIN_SAVING:g} and > {FLOOR_FACTOR:g} x floor, every pair lower"
    else:
        instr_ok = delta <= floor
        row["gate"] = "not higher than the noise floor"
    row["instructions_ok"] = instr_ok
    row["allocations_ok"] = calls_ok and bytes_ok
    row["verdict"] = "PASS" if instr_ok and calls_ok and bytes_ok else "FAIL"
    return row


def analyze(manifest, recorded_floors=None, aimed=("rp",)):
    """The summary of a completed matrix manifest.

    `recorded_floors` is {"workload/runtime": floor} from earlier A vs A'
    summaries; a workload without one uses A's own range.
    """
    noise = manifest["mode"] == "noise"
    runs = [r for r in manifest["runs"] if r.get("instructions") is not None]
    rows = []
    for workload, runtime in sorted({(r["workload"], r["runtime"]) for r in runs}):
        metrics = {label: variant_metrics(runs, workload, runtime, label) for label in "AB"}
        key = f"{workload}/{runtime}"
        if noise:
            floor, source = noise_floor(metrics["A"], metrics["B"]), "A vs A' range"
        elif recorded_floors and key in recorded_floors:
            floor, source = recorded_floors[key], "recorded A vs A' run"
        else:
            values = metrics["A"]["instructions"]["values"]
            floor, source = max(values) - min(values), "A's own range"
        row = judge(workload, runtime, metrics["A"], metrics["B"], floor, source, aimed, noise)
        row["metrics"] = metrics
        rows.append(row)
    gated = [row for row in rows if row["verdict"] in ("PASS", "FAIL")]
    summary = {
        "mode": manifest["mode"],
        "variants": {label: {k: v for k, v in manifest["variants"][label].items() if k != "root"} for label in "AB"},
        "aimed": list(aimed),
        "rows": rows,
        "noise_floors": {f"{r['workload']}/{r['runtime']}": r["noise_floor"] for r in rows},
    }
    if noise:
        hashes = [manifest["variants"][label].get("binaries") for label in "AB"]
        summary["binaries_identical"] = hashes[0] == hashes[1]
    else:
        summary["accepted"] = bool(gated) and all(row["verdict"] == "PASS" for row in gated)
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
    headers = [
        "Workload",
        "Runtime",
        "A median",
        "B median",
        "B - A",
        "Noise floor",
        "Pairs lower",
        "Calls A / B",
        "Bytes A / B",
        "Verdict",
    ]
    rows = []
    for row in summary["rows"]:
        lower = sum(1 for p in row["pair_deltas"] if p < 0)
        rows.append(
            [
                row["workload"],
                row["runtime"],
                fmt(row["a_median"]),
                fmt(row["b_median"]),
                fmt(row["delta"]),
                fmt(row["noise_floor"]),
                f"{lower} of {len(row['pair_deltas'])}",
                f"{row['a_calls']:.4f} / {row['b_calls']:.4f}",
                f"{row['a_bytes']:.2f} / {row['b_bytes']:.2f}",
                row["verdict"],
            ]
        )
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

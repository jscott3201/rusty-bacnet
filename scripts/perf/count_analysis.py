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
  oracles and shutdown cancel out.
- the gated figure: the slope of the instructions without the harness's
  counting allocator (its functions' own instructions and the atomic
  helpers they call) and without the system allocator's work (#2555),
  which is three parts (`allocator_record`):
  - the self cost of glibc's malloc, free and their internals, the
    functions ALLOCATOR_RE names;
  - the copy a moving realloc makes: the calls to memcpy or memmove whose
    caller is `_int_malloc` or `_int_realloc` (ALLOCATOR_COPY_CALLERS;
    callgrind books most of them to `_int_malloc`, which `_int_realloc`
    has just called). Not the calls out of the entry points (`malloc`,
    `free`, `realloc`): Rust reaches those with tail calls, so callgrind
    books the program's own copies after them to the entry point (about
    710 instructions per WriteProperty on `malloc -> __GI_memcpy`), and
    those stay gated. A run whose allocator-booked copies outnumber
    `_int_realloc`'s calls to `_int_malloc` (one per moving realloc) is
    refused, since some of them would then be the program's;
  - the outline atomic helpers (`__aarch64_*`, arm64 only) an allocator
    function calls in libc.so.6 itself, for its arena locks and fastbins.
    The binary's own copies of those helpers stay gated, even when
    callgrind books a call to them to `malloc` after a tail call.
  Each part is taken once: a function's self cost, or the inclusive cost
  of a call edge into a function that isn't in ALLOCATOR_RE. Every other
  memcpy, memmove and memset stays in the gated figure. Reported beside
  it: the allocator's slope, the total (the two together: the figure gated
  before #2555) and, in summary.json, the figure with the harness's share.
- allocations: the counted loop's allocator calls and requested bytes at the
  long run, divided by its length (the harness's RESULT line).
- noise floor: the range (max - min) of the twelve gated slopes of two clean
  builds of the baseline (A and A') in the same matrix as the verdict.
  Counts differ between machines and sessions, so a floor from another run
  is used only when asked for (quick mode), and each row names its source.
- hash-map keys (#2591): the harness fixes them from a seed in every run
  (its `hash_keys` module; perf_counts.py sets the seed), so the server's
  object map probes alike in every process. With per-process keys, as
  production has them, about one process in twelve paid 30 to 290 more
  instructions per request on ReadPropertyMultiple or WriteProperty, and
  whether one landed among the twelve set those rows' floors anywhere from
  6 to 300. Each run records the state its harness reported (`fixed`, or
  `random` / none from a harness before #2591); a row says which it had,
  and a comparison whose runs differ isn't accepted.

Gates (#1979 decision, 2026-11-02; the gated figure from #2555's,
2026-11-14), for a comparison (A, A' and B):

- aimed round trips (rp by default): the median gated slope falls by at
  least 100 instructions and by more than 3 times the noise floor, and every
  A and B ordinal pair falls; an aimed codec workload has the same rule
  without the 100-instruction minimum;
- every other row: the median gated slope rises by no more than the noise
  floor;
- every row: allocation calls per request rise by no more than 0.01;
  requested bytes are reported, not gated;
- the system allocator's instructions are reported, not gated (maintainer's
  decision on #2555, 2026-11-14). Which free chunk serves a request depends
  on where earlier allocations landed in the heap, which differs between
  builds whose request path is the same, and on ReadPropertyMultiple
  between runs of one binary too: a struct 16 bytes larger moved that row
  by about 200 instructions per request, all inside glibc, with equal
  allocation calls and bytes. Allocation calls stay gated, so allocating
  more still fails; a change that makes each allocation dearer without
  adding calls (a size crossing a bin) is reported only, with a note under
  the table when the allocator's change exceeds the total's own floor;
- acceptance needs every row to pass, rp, wp and rpm on both runtimes when
  any round trip runs, only known, requested aimed workloads, and the same
  hash-map keys in every run. A run of fewer round trips judges its rows
  but can't accept a change, however they come out.

The allocator's functions are glibc's, as the image scripts/perf/Dockerfile
pins names them: its valgrind package depends on libc6-dbg, which gives the
internal functions (`_int_malloc`, `unlink_chunk`) their names. Three
checks refuse a run rather than gate a figure that still holds the
allocator (a fourth, on the copies, is above):
- a run that allocated must show ALLOCATOR_REQUIRED: with another libc
  (musl, static or stripped) or a glibc without its debugging symbols,
  those names are missing;
- a function glibc's allocator sources define (ALLOCATOR_FILE_RE) must be
  in ALLOCATOR_RE: a newer glibc that renames or splits an internal stops
  here;
- a variant whose loop makes at least GUARD_MIN_CALLS allocation calls
  per request must spend at least MIN_ALLOCATOR_PER_CALL of the
  allocator's instructions per call (medians over its ordinals; glibc's
  own: 182 to 208 on the round trips). glibc's own functions run
  at start-up whatever serves the program, so the names alone don't show
  that the program's allocations reach glibc; a Rust global allocator of
  another kind (jemalloc, mimalloc) would leave the per-call figure near
  zero.
After a deliberate glibc bump, update ALLOCATOR_INTERNALS (and
ALLOCATOR_REQUIRED, if an internal it names was renamed) from the new
image's `nm -l` listing of libc6-dbg.

The no-regression check (`--aim none`, #2314) aims at nothing: every row
must stay within its floor and keep its allocation calls, for a change that
isn't an optimization. Each row reports its change in floors.

An override (`--allow rpm/ct:21`, the maintainer's decision on #2591,
2026-11-25) lets a row that isn't aimed at rise above its floor up to a
named bound, in instructions per request, for a rise that `perf_counts.py
attribute` has explained and the PR accepts without a code fix. A row not
named is judged as always; a named row that passed anyway reports its
override as unused; a rise past the bound still fails; allocation calls are
never covered. The summary lists each override with its bound, the rise
and the floor, and an accepted run that used one says so.

When a comparison fails, compare per-function call counts first
(`perf_counts.py attribute`): unchanged calls with changed instructions
point at codegen (inlining) rather than at more work per request.
"""

import json
import math
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
# What the noise floors and the instruction gates measure. A noise run's
# summary.json records it, a quick comparison's manifest records the one its
# floors are of, and both the plan and the analysis refuse floors of another
# figure: before #2555 they held the system allocator's spread, and before
# its fix round the block copies and atomic helpers the allocator runs.
GATED_FIGURE = ("instructions without the harness's counting allocator and the system allocator "
                "(its functions, realloc's block copies, libc's atomic helpers; #2555)")
# Allocator instructions per allocation call below which a loop's
# allocations can't be glibc's (its own run 182 to 208 per call on the round
# trips over 17 matrices, 2026-11-14), checked where the loop makes at least
# GUARD_MIN_CALLS allocation calls per request or iteration.
MIN_ALLOCATOR_PER_CALL = 10.0
GUARD_MIN_CALLS = 0.5


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
# The harness's hash-map keys (#2591), and with fixed keys one hash under
# the main thread's first keys; a harness before #2591 prints neither.
HASH_KEYS_RE = re.compile(r"^RESULT .*\bhash_keys=(fixed|random)\b", re.M)
HASH_CHECK_RE = re.compile(r"^RESULT .*\bhash_check=([0-9a-f]{16})\b", re.M)


def parse_result(stderr_text):
    """The harness's RESULT line: workload, runtime, n, allocs, bytes, the
    hash-map keys (`fixed`, `random`, or None from a harness before #2591)
    and, with fixed keys, their check value."""
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
        "hash_keys": keys[1] if (keys := HASH_KEYS_RE.search(stderr_text)) else None,
        "hash_check": check[1] if (check := HASH_CHECK_RE.search(stderr_text)) else None,
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


def parse_callgrind(text, files=None, object_edges=None):
    """Self Ir per source line and per function, and call edges, from a
    callgrind output file (positions: line).

    Returns (lines, functions, edges): lines is {(file, line): Ir}, functions
    {normalized function: Ir}, edges {(caller, callee): [calls, inclusive Ir]}.
    A call's inclusive cost (the line after `calls=`) isn't self cost. A
    `files` dict, when given, gets {normalized function: the source file
    callgrind names for it}; an `object_edges` dict gets the edges keyed by
    the callee's object too, {(caller, callee, callee object): [calls,
    inclusive Ir]}. Callgrind writes `cob=` before a call only when the
    callee's object isn't the caller's, so a call without one is into the
    caller's object (`ob=`).
    """
    names = {"fl": {}, "fn": {}, "ob": {}}
    lines, functions, edges = {}, {}, {}
    file = sub_file = function = callee = None
    obj = callee_obj = None
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
                callee_obj = None
                if files is not None:
                    files.setdefault(function, file)
            elif key in ("cfl", "cfi"):
                name("fl", value)
            elif key == "cfn":
                callee = normalize_symbol(name("fn", value))
            elif key == "calls":
                pending_calls = int(value.split()[0])
            elif key == "ob":
                obj = name("ob", value)
            elif key == "cob":
                callee_obj = name("ob", value)
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
            if object_edges is not None:
                edge = object_edges.setdefault((function, callee, callee_obj or obj), [0, 0])
                edge[0] += pending_calls
                edge[1] += cost
            pending_calls = None
            callee_obj = None
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


# The system allocator's functions (#2555): every function the image's
# glibc 2.39 debugging symbols (libc6-dbg, `nm -l`) place in its allocator
# sources, malloc/malloc.c and the arena.c, hooks.c and morecore.c it
# includes. A full matrix's 216 runs (2026-11-14) ran 28 of them: the entry
# points malloc, free, calloc, realloc and posix_memalign, and _mid_memalign,
# the _int_* internals, unlink_chunk, malloc_consolidate, sysmalloc,
# alloc_perturb, __glibc_morecore, the cache and arena set-up and the four
# tunable callbacks GLIBC_TUNABLES reaches. An entry point may carry one of
# glibc's alias prefixes (__libc_malloc), any name GCC's clone suffixes
# (.isra.0, .part.0, .constprop.0, and .cold with or without a number) or a
# symbol version. A function those sources define that the
# list doesn't name stops the run (ALLOCATOR_FILE_RE), so a newer glibc's
# renamed or split internals can't stay in the gated figure unnoticed.
# The libc utilities elsewhere in malloc/ (dynarrays, scratch buffers,
# thread and process freeres) call the allocator like any other code.
ALLOCATOR_FILE_RE = re.compile(r"/malloc/(?:malloc|arena|hooks|morecore)\.c$")
ALLOCATOR_ENTRY_POINTS = (
    "malloc", "free", "cfree", "calloc", "realloc", "memalign", "aligned_alloc", "posix_memalign", "valloc",
    "pvalloc", "malloc_usable_size", "malloc_trim", "malloc_info", "malloc_stats", "mallinfo", "mallinfo2",
    "mallopt",
)
ALLOCATOR_INTERNALS = (
    "_int_malloc", "_int_free", "_int_free_merge_chunk", "_int_free_create_chunk", "_int_free_maybe_consolidate",
    "_int_realloc", "_int_memalign", "_mid_memalign", "unlink_chunk", "malloc_consolidate", "sysmalloc",
    "sysmalloc_mmap", "sysmalloc_mmap_fallback", "systrim", "munmap_chunk", "alloc_perturb", "malloc_printerr",
    "int_mallinfo", "__glibc_morecore", "__default_morecore",
    # Per-thread caches and arenas (the multi-thread runtime's workers), fork.
    "ptmalloc_init", "tcache_init", "arena_get2", "arena_get_retry", "detach_arena", "new_heap", "alloc_new_heap",
    "__malloc_arena_thread_freeres", "__malloc_fork_lock_parent", "__malloc_fork_unlock_parent",
    "__malloc_fork_unlock_child",
    # The GLIBC_TUNABLES callbacks (start-up only).
    "_dl_tunable_set_mmap_threshold", "_dl_tunable_set_mmaps_max", "_dl_tunable_set_top_pad",
    "_dl_tunable_set_perturb_byte", "_dl_tunable_set_trim_threshold", "_dl_tunable_set_arena_max",
    "_dl_tunable_set_arena_test", "_dl_tunable_set_tcache_max", "_dl_tunable_set_tcache_count",
    "_dl_tunable_set_tcache_unsorted_limit", "_dl_tunable_set_mxfast", "_dl_tunable_set_hugetlb",
)
ALLOCATOR_RE = re.compile(
    r"^(?:(?:__GI___libc_|__libc_|__)?(?:" + "|".join(map(re.escape, ALLOCATOR_ENTRY_POINTS)) + ")"
    r"|(?:" + "|".join(map(re.escape, ALLOCATOR_INTERNALS)) + "))"
    r"(?:\.(?:isra|part|constprop)\.\d+|\.cold(?:\.\d+)?)*(?:@@?GLIBC_[\d.]+)?$")
# A run that allocated reaches these; missing, the allocator isn't glibc's
# with its symbols, and the run is refused.
ALLOCATOR_REQUIRED = ("malloc", "free", "_int_malloc", "_int_free")
# A moving realloc's copy: memcpy or memmove called by these (glibc's
# `_int_realloc` calls `_int_malloc` for the new block, then copies; most
# copies are booked to `_int_malloc`). The callee is matched by its base
# name's family, since the variant glibc's IFUNC picks differs per CPU:
# memcpy, memmove, glibc's internal aliases (`__GI_memcpy`) and the
# variants (`__memcpy_generic`, `__memcpy_simd` and the like on arm64;
# `__memmove_avx_unaligned_erms`, `__memcpy_sse2_unaligned` and the like
# on x86_64, whose memcpy is often its memmove). Not mempcpy or memset,
# which the allocator doesn't call for a move.
ALLOCATOR_COPY_CALLERS = ("_int_malloc", "_int_realloc")
COPY_RE = re.compile(r"^(?:__GI_)?(?:__)?mem(?:cpy|move)(?:_\w+)?(?:@@?GLIBC_[\d.]+)?$")
# The outline atomic helpers GCC's -moutline-atomics emits on arm64
# (`__aarch64_cas4_acq`, `__aarch64_swp8_acq`...): libc.so.6 links its own
# copies, and the program binary its own. x86_64 has none (its atomics are
# inline), so this part is zero there.
HELPER_PREFIX = "__aarch64_"
LIBC_RE = re.compile(r"(?:^|/)libc\.so\.6$")


def allocator_costs(functions):
    """The system allocator's self Ir per function in one run: {function:
    Ir} for the functions ALLOCATOR_RE names that ran. Self cost, so a
    function they call (memcpy from `_int_realloc`) isn't included."""
    return {f: v for f, v in functions.items() if v and ALLOCATOR_RE.match(f)}


def base_symbol(function):
    """A C function's name without GCC's clone suffix or a symbol version."""
    return re.sub(r"[.@].*$", "", function)


def allocator_copies(edges):
    """A moving realloc's copies in one run: (calls, inclusive Ir) of the
    memcpy and memmove calls `_int_malloc` and `_int_realloc` make, and the
    calls `_int_realloc` makes to `_int_malloc` (one per moving realloc, so
    no fewer than the copies)."""
    calls = cost = moves = 0
    for (caller, callee), (count, inclusive) in edges.items():
        caller_base = base_symbol(caller)
        if caller_base in ALLOCATOR_COPY_CALLERS and COPY_RE.match(callee):
            calls += count
            cost += inclusive
        elif caller_base == "_int_realloc" and base_symbol(callee) == "_int_malloc":
            moves += count
    return calls, cost, moves


def allocator_helpers(object_edges):
    """Inclusive Ir of the atomic helpers libc.so.6's own allocator
    functions call in libc.so.6 (the binary's copies of the helpers aren't
    the allocator's, whoever callgrind books the call to)."""
    return sum(inclusive for (caller, callee, callee_obj), (_count, inclusive) in object_edges.items()
               if callee.startswith(HELPER_PREFIX) and callee_obj and LIBC_RE.search(callee_obj)
               and ALLOCATOR_RE.match(caller))


# The allocator fields every run record carries; a run missing one was
# recorded before the figure it belongs to, and `perf_counts.fill_allocator`
# takes them all again from the run's callgrind file.
ALLOCATOR_FIELDS = ("allocator_ir", "allocator_self_ir", "allocator_copy_ir", "allocator_copy_calls",
                    "allocator_realloc_moves", "allocator_helpers_ir", "allocator_symbols", "allocator_unlisted")


def allocator_record(functions, files, edges, object_edges):
    """The run record's allocator fields: `allocator_ir`, the system
    allocator's share the gate leaves out (its functions' self Ir, a moving
    realloc's copies and libc's atomic helpers it calls, each also recorded
    alone), the copy calls and `_int_realloc`'s calls to `_int_malloc`, the
    functions counted (base names), and any function the allocator's
    sources define that ALLOCATOR_RE doesn't name. The arguments are
    `parse_callgrind`'s, with its `files` and `object_edges`."""
    costs = allocator_costs(functions)
    unlisted = sorted({f for f, path in files.items()
                       if path and ALLOCATOR_FILE_RE.search(path) and not ALLOCATOR_RE.match(f)})
    copy_calls, copy_ir, moves = allocator_copies(edges)
    helpers_ir = allocator_helpers(object_edges)
    own = sum(costs.values())
    return {
        "allocator_ir": own + copy_ir + helpers_ir,
        "allocator_self_ir": own,
        "allocator_copy_ir": copy_ir,
        "allocator_copy_calls": copy_calls,
        "allocator_realloc_moves": moves,
        "allocator_helpers_ir": helpers_ir,
        "allocator_symbols": sorted({base_symbol(f) for f in costs}),
        "allocator_unlisted": unlisted,
    }


def allocator_record_of(cg_text):
    """`allocator_record` of one callgrind file's text."""
    files, object_edges = {}, {}
    _lines, functions, edges = parse_callgrind(cg_text, files, object_edges)
    return allocator_record(functions, files, edges, object_edges)


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


GLIBC_BUMP = ("after a deliberate glibc bump, update ALLOCATOR_INTERNALS and ALLOCATOR_REQUIRED in "
              "scripts/perf/count_analysis.py from the new image's libc6-dbg symbols (`nm -l`)")


def check_allocator_ir(run):
    """A run that allocated must show the system allocator's entry points and
    internals, the allocator's sources no function the list misses, and no
    more allocator-booked copies than moving reallocs: otherwise the gated
    figure would quietly keep the allocator's cost (another libc, a static
    or stripped one, a newer glibc's internals) or lose the program's own
    copies."""
    missing_fields = [k for k in ALLOCATOR_FIELDS if run.get(k) is None]
    if missing_fields:
        raise ValueError(f"{run['name']}: no system-allocator figures recorded ({', '.join(missing_fields)}) and "
                         "no callgrind file in raw/ to take them from")
    if run["allocator_unlisted"]:
        raise ValueError(f"{run['name']}: glibc's allocator sources define functions ALLOCATOR_RE doesn't name "
                         f"({', '.join(run['allocator_unlisted'])}); if they are allocator code, add them to "
                         f"ALLOCATOR_INTERNALS; {GLIBC_BUMP}")
    found = set(run["allocator_symbols"])
    missing = [s for s in ALLOCATOR_REQUIRED if s not in found]
    if run["allocs"] > 0 and missing:
        raise ValueError(f"{run['name']}: {run['allocs']} allocations but no cost in the system allocator's "
                         f"{', '.join(missing)}: the gate needs glibc's allocator with its symbols, as in "
                         f"scripts/perf/Dockerfile's image; {GLIBC_BUMP}")
    if run["allocator_copy_calls"] > run["allocator_realloc_moves"]:
        raise ValueError(f"{run['name']}: callgrind books {run['allocator_copy_calls']} memcpy/memmove calls to "
                         f"{' and '.join(ALLOCATOR_COPY_CALLERS)} but _int_realloc calls _int_malloc only "
                         f"{run['allocator_realloc_moves']} times (once per moving realloc), so some of those "
                         "copies are the program's and the allocator's can't be told apart; look at the edges "
                         "into memcpy with callgrind_annotate --tree=caller before changing ALLOCATOR_COPY_CALLERS")


def check_allocator_per_call(where, allocator_slope, call_slope):
    """A loop that allocates must spend glibc-like allocator instructions on
    each call: glibc's functions run at start-up whatever allocator the
    program uses, so ALLOCATOR_REQUIRED alone doesn't show that the loop's
    allocations reach glibc. Medians of a variant's ordinals, per request."""
    if call_slope >= GUARD_MIN_CALLS and allocator_slope < MIN_ALLOCATOR_PER_CALL * call_slope:
        raise ValueError(f"{where}: {allocator_slope / call_slope:.1f} system-allocator instructions per allocation "
                         f"call ({call_slope:.2f} calls per request), under {MIN_ALLOCATOR_PER_CALL:g} (glibc's own: "
                         "182 to 208 on the round trips): the loop's allocations don't reach glibc's allocator "
                         "(another global allocator?), so the gated figure would still hold the allocator's cost; "
                         "the gate needs glibc's allocator, as in scripts/perf/Dockerfile's image")


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

    `instructions`, the gated figure, excludes the harness's own counting
    allocator (each run's `harness_ir`, from its profile) and the system
    allocator's self cost (`allocator_ir`); `allocator` is the latter's
    slope, `total` the two together (instructions without the harness's
    share), and `instructions_with_harness` the raw count's slope.
    """
    short_n, long_n = lengths(workload)
    by_ordinal = ordinal_runs(runs, workload, runtime, label)
    slopes, allocator, total, raw, calls, bytes_, call_slopes = [], [], [], [], [], [], []
    for ordinal in sorted(by_ordinal):
        pair = by_ordinal[ordinal]
        if short_n not in pair or long_n not in pair:
            raise ValueError(f"{workload}/{runtime}/{label}{ordinal}: missing a run length")
        short, long = pair[short_n], pair[long_n]
        for run in (short, long):
            check_harness_ir(run)
            check_allocator_ir(run)
        raw.append(slope(short["instructions"], long["instructions"], short_n, long_n))
        total.append(slope(short["instructions"] - short["harness_ir"],
                           long["instructions"] - long["harness_ir"], short_n, long_n))
        allocator.append(slope(short["allocator_ir"], long["allocator_ir"], short_n, long_n))
        slopes.append(total[-1] - allocator[-1])
        calls.append(long["allocs"] / long_n)
        bytes_.append(long["bytes"] / long_n)
        call_slopes.append(slope(short["allocs"], long["allocs"], short_n, long_n))
    if not slopes:
        raise ValueError(f"{workload}/{runtime}/{label}: no runs")
    check_allocator_per_call(f"{workload}/{runtime}/{label}", median(allocator), median(call_slopes))
    return {
        "instructions": describe(slopes),
        "allocator": describe(allocator),
        "total": describe(total),
        "instructions_with_harness": describe(raw),
        "alloc_calls": describe(calls),
        "alloc_bytes": describe(bytes_),
        "alloc_call_slopes": describe(call_slopes),
    }


def noise_floor(metrics_a, metrics_p, figure="instructions"):
    """The range of all slopes of the two builds of the baseline: of the
    gated figure, or of another one `figure` names (the total)."""
    values = metrics_a[figure]["values"] + metrics_p[figure]["values"]
    return max(values) - min(values)


def reported_figures(metrics_a, metrics_b):
    """The row fields beside the gated figure: the system allocator's and
    the total's medians and changes, and the figure with the harness's
    share."""
    a_alloc, b_alloc = metrics_a["allocator"]["median"], metrics_b["allocator"]["median"]
    a_total, b_total = metrics_a["total"]["median"], metrics_b["total"]["median"]
    return {
        "a_allocator": a_alloc, "b_allocator": b_alloc, "allocator_delta": b_alloc - a_alloc,
        "a_total": a_total, "b_total": b_total, "total_delta": b_total - a_total,
        "a_median_with_harness": metrics_a["instructions_with_harness"]["median"],
        "b_median_with_harness": metrics_b["instructions_with_harness"]["median"],
    }


# `--aim none`: the no-regression check, for a change that isn't an
# optimization. Every row must stay within its floor; no row has to fall.
NO_REGRESSION = "none"


def regression_only(aimed):
    return tuple(aimed) == (NO_REGRESSION,)


def validate_aim(aimed, workloads):
    """Refuses aimed workloads that aren't known or weren't requested, and
    `none` mixed with workload names."""
    if regression_only(aimed):
        return
    if NO_REGRESSION in aimed:
        raise ValueError("--aim none can't be combined with workload names")
    known = set(ROUND_TRIP) | set(CODEC)
    unknown = [w for w in aimed if w not in known]
    if unknown:
        raise ValueError(f"unknown aimed workload(s): {', '.join(unknown)}")
    if not aimed:
        raise ValueError("no aimed workload")
    missing = [w for w in aimed if w not in workloads]
    if missing:
        raise ValueError(f"aimed workload(s) not in the run: {', '.join(missing)}")


def parse_allow(spec, rows, aimed=()):
    """An override spec, "rpm/ct:21,rpm/mt:25", as {row: bound}. Each row
    must be one of `rows` (this run's "workload/runtime" keys) and not an
    aimed workload's, named once, with a finite bound above zero, in
    instructions per request (per iteration for a codec row)."""
    allow = {}
    for part in (spec or "").split(","):
        part = part.strip()
        if not part:
            continue
        row, colon, value = (x.strip() for x in part.partition(":"))
        if not colon or not row or not value:
            raise ValueError(f"--allow entry {part!r} isn't ROW:BOUND, such as rpm/ct:21")
        if row not in rows:
            raise ValueError(f"--allow names {row!r}, which isn't a row of this run ({', '.join(sorted(rows))})")
        if row.split("/")[0] in aimed and not regression_only(aimed):
            raise ValueError(f"--allow names {row!r}, an aimed row: an override covers a rise, not a saving")
        if row in allow:
            raise ValueError(f"--allow names {row!r} twice")
        try:
            bound = float(value)
        except ValueError:
            raise ValueError(f"--allow bound for {row} must be a number, not {value!r}") from None
        if not math.isfinite(bound) or bound <= 0:
            raise ValueError(f"--allow bound for {row} must be above zero, not {value!r}")
        allow[row] = bound
    return allow


def judge(workload, runtime, metrics_a, metrics_b, floor, floor_source, aimed, bound=None):
    """One row of a comparison's verdict table.

    Round trips aimed at: a saving of at least MIN_SAVING and more than
    FLOOR_FACTOR floors, every pair lower. Codec workloads aimed at: the
    same without the 100-instruction minimum, which is a per-request bar.
    Others: no rise beyond the floor, or with an override (`bound`) no rise
    beyond the bound. All: allocation calls up by at most ALLOC_TOLERANCE,
    which no override covers. Every instruction rule judges the gated figure; the
    system allocator's instructions and requested bytes are reported, not
    gated (#2555).
    """
    a, b = metrics_a["instructions"], metrics_b["instructions"]
    delta = b["median"] - a["median"]
    pairs = [vb - va for va, vb in zip(a["values"], b["values"])]
    row = {
        "workload": workload,
        "runtime": runtime,
        "a_median": a["median"],
        "b_median": b["median"],
        **reported_figures(metrics_a, metrics_b),
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
    if workload in aimed and not regression_only(aimed):
        minimum = MIN_SAVING if workload in ROUND_TRIP else 0.0
        instr_ok = saving >= minimum and saving > FLOOR_FACTOR * floor and every_pair_lower
        row["gate"] = f"saving >= {minimum:g} and > {FLOOR_FACTOR:g} x floor, every pair lower"
    else:
        # A floor under EXACT (the codec rows: a few instructions per whole
        # program) counts as EXACT, so +0.00003 per iteration isn't a rise.
        within = delta <= max(floor, EXACT)
        instr_ok = within or (bound is not None and delta <= bound)
        row["gate"] = "not higher than the noise floor"
        if bound is not None:
            row["override"] = {"bound": bound, "used": instr_ok and not within}
            row["gate"] += f", or with the override not higher than {bound:g}"
    row["instructions_ok"] = instr_ok
    row["allocations_ok"] = row["b_calls"] - row["a_calls"] <= ALLOC_TOLERANCE
    row["verdict"] = "PASS" if instr_ok and row["allocations_ok"] else "FAIL"
    return row


def coverage_problems(rows, aimed):
    """Why a comparison's rows can't support an acceptance, if they can't."""
    present = {(r["workload"], r["runtime"]) for r in rows}
    problems = []
    # The no-regression check is about the round trips: a codec-only run
    # can't show that nothing got worse.
    if regression_only(aimed) or any(w in ROUND_TRIP for w, _ in present):
        for workload in ROUND_TRIP:
            for runtime in RUNTIMES:
                if (workload, runtime) not in present:
                    problems.append(f"round trips need rp, wp and rpm on ct and mt; {workload}/{runtime} is missing")
    for workload in () if regression_only(aimed) else aimed:
        if not any(w == workload for w, _ in present):
            problems.append(f"aimed workload {workload} has no rows")
    return problems


RUNTIMES = ("ct", "mt")


def check_floors_figure(figure, source):
    """Recorded floors apply only to the figure they were measured on."""
    if figure != GATED_FIGURE:
        raise ValueError(f"{source} holds floors of another figure ({figure or 'the total, before #2555'}), "
                         f"not the gated one ({GATED_FIGURE}); measure them again with a noise run")


FIXED_KEYS, PER_PROCESS, MIXED_KEYS = "fixed", "per process", "mixed"


def hash_keys(runs):
    """How a row's runs drew their hash-map keys (#2591): FIXED_KEYS when the
    harness fixed them in every run, to the same keys (one check value),
    PER_PROCESS when none did (a harness before #2591, or a run without a
    seed), MIXED_KEYS otherwise."""
    states = {(FIXED_KEYS, r.get("hash_check")) if r.get("hash_keys") == "fixed" else (PER_PROCESS, None)
              for r in runs}
    return states.pop()[0] if len(states) == 1 else MIXED_KEYS


def allocator_notes(rows):
    """One line per row whose system-allocator share rose by more than the
    total's own A vs A' floor: a dearer allocation (a size moving to a
    larger bin, say) shows only in that column, since it isn't gated. A
    note, not a verdict; quick mode has no such floor and no notes."""
    notes = []
    for row in rows:
        floor = row.get("total_noise_floor")
        if floor is not None and row["allocator_delta"] > floor:
            notes.append(f"Note (reported, not gated): on {row['workload']}/{row['runtime']} the system allocator's "
                         f"instructions rose by {fmt(row['allocator_delta'])} per request, more than the total's "
                         f"noise floor ({fmt(floor)}); allocation calls {row['a_calls']:.4f} / "
                         f"{row['b_calls']:.4f}. Dearer allocations aren't gated; `perf_counts.py attribute` "
                         "lists the allocator's functions.")
    return notes


def analyze(manifest, recorded_floors=None, aimed=("rp",), allow=None):
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
        # The same checks as the plan's, for a re-analysis.
        parse_allow(",".join(f"{row}:{bound!r}" for row, bound in (allow or {}).items()),
                    {f"{w}/{r}" for w, r in pairs}, aimed)
    elif allow:
        raise ValueError("a noise run has no candidate, so --allow doesn't apply")
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
                **reported_figures(metrics["A"], metrics["P"]),
                "delta": metrics["P"]["instructions"]["median"] - metrics["A"]["instructions"]["median"],
                "pair_deltas": [p - a for a, p in zip(metrics["A"]["instructions"]["values"],
                                                      metrics["P"]["instructions"]["values"])],
                "a_calls": metrics["A"]["alloc_calls"]["median"], "b_calls": metrics["P"]["alloc_calls"]["median"],
                "a_bytes": metrics["A"]["alloc_bytes"]["median"], "b_bytes": metrics["P"]["alloc_bytes"]["median"],
                "gate": "none: measures the noise floor", "verdict": "floor",
            }
        else:
            row = judge(workload, runtime, metrics["A"], metrics["B"], floor, source, aimed, (allow or {}).get(key))
        # The total's own floor, for comparison with the gated figure's
        # (reported only; quick mode has no A' and no such floor).
        row["total_noise_floor"] = noise_floor(metrics["A"], metrics["P"], "total") if "P" in metrics else None
        row["hash_keys"] = hash_keys([r for r in runs if (r["workload"], r["runtime"]) == (workload, runtime)])
        row["metrics"] = metrics
        rows.append(row)
    summary = {
        "mode": manifest["mode"],
        "variants": {label: {k: v for k, v in variant.items() if k != "root"}
                     for label, variant in manifest["variants"].items()},
        "rows": rows,
        "allocator_notes": allocator_notes(rows),
        # Floors of the gated figure; quick mode refuses floors of another.
        "figure": GATED_FIGURE,
        "noise_floors": {f"{r['workload']}/{r['runtime']}": r["noise_floor"] for r in rows},
        # The run settings (allocator, hash seed) the floors hold for; quick
        # mode checks them.
        "environment": manifest.get("environment", {}),
        # Whether the harness fixed the hash-map keys, over every run.
        "hash_keys": hash_keys(runs),
    }
    binaries = [manifest["variants"][label].get("binaries") for label in ("A", "P") if label in manifest["variants"]]
    if len(binaries) == 2:
        summary["baseline_builds_identical"] = binaries[0] == binaries[1]
    if not noise:
        problems = coverage_problems(rows, aimed)
        problems += [f"on {r['workload']}/{r['runtime']} the runs' hash-map keys differ (fixed in some and not "
                     "in others, or fixed to different keys), so the builds didn't probe the object map alike"
                     for r in rows if r["hash_keys"] == MIXED_KEYS]
        if summary.get("baseline_builds_identical") is False:
            problems.append("the baseline's two builds (A and A') differ, so the build isn't reproducible")
        summary["aimed"] = list(aimed)
        summary["coverage_problems"] = problems
        summary["accepted"] = not problems and bool(rows) and all(r["verdict"] == "PASS" for r in rows)
        summary["overrides"] = [{"row": f"{r['workload']}/{r['runtime']}", **r["override"], "rise": r["delta"],
                                 "floor": r["noise_floor"], "verdict": r["verdict"]} for r in rows if "override" in r]
        summary["accepted_with_override"] = summary["accepted"] and any(o["used"] for o in summary["overrides"])
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


def change_over_floor(row):
    """The median change in noise floors: "+0.4 x" is within, "+2.1 x" above.
    Below EXACT instructions, a floor or a change counts as none (the codec
    runs differ by a few instructions per whole program)."""
    if abs(row["delta"]) < EXACT:
        return "0 x"
    if row["noise_floor"] >= EXACT:
        return f"{row['delta'] / row['noise_floor']:+.1f} x"
    return "+inf" if row["delta"] > 0 else "-inf"


EXACT = 0.01  # instructions per request or iteration


def function_changes(a, b, threshold=0.5):
    """Rows (change, a, b, function) of per-function values (self Ir or calls
    per request) that moved by at least `threshold`, largest first."""
    rows = [(b.get(k, 0.0) - a.get(k, 0.0), a.get(k, 0.0), b.get(k, 0.0), k) for k in set(a) | set(b)]
    return sorted((r for r in rows if abs(r[0]) >= threshold), key=lambda r: (-abs(r[0]), r[3]))


def verdict_table(summary):
    """One row per workload and runtime, the first two cells naming it.
    The gated figure excludes the harness's counting allocator and the
    system allocator; the allocator's figure and the total (the two
    together) follow, reported only, the total with its own floor where the
    matrix has A'. The floor's source is on every row. Requested bytes are
    reported, not gated."""
    other = "A'" if summary["mode"] == "noise" else "B"
    headers = ["Workload", "Runtime", "Gated A median", f"Gated {other} median", f"Gated {other} - A",
               "Noise floor (source)", "Change / floor", "Pairs lower", f"Allocator A / {other} (report only)",
               f"Allocator {other} - A (report only)", f"Total A / {other} (report only)",
               f"Total {other} - A, its floor (report only)", f"Calls A / {other}",
               f"Bytes A / {other} (report only)", "Verdict"]
    rows = []
    for row in summary["rows"]:
        lower = sum(1 for p in row["pair_deltas"] if p < 0)
        total_floor = row.get("total_noise_floor")
        rows.append([
            row["workload"], row["runtime"], fmt(row["a_median"]), fmt(row["b_median"]), fmt(row["delta"]),
            f"{fmt(row['noise_floor'])} ({row['noise_floor_source']}; hash keys {row.get('hash_keys', PER_PROCESS)})",
            change_over_floor(row),
            f"{lower} of {len(row['pair_deltas'])}",
            f"{fmt(row['a_allocator'])} / {fmt(row['b_allocator'])}", fmt(row["allocator_delta"]),
            f"{fmt(row['a_total'])} / {fmt(row['b_total'])}",
            fmt(row["total_delta"]) + (f" ({fmt(total_floor)})" if total_floor is not None else ""),
            f"{row['a_calls']:.4f} / {row['b_calls']:.4f}", f"{row['a_bytes']:.2f} / {row['b_bytes']:.2f}",
            row["verdict"] + (" (override)" if row.get("override", {}).get("used") else ""),
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


# Item 2a's semaphore cost split by who called it (#2314), from a profile run
# with callgrind's --separate-callers: each function name then carries its
# callers, nearest first, joined by "'". The nearest caller that matches a
# group decides; bounded mpsc channels return permits through the same
# semaphore, so they are a group of their own. The network layer's APDU
# receiver has the channel's receive inlined (no mpsc frame is left between
# it and the semaphore), so its frames count as the channel too.
SEMAPHORE_RE = re.compile(r"tokio::sync::batch_semaphore")
CALLER_GROUPS = (
    ("channel permits (bounded mpsc)",
     re.compile(r"tokio::sync::mpsc|bacnet_network::layer::ingress|ApduReceiver")),
    ("2a: client (transaction table and other client locks)", re.compile(r"bacnet_client")),
    ("2b: server (database and other server locks)", re.compile(r"bacnet_server|bacnet_device_core|bacnet_objects")),
)


def caller_frames(name):
    """A --separate-callers function name split into frames, nearest first;
    recursion-depth suffixes ('2) are dropped."""
    return [frame for frame in name.split("'") if frame and not frame.isdigit()]


def split_by_caller(functions):
    """Semaphore self Ir per caller group: {group: Ir}."""
    out = {}
    for name, cost in functions.items():
        frames = caller_frames(name)
        if not frames or not SEMAPHORE_RE.search(frames[0]):
            continue
        group = next((g for frame in frames[1:] for g, pattern in CALLER_GROUPS if pattern.search(frame)),
                     "other or deeper than the caller depth")
        out[group] = out.get(group, 0) + cost
    return out


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

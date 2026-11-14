#!/usr/bin/env python3
"""Tests for count_analysis.py: parsing, slopes, the noise floor and gates.

  python3 -m unittest discover -s scripts/perf
"""

import argparse
import contextlib
import copy
import io
import json
import re
import tempfile
import unittest
from pathlib import Path

import count_analysis as ca
from count_analysis import median
import perf_counts

DATA = Path(__file__).resolve().parent / "testdata"


ALLOCATOR = 1_500.0  # the system allocator's share per request in the fixtures


def runs_for(label, slopes, workload, runtime, calls=22.0, bytes_=2196.0, base=5_000_000, harness=42.0,
             allocator=ALLOCATOR):
    """Runs whose gated slopes are exactly `slopes`: each ordinal's short run
    costs `base + slope * short`, its long run `base + slope * long`.
    `harness` is the counting allocator's share per request and `allocator`
    the system allocator's (one value, or one per ordinal), added on top."""
    short, long = ca.lengths(workload)
    out = []
    for ordinal, slope in enumerate(slopes, 1):
        alloc = allocator[ordinal - 1] if isinstance(allocator, (list, tuple)) else allocator
        for n in (short, long):
            # The allocator's share: its functions, then 5% block copies and
            # 1% atomic helpers, one copy per moving realloc.
            share = round(200_000 + alloc * n)
            copy_ir, helpers_ir = share // 20, share // 100
            out.append({
                "name": f"{workload}-{runtime}-{label}{ordinal}-{n}",
                "workload": workload, "runtime": runtime, "label": label, "ordinal": ordinal, "n": n,
                "instructions": round(base + (slope + harness + alloc) * n), "harness_ir": round(harness * n),
                "allocator_ir": share, "allocator_self_ir": share - copy_ir - helpers_ir,
                "allocator_copy_ir": copy_ir, "allocator_copy_calls": n, "allocator_realloc_moves": n,
                "allocator_helpers_ir": helpers_ir, "allocator_symbols": sorted(ca.ALLOCATOR_REQUIRED),
                "allocator_unlisted": [],
                "allocs": round(calls * n), "bytes": round(bytes_ * n),
            })
    return out


def manifest(slopes_a, slopes_b, workload="rp", runtime="ct", calls=(22.0, 22.0), bytes_=(2196.0, 2196.0),
             mode="compare", slopes_p=None, cells=None, harness=42.0, allocator=(ALLOCATOR, ALLOCATOR)):
    """A completed manifest. A comparison has A, A' (P) and B; a noise run A
    and P (`slopes_b` are P's). `cells` lists extra (workload, runtime)
    pairs given the same slopes, for coverage. `allocator` is the system
    allocator's share of A (and A' in a comparison), then of B (of A' in a
    noise run)."""
    runs = []
    labels = ("A", "P") if mode == "noise" else ("A", "P", "B")
    for w, r in [(workload, runtime)] + list(cells or []):
        for label in labels:
            if label == "A":
                values, rate, byte_rate, alloc = slopes_a, calls[0], bytes_[0], allocator[0]
            elif label == "P":
                values = slopes_b if mode == "noise" else (slopes_p or slopes_a)
                rate, byte_rate = calls[0], bytes_[0]
                alloc = allocator[1] if mode == "noise" else allocator[0]
            else:
                values, rate, byte_rate, alloc = slopes_b, calls[1], bytes_[1], allocator[1]
            runs += runs_for(label, values, w, r, rate, byte_rate, harness=harness, allocator=alloc)
    variants = {label: {"sha": label * 40, "ref": label, "binaries": {"counts": "same"}} for label in labels}
    order = ("APPA" if mode == "noise" else "ABPPBA") * (len(slopes_a) // 2)
    return {"mode": mode, "variants": variants, "runs": runs, "completed": True, "order": order}


ALL_ROUND_TRIPS = [(w, r) for w in ca.ROUND_TRIP for r in ca.RUNTIMES if (w, r) != ("rp", "ct")]


def synthetic_cg(n):
    """A callgrind file of an rp/ct run of length n: per request, the
    program 300 instructions, the counting allocator 42, glibc's allocator
    functions 150, a moving realloc's copy 20 (booked to _int_malloc) and
    libc's atomic helper 6, on top of fixed set-up costs."""
    cost = {name: fixed + per * n for name, (fixed, per) in {
        "harness": (100, 42), "main": (5_000, 300), "malloc": (900, 40), "_int_malloc": (300, 45),
        "free": (200, 20), "_int_free": (100, 30), "_int_realloc": (0, 5), "malloc_consolidate": (0, 10),
        "copy": (0, 20), "helper": (0, 6)}.items()}
    total = sum(cost.values())
    return f"""# callgrind format
version: 1
positions: line
events: Ir
summary: {total}

ob=(1) /target/release/counts
fl=(1) src/lib.rs
fn=(1) <bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc
1 {cost['harness']}
fl=(2) src/main.rs
fn=(2) main
1 {cost['main']}
ob=(2) /usr/lib/aarch64-linux-gnu/libc.so.6
fl=(3) ./malloc/./malloc/malloc.c
fn=(3) malloc
1 {cost['malloc']}
fn=(4) _int_malloc
1 {cost['_int_malloc']}
cfl=(4) ./string/../sysdeps/aarch64/memcpy.S
cfn=(5) __GI_memcpy
calls={n} 1
2 {cost['copy']}
fn=(6) free
1 {cost['free']}
fn=(7) _int_free
1 {cost['_int_free']}
fn=(8) _int_realloc
1 {cost['_int_realloc']}
cfn=(4)
calls={n} 1
2 0
fn=(9) malloc_consolidate
1 {cost['malloc_consolidate']}
cfl=(5) ???
cfn=(10) __aarch64_swp8_acq
calls={n} 1
2 {cost['helper']}
fl=(4)
fn=(5)
1 {cost['copy']}
fl=(5)
fn=(10)
1 {cost['helper']}

totals: {total}
"""


def synthetic_matrix(out, order):
    """A completed rp/ct matrix in `order` whose runs were recorded before
    #2555 (no allocator fields), with their callgrind files in out/raw."""
    raw = out / "raw"
    raw.mkdir()
    runs, counts = [], {}
    for position, label in enumerate(order, 1):
        counts[label] = counts.get(label, 0) + 1
        for n in ca.lengths("rp"):
            name = f"rp-ct-{position:02}-{label}{counts[label]}-{n}"
            text = synthetic_cg(n)
            (raw / f"{name}.cg").write_text(text)
            runs.append({"name": name, "workload": "rp", "runtime": "ct", "position": position, "label": label,
                         "ordinal": counts[label], "n": n, "instructions": ca.parse_summary(text),
                         "harness_ir": 100 + 42 * n, "allocs": 2 * n, "bytes": 200 * n})
    variants = {label: {"label": label, "sha": label * 40, "ref": label, "binaries": {"counts": "same"}}
                for label in counts}
    return {"mode": "compare", "completed": True, "order": order, "workloads": ["rp"], "runtimes": ["ct"],
            "variants": variants, "runs": runs}


class Parsing(unittest.TestCase):
    def test_summary_and_result(self):
        text = (DATA / "small.cg").read_text()
        self.assertEqual(ca.parse_summary(text), 525)
        result = ca.parse_result("==7== noise\nRESULT workload=rp runtime=ct n=1000 warmup=200 allocs=22000 "
                                 "bytes=2196000 allocs_per_iter=22.000000 bytes_per_iter=2196.000000\n")
        self.assertEqual((result["workload"], result["n"], result["allocs"], result["bytes"]),
                         ("rp", 1000, 22000, 2196000))
        with self.assertRaises(ValueError):
            ca.parse_result("no result here")

    def test_callgrind_lines_functions_and_edges(self):
        lines, functions, edges = ca.parse_callgrind((DATA / "small.cg").read_text())
        # Self cost sums to the program total; the call's inclusive 1000 isn't self cost.
        self.assertEqual(sum(lines.values()), 525)
        self.assertEqual(lines[("src/a.rs", 10)], 100)
        self.assertEqual(lines[("src/inl.rs", 20)], 50)  # fi= switches the file
        self.assertEqual(lines[("src/a.rs", 11)], 25)  # fe= switches back
        self.assertEqual(lines[("src/a.rs", 32)], 25)  # "+2" and "*" positions
        self.assertEqual(functions, {"a::main": 175, "a::helper": 325, "clock_gettime@@GLIBC_2.17": 25})
        self.assertEqual(edges, {("a::main", "a::helper"): [3, 1000]})

    def test_annotate(self):
        symbols = ca.parse_annotate((DATA / "small.sym.txt").read_text())
        # Rows for one function from several files sum; hashes and '2 go.
        self.assertEqual(symbols["a::main"], 175)
        self.assertEqual(symbols["a::helper"], 325)
        self.assertEqual(symbols["0x000000000014f988"], 6)
        extend = "<smallvec::SmallVec<[u8; 6]> as core::iter::traits::collect::Extend<u8>>::extend::" \
                 "<core::iter::adapters::cloned::Cloned<core::slice::iter::Iter<u8>>>"
        self.assertEqual(symbols[extend], 1000)
        self.assertNotIn("PROGRAM TOTALS", symbols)

    # The fixture's allocator share (alloc.cg): its functions' self cost,
    # malloc 100 (not the 200 its call to _int_malloc costs), _int_malloc
    # 200 (its inlined header line too), unlink_chunk 30, free and free'2
    # 50, _int_free 45, _int_realloc 60, malloc_consolidate 12, the tunable
    # callback 5; the moving realloc's copies, booked to _int_malloc (25)
    # and _int_realloc (80); libc's atomic helper malloc_consolidate calls
    # (6). Not the allocator: the memcpy (30) and the binary's own helper
    # (8) callgrind books to malloc after a tail call, main's call to libc's
    # helper (4), the scratch-buffer utility (15), the counting allocator
    # (20).
    SELF, COPY, HELPERS = 100 + 200 + 30 + 50 + 45 + 60 + 12 + 5, 25 + 80, 6

    def record(self, text=None):
        return ca.allocator_record_of(text if text is not None else (DATA / "alloc.cg").read_text())

    def test_system_allocator_share(self):
        files, object_edges = {}, {}
        text = (DATA / "alloc.cg").read_text()
        _lines, functions, edges = ca.parse_callgrind(text, files, object_edges)
        self.assertEqual(sum(functions.values()), ca.parse_summary(text))
        self.assertEqual(files["malloc"], "./malloc/./malloc/malloc.c")
        self.assertEqual(files["main"], "src/main.rs")
        # A call without cob= is into the caller's own object, also right
        # after a call with one.
        libc, counts = "/usr/lib/aarch64-linux-gnu/libc.so.6", "/target/release/counts"
        self.assertEqual(object_edges[("malloc_consolidate", "__aarch64_swp8_acq", libc)], [2, 6])
        self.assertEqual(object_edges[("malloc", "__aarch64_ldadd8_relax", counts)], [1, 8])
        self.assertEqual(object_edges[("main", "__aarch64_swp8_acq", libc)], [1, 4])
        self.assertEqual(object_edges[("main", "__aarch64_ldadd8_relax", counts)], [1, 0])
        record = ca.allocator_record(functions, files, edges, object_edges)
        self.assertEqual(record, self.record(text))
        self.assertEqual(record["allocator_self_ir"], self.SELF)
        self.assertEqual(record["allocator_copy_ir"], self.COPY)
        self.assertEqual((record["allocator_copy_calls"], record["allocator_realloc_moves"]), (2, 2))
        self.assertEqual(record["allocator_helpers_ir"], self.HELPERS)
        self.assertEqual(record["allocator_ir"], self.SELF + self.COPY + self.HELPERS)
        self.assertEqual(record["allocator_symbols"], ["_dl_tunable_set_tcache_count", "_int_free", "_int_malloc",
                                                       "_int_realloc", "free", "malloc", "malloc_consolidate",
                                                       "unlink_chunk"])
        self.assertEqual(record["allocator_unlisted"], [])
        # The harness's share and the allocator's don't overlap, and the
        # gated rest is the program's: main, the binary's helper, the
        # memcpy booked to malloc, libc's helper main called, the utility.
        self.assertEqual(ca.harness_ir(functions, edges), 20)
        self.assertEqual(ca.parse_summary(text) - 20 - record["allocator_ir"], 285 + 8 + 30 + 4 + 15)

    def test_copies_booked_to_entry_points_stay_gated(self):
        # (a) Only _int_malloc's and _int_realloc's calls are the allocator's
        # copies: a memcpy booked to malloc, realloc or free after a tail call
        # is the program's, whatever its variant's name.
        edges = {("_int_malloc", "__GI_memcpy"): [3, 30], ("_int_realloc", "__memcpy_generic"): [1, 9],
                 ("_int_malloc", "memmove"): [1, 5], ("_int_realloc.isra.0", "__memmove_avx_unaligned_erms"): [1, 7],
                 ("_int_realloc", "_int_malloc"): [6, 600], ("malloc", "__GI_memcpy"): [9, 900],
                 ("realloc", "memcpy"): [1, 20], ("free", "__memcpy_generic"): [1, 50],
                 ("_int_malloc", "__GI_memset"): [1, 40], ("_int_realloc", "__mempcpy"): [1, 60],
                 ("_int_malloc", "_int_malloc"): [4, 400]}
        self.assertEqual(ca.allocator_copies(edges), (6, 51, 6))

    def test_x86_64_names_without_atomic_helpers(self):
        # x86_64: another memcpy variant name and libc path, and no outline
        # atomic helpers at all (the run still has its copies and passes).
        text = ((DATA / "alloc.cg").read_text().replace("aarch64-linux-gnu", "x86_64-linux-gnu")
                .replace("__memcpy_generic", "__memmove_avx_unaligned_erms")
                .replace("__GI_memcpy", "__memcpy_avx_unaligned_erms")
                .replace("__aarch64_swp8_acq", "__lll_lock_wait_private")
                .replace("__aarch64_ldadd8_relax", "core::sync::atomic::atomic_add"))
        record = self.record(text)
        self.assertEqual((record["allocator_copy_ir"], record["allocator_helpers_ir"]), (self.COPY, 0))
        self.assertEqual(record["allocator_ir"], self.SELF + self.COPY)
        ca.check_allocator_ir({"name": "x86", "allocs": 2, **record})

    def test_only_libc_helpers_called_by_the_allocator_are_its_own(self):
        # (b) The binary's own helper booked to malloc after a tail call, and
        # libc's helper called by the program, stay gated.
        libc = "/usr/lib/aarch64-linux-gnu/libc.so.6"
        object_edges = {("malloc_consolidate", "__aarch64_swp8_acq", libc): [2, 6],
                        ("_int_free", "__aarch64_cas4_acq", "/lib/libc.so.6"): [1, 5],
                        ("malloc", "__aarch64_ldadd8_relax", "/target/release/counts"): [1, 8],
                        ("main", "__aarch64_cas4_acq", libc): [1, 4],
                        ("malloc", "__GI_memcpy", libc): [1, 30]}
        self.assertEqual(ca.allocator_helpers(object_edges), 11)

    def test_more_allocator_copies_than_moving_reallocs_are_refused(self):
        text = (DATA / "alloc.cg").read_text().replace("calls=2 3800", "calls=1 3800")
        record = self.record(text)
        self.assertEqual((record["allocator_copy_calls"], record["allocator_realloc_moves"]), (2, 1))
        with self.assertRaisesRegex(ValueError, "can't be told apart"):
            ca.check_allocator_ir({"name": "rpm-ct", "allocs": 2, **record})
        ca.check_allocator_ir({"name": "rpm-ct", "allocs": 2, **self.record()})

    def test_unlisted_allocator_function_is_reported(self):
        # A newer glibc's split of an internal: defined in malloc.c, not listed.
        record = self.record((DATA / "alloc.cg").read_text().replace("unlink_chunk.isra.0", "unlink_chunk_v2.isra.0"))
        self.assertEqual(record["allocator_unlisted"], ["unlink_chunk_v2.isra.0"])
        self.assertEqual(record["allocator_self_ir"], self.SELF - 30)
        with self.assertRaisesRegex(ValueError, "ALLOCATOR_INTERNALS"):
            ca.check_allocator_ir({"name": "rpm-ct", "allocs": 2, **record})
        # ...and one in arena.c, which malloc.c includes.
        record = self.record((DATA / "alloc.cg").read_text().replace("_dl_tunable_set_tcache_count",
                                                                      "arena_get_retry_v2"))
        self.assertEqual(record["allocator_unlisted"], ["arena_get_retry_v2"])

    def test_allocator_names_take_clone_suffixes_and_versions_only(self):
        for name in ("malloc", "unlink_chunk.isra.0", "_int_free_maybe_consolidate.part.0", "free@@GLIBC_2.17",
                     "_mid_memalign.isra.0", "__libc_malloc", "__GI___libc_free", "__posix_memalign",
                     "cfree@GLIBC_2.17", "systrim.constprop.0", "_dl_tunable_set_tcache_count", "_int_free.cold",
                     "_int_malloc.cold.1", "sysmalloc.isra.0.cold"):
            self.assertTrue(ca.ALLOCATOR_RE.match(name), name)
        for name in ("memcpy", "__GI_memcpy", "mallocx", "<alloc::raw_vec::RawVec<u8>>::grow_one", "free_list",
                     "__libc_scratch_buffer_set_array_size", "__libc_thread_freeres", "__aarch64_swp8_acq",
                     "__libc__int_malloc", "<bacnet_counts::CountingAlloc as GlobalAlloc>::alloc", "malloc.coldx",
                     "free.cold1"):
            self.assertFalse(ca.ALLOCATOR_RE.match(name), name)
        for name in ("memcpy", "memmove", "__GI_memcpy", "__GI_memmove", "__memcpy_generic", "__memcpy_simd",
                     "__memmove_avx_unaligned_erms", "__memcpy_sse2_unaligned", "memcpy@@GLIBC_2.17"):
            self.assertTrue(ca.COPY_RE.match(name), name)
        for name in ("memset", "__GI_memset", "__mempcpy", "mempcpy", "wmemcpy", "<[u8]>::copy_from_slice"):
            self.assertFalse(ca.COPY_RE.match(name), name)

    def test_normalize(self):
        self.assertEqual(ca.normalize_symbol("tokio[1a2b3c4d5e6f7a8b]::spawn"), "tokio::spawn")
        self.assertEqual(ca.normalize_symbol("foo::bar::h0123456789abcdef"), "foo::bar")
        self.assertEqual(ca.normalize_symbol("free'2"), "free")
        self.assertEqual(ca.normalize_symbol("SmallVec<[u8; 6]>"), "SmallVec<[u8; 6]>")


class Slopes(unittest.TestCase):
    def test_known_slope(self):
        m = manifest([40_000.0] * 6, [39_800.0] * 6)
        metrics = ca.variant_metrics(m["runs"], "rp", "ct", "A")
        self.assertAlmostEqual(metrics["instructions"]["median"], 40_000.0, places=3)
        self.assertAlmostEqual(metrics["alloc_calls"]["median"], 22.0)
        self.assertAlmostEqual(metrics["alloc_call_slopes"]["median"], 22.0)

    def test_codec_lengths(self):
        m = manifest([219.0] * 6, [219.0] * 6, workload="decode_npdu_100b_payload", runtime="none")
        metrics = ca.variant_metrics(m["runs"], "decode_npdu_100b_payload", "none", "B")
        self.assertAlmostEqual(metrics["instructions"]["median"], 219.0)

    def test_missing_length_is_an_error(self):
        m = manifest([1.0] * 6, [1.0] * 6)
        m["runs"] = [r for r in m["runs"] if not (r["label"] == "A" and r["ordinal"] == 2 and r["n"] == 10_000)]
        with self.assertRaises(ValueError):
            ca.variant_metrics(m["runs"], "rp", "ct", "A")


class Gates(unittest.TestCase):
    A = [40_000.0, 40_004.0, 40_002.0, 39_998.0, 40_001.0, 40_003.0]  # range 6 with itself
    P = [40_005.0, 40_000.0, 40_001.0, 40_002.0, 40_003.0, 40_001.0]  # range of A and P: 7

    def summary(self, b, aimed=("rp",), full=True, **kwargs):
        cells = kwargs.pop("cells", ALL_ROUND_TRIPS if full else None)
        floors = kwargs.pop("floors", None)
        return ca.analyze(manifest(self.A, b, slopes_p=self.P, cells=cells, **kwargs), floors, aimed)

    def row(self, summary, workload="rp", runtime="ct"):
        return next(r for r in summary["rows"] if (r["workload"], r["runtime"]) == (workload, runtime))

    def test_noise_floor_is_the_range_of_both_builds(self):
        s = ca.analyze(manifest(self.A, [40_010.0] * 6, mode="noise"))
        self.assertAlmostEqual(s["rows"][0]["noise_floor"], 12.0, places=3)
        self.assertEqual(s["rows"][0]["verdict"], "floor")
        self.assertTrue(s["baseline_builds_identical"])
        self.assertNotIn("accepted", s)

    def test_comparison_floor_comes_from_its_own_a_prime(self):
        s = self.summary([x - 520 for x in self.A])
        row = self.row(s)
        self.assertAlmostEqual(row["noise_floor"], 7.0, places=3)
        self.assertEqual(row["noise_floor_source"], "A vs A' in this matrix")
        self.assertEqual(row["verdict"], "PASS")
        self.assertTrue(s["accepted"])

    def test_saving_under_100_fails(self):
        s = self.summary([x - 99 for x in self.A])
        self.assertEqual(self.row(s)["verdict"], "FAIL")
        self.assertFalse(s["accepted"])

    def test_saving_must_beat_three_floors(self):
        wide = [40_000.0, 40_060.0] + self.P[2:]  # floor 62: the bar is 186
        s = ca.analyze(manifest(self.A, [x - 150 for x in self.A], slopes_p=wide, cells=ALL_ROUND_TRIPS))
        self.assertEqual(self.row(s)["verdict"], "FAIL")
        s = ca.analyze(manifest(self.A, [x - 190 for x in self.A], slopes_p=wide, cells=ALL_ROUND_TRIPS))
        self.assertEqual(self.row(s)["verdict"], "PASS")

    def test_every_pair_must_fall(self):
        b = [x - 520 for x in self.A]
        b[3] = self.A[3] + 1
        s = self.summary(b)
        self.assertFalse(self.row(s)["instructions_ok"])
        self.assertFalse(s["accepted"])

    def test_allocation_calls_gate_and_bytes_report_only(self):
        b = [x - 520 for x in self.A]
        self.assertTrue(self.summary(b, calls=(22.0, 22.009))["accepted"])
        more = self.summary(b, calls=(22.0, 22.02))
        self.assertFalse(self.row(more)["allocations_ok"])
        self.assertFalse(more["accepted"])
        bytes_up = self.summary(b, bytes_=(2196.0, 2296.0))
        self.assertEqual(self.row(bytes_up)["verdict"], "PASS")
        self.assertTrue(bytes_up["accepted"])

    def test_harness_allocator_is_excluded_from_the_gate(self):
        s = self.summary([x - 520 for x in self.A], harness=545.0)
        row = self.row(s)
        self.assertAlmostEqual(row["a_median"], median(self.A), places=3)
        self.assertAlmostEqual(row["a_total"], median(self.A) + ALLOCATOR, places=3)
        self.assertAlmostEqual(row["a_median_with_harness"], median(self.A) + ALLOCATOR + 545.0, places=3)

    # The system allocator is reported, not gated (#2555).

    def test_allocator_only_move_passes_and_is_reported(self):
        # (a) A start-up allocation in another bin: +190 inside glibc only,
        # every B ordinal, with equal calls and bytes. The total (the old
        # gate) rises 27 floors; the gated figure doesn't move.
        s = self.summary(list(self.P), aimed=("none",), allocator=(ALLOCATOR, ALLOCATOR + 190.0))
        row = self.row(s)
        self.assertEqual(row["verdict"], "PASS")
        self.assertTrue(s["accepted"])
        self.assertAlmostEqual(row["allocator_delta"], 190.0, places=3)
        self.assertAlmostEqual(row["total_delta"], row["delta"] + 190.0, places=3)
        self.assertGreater(row["total_delta"], row["total_noise_floor"])
        self.assertAlmostEqual(row["b_allocator"], ALLOCATOR + 190.0, places=3)

    def test_non_allocator_rise_past_the_floor_fails(self):
        # (b) Eight more instructions outside the allocator, floor 7: fails,
        # also when the allocator falls by as much and the total is flat.
        for allocator in ((ALLOCATOR, ALLOCATOR), (ALLOCATOR, ALLOCATOR - 8.0)):
            s = self.summary([x + 8 for x in self.A], aimed=("none",), allocator=allocator)
            with self.subTest(allocator=allocator):
                row = self.row(s)
                self.assertEqual(row["verdict"], "FAIL")
                self.assertFalse(row["instructions_ok"])
                self.assertFalse(s["accepted"])
        self.assertAlmostEqual(row["total_delta"], 0.0, places=3)

    def test_allocation_calls_still_gate_with_flat_instructions(self):
        # (c) 0.02 more allocation calls per request, instructions flat.
        s = self.summary(list(self.P), aimed=("none",), calls=(22.0, 22.02))
        row = self.row(s)
        self.assertTrue(row["instructions_ok"])
        self.assertFalse(row["allocations_ok"])
        self.assertEqual(row["verdict"], "FAIL")
        self.assertFalse(s["accepted"])

    def test_aimed_saving_inside_the_allocator_is_no_saving(self):
        # (d) rp saves 520 per request, all of it inside glibc: no saving.
        s = self.summary(list(self.A), allocator=(ALLOCATOR, ALLOCATOR - 520.0))
        row = self.row(s)
        self.assertAlmostEqual(row["total_delta"], -520.0, places=3)
        self.assertAlmostEqual(row["delta"], 0.0, places=3)
        self.assertFalse(row["instructions_ok"])
        self.assertFalse(s["accepted"])
        # The same saving outside the allocator passes.
        self.assertTrue(self.summary([x - 520 for x in self.A])["accepted"])

    def test_noise_floor_leaves_out_the_allocator_spread(self):
        # A' spreads 60 inside the allocator only: the gated floor stays 7.
        spread = [ALLOCATOR, ALLOCATOR + 60.0] + [ALLOCATOR] * 4
        s = ca.analyze(manifest(self.A, list(self.P), mode="noise", allocator=(ALLOCATOR, spread)))
        row = s["rows"][0]
        self.assertAlmostEqual(row["noise_floor"], 7.0, places=3)
        # A' ordinal 2 at 40,000 + 60 against A ordinal 4 at 39,998.
        self.assertAlmostEqual(row["total_noise_floor"], 62.0, places=3)
        self.assertEqual(s["figure"], ca.GATED_FIGURE)
        self.assertIn("| Allocator A / A' (report only) | Allocator A' - A (report only) |", ca.verdict_table(s))

    def test_runs_without_allocator_symbols_are_refused(self):
        # (e) A run that allocated with none of glibc's allocator functions
        # (another libc, a static or stripped one), or with an allocator
        # function the list doesn't name, or recorded without the share.
        cases = {
            "no symbols": {"allocator_symbols": [], "allocator_ir": 0},
            "no internals": {"allocator_symbols": ["free", "malloc"]},
            "unlisted": {"allocator_unlisted": ["_int_free_chunk"]},
            "not recorded": {"allocator_ir": None},
            "recorded before the copies and helpers": {"allocator_copy_ir": None},
            "more copies than moving reallocs": {"allocator_copy_calls": 1_001},
        }
        for case, fields in cases.items():
            m = self.full()
            m["runs"][0].update(fields)
            with self.subTest(case=case), self.assertRaisesRegex(ValueError, "allocator"):
                ca.analyze(m, None, ("none",))
        quiet = self.full()
        for run in quiet["runs"]:
            run.update(allocs=0, bytes=0, allocator_symbols=[], allocator_ir=0)
        # A loop that doesn't allocate needs no allocator.
        self.assertIn("rows", ca.analyze(quiet, None, ("none",)))

    def test_table_rows_keep_their_first_cells_and_gain_the_columns(self):
        s = self.summary(list(self.P), aimed=("none",), allocator=(ALLOCATOR, ALLOCATOR + 190.0))
        text = ca.verdict_table(s)
        header = text.splitlines()[0]
        self.assertTrue(header.startswith("| Workload | Runtime | Gated A median | Gated B median | Gated B - A |"))
        for column in ("Allocator A / B (report only)", "Allocator B - A (report only)", "Total A / B (report only)",
                       "Total B - A, its floor (report only)"):
            self.assertIn(column, header)
        row = next(line for line in text.splitlines() if line.startswith("| rp | ct |"))
        self.assertIn("| 1,500.0 / 1,690.0 | 190.0 |", row)
        self.assertTrue(row.endswith("| PASS |"))
        # The lead reads the rows and the verdict with this; the notes and
        # the header don't match it.
        lead = re.compile(r"\| (rp|wp|rpm) \||Accepted")
        markdown = perf_counts.summary_markdown(s).splitlines()
        self.assertEqual(sum(1 for line in markdown if lead.search(line)), len(s["rows"]) + 1)
        self.assertTrue(any(line.startswith("Note (reported, not gated): on rp/ct") for line in markdown))

    def test_dearer_allocations_get_a_note_not_a_verdict(self):
        # +190 inside glibc on every row, the total's floor 7: one note per
        # row, all rows pass.
        s = self.summary(list(self.P), aimed=("none",), allocator=(ALLOCATOR, ALLOCATOR + 190.0))
        self.assertTrue(s["accepted"])
        self.assertEqual(len(s["allocator_notes"]), len(s["rows"]))
        self.assertIn("rose by 190.0 per request, more than the total's noise floor (7.0)", s["allocator_notes"][0])
        # Within the total's floor, or a fall: no note.
        for allocator in (ALLOCATOR + 7.0, ALLOCATOR - 190.0):
            with self.subTest(allocator=allocator):
                quiet = self.summary(list(self.P), aimed=("none",), allocator=(ALLOCATOR, allocator))
                self.assertEqual(quiet["allocator_notes"], [])

    def test_aimed_saving_is_judged_on_the_gated_figure(self):
        # rp saves 150 outside the allocator while the allocator rises 400:
        # the total rises 250, the aimed row still passes, and a note says
        # the allocator rose.
        s = self.summary([x - 150 for x in self.A], allocator=(ALLOCATOR, ALLOCATOR + 400.0))
        row = self.row(s)
        self.assertAlmostEqual(row["delta"], -150.0, places=3)
        self.assertAlmostEqual(row["total_delta"], 250.0, places=3)
        self.assertEqual(row["verdict"], "PASS")
        self.assertTrue(s["accepted"])
        self.assertTrue(any("on rp/ct" in note and "rose by 400.0" in note for note in s["allocator_notes"]))

    def test_loops_whose_allocations_skip_glibc_are_refused(self):
        # 22 calls per request at 4.5 allocator instructions each: another
        # global allocator serves the loop (glibc's ran at start-up only).
        with self.assertRaisesRegex(ValueError, "allocations don't reach glibc"):
            self.summary(list(self.P), aimed=("none",), allocator=(100.0, 100.0))
        # At 10 per call it passes; under GUARD_MIN_CALLS per request, or
        # with no allocations, there's nothing to judge.
        self.assertTrue(self.summary(list(self.P), aimed=("none",), allocator=(220.0, 220.0))["accepted"])
        self.assertTrue(self.summary(list(self.P), aimed=("none",), calls=(0.4, 0.4),
                                     allocator=(0.0, 0.0))["accepted"])

    def test_other_workloads_must_not_rise_beyond_the_floor(self):
        b = [x - 520 for x in self.A]
        rows = ca.analyze(manifest(self.A, b, slopes_p=self.P, cells=ALL_ROUND_TRIPS))["rows"]
        self.assertTrue(all(r["verdict"] == "PASS" for r in rows))
        above = ca.analyze(manifest(self.A, [x + 8 for x in self.A], workload="wp", slopes_p=self.P,
                                    cells=[(w, r) for w in ca.ROUND_TRIP for r in ca.RUNTIMES
                                           if (w, r) != ("wp", "ct")]), None, ("rp",))
        self.assertEqual(self.row(above, "wp")["verdict"], "FAIL")

    # B1: acceptance needs full coverage and a valid aim.

    def test_wp_only_run_aimed_at_rp_is_refused(self):
        with self.assertRaises(ValueError):
            ca.analyze(manifest(self.A, [x + 1 for x in self.A], workload="wp", slopes_p=self.P), None, ("rp",))

    def test_ct_only_run_is_not_accepted(self):
        s = self.summary([x - 520 for x in self.A], full=False)
        self.assertEqual(self.row(s)["verdict"], "PASS")
        self.assertFalse(s["accepted"])
        self.assertTrue(any("rp/mt" in p for p in s["coverage_problems"]))

    def test_unknown_aim_is_refused(self):
        with self.assertRaisesRegex(ValueError, "unknown aimed workload"):
            self.summary([x - 520 for x in self.A], aimed=("RP",))
        with self.assertRaisesRegex(ValueError, "unknown aimed workload"):
            ca.validate_aim(("RP",), {"rp", "RP"})
        with self.assertRaises(ValueError):
            self.summary([x - 520 for x in self.A], aimed=())

    def test_aimed_codec_workload_uses_its_own_rule(self):
        codec = "full_stack_encode_rp"
        a, p = [2367.0] * 6, [2367.0] * 6  # exact counts: floor 0
        saved = ca.analyze(manifest(a, [2300.0] * 6, workload=codec, runtime="none", slopes_p=p), None, (codec,))
        self.assertEqual(saved["rows"][0]["verdict"], "PASS")  # 67 saved: no 100 minimum
        self.assertTrue(saved["accepted"])
        same = ca.analyze(manifest(a, [2367.0] * 6, workload=codec, runtime="none", slopes_p=p), None, (codec,))
        self.assertFalse(same["accepted"])  # aimed but nothing saved

    def test_unaimed_codec_rise_fails(self):
        codec = "decode_npdu_100b_payload"
        s = ca.analyze(manifest([219.0] * 6, [220.0] * 6, workload=codec, runtime="none",
                                slopes_p=[219.0] * 6, cells=[("full_stack_encode_rp", "none")]),
                       None, ("full_stack_encode_rp",))
        self.assertEqual(self.row(s, codec, "none")["verdict"], "FAIL")
        self.assertFalse(s["accepted"])

    def test_quick_mode_needs_recorded_floors_and_says_so(self):
        m = manifest(self.A, [x - 520 for x in self.A], cells=ALL_ROUND_TRIPS)
        for variant in [r for r in m["runs"] if r["label"] == "P"]:
            m["runs"].remove(variant)
        del m["variants"]["P"]
        m["order"] = "ABBA" * 3
        with self.assertRaises(ValueError):
            ca.analyze(m)
        floors = {f"{w}/{r}": 60.0 for w in ca.ROUND_TRIP for r in ca.RUNTIMES}
        s = ca.analyze(m, floors)
        self.assertEqual(self.row(s)["noise_floor_source"], "recorded noise run (quick mode)")
        self.assertTrue(s["accepted"])

    # Matrix shape, reproducible baseline and the harness share (second review).

    def full(self):
        return manifest(self.A, [x - 520 for x in self.A], slopes_p=self.P, cells=ALL_ROUND_TRIPS)

    def test_short_or_uneven_matrices_are_refused(self):
        one_block = manifest(self.A[:2], [x - 520 for x in self.A[:2]], slopes_p=self.P[:2], cells=ALL_ROUND_TRIPS)
        with self.assertRaisesRegex(ValueError, "fewer than six"):
            ca.analyze(one_block)
        short_b = self.full()
        short_b["runs"] = [r for r in short_b["runs"] if not (r["label"] == "B" and r["ordinal"] > 2)]
        with self.assertRaisesRegex(ValueError, "ordinals"):
            ca.analyze(short_b)
        short_p = self.full()
        short_p["runs"] = [r for r in short_p["runs"] if not (r["label"] == "P" and r["ordinal"] > 1)]
        with self.assertRaisesRegex(ValueError, "ordinals"):
            ca.analyze(short_p)
        duplicate = self.full()
        extra = copy.deepcopy(next(r for r in duplicate["runs"] if r["label"] == "B"))
        extra["instructions"] -= 10_000_000
        duplicate["runs"].append(extra)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            ca.analyze(duplicate)
        self.assertTrue(ca.analyze(self.full())["accepted"])

    def test_differing_baseline_builds_are_not_accepted(self):
        m = self.full()
        m["variants"]["P"]["binaries"] = {"counts": "other"}
        s = ca.analyze(m)
        self.assertFalse(s["accepted"])
        self.assertTrue(any("A and A'" in p for p in s["coverage_problems"]))

    def test_missing_harness_share_is_refused(self):
        for value in (0, None):
            m = self.full()
            m["runs"][0]["harness_ir"] = value
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "counting-allocator"):
                ca.analyze(m)

    # The no-regression check (#2314).

    def test_no_regression_accepts_a_commit_against_itself(self):
        same = ca.analyze(manifest(self.A, list(self.P), slopes_p=self.P, cells=ALL_ROUND_TRIPS), None, ("none",))
        self.assertTrue(same["accepted"])
        self.assertTrue(all(r["gate"] == "not higher than the noise floor" for r in same["rows"]))
        self.assertIn("Change / floor", ca.verdict_table(same))

    def test_no_regression_refuses_a_forced_extra_loop(self):
        # A loop of 500 instructions per request added to every request.
        slower = ca.analyze(manifest(self.A, [x + 500 for x in self.A], slopes_p=self.P, cells=ALL_ROUND_TRIPS),
                            None, ("none",))
        self.assertFalse(slower["accepted"])
        self.assertEqual({r["verdict"] for r in slower["rows"]}, {"FAIL"})
        self.assertEqual(ca.change_over_floor(self.row(slower)), "+71.4 x")

    def test_no_regression_refuses_added_allocations_and_needs_coverage(self):
        more = ca.analyze(manifest(self.A, list(self.P), slopes_p=self.P, cells=ALL_ROUND_TRIPS,
                                   calls=(22.0, 23.0)), None, ("none",))
        self.assertFalse(more["accepted"])
        partial = ca.analyze(manifest(self.A, list(self.P), slopes_p=self.P), None, ("none",))
        self.assertFalse(partial["accepted"])
        with self.assertRaisesRegex(ValueError, "can't be combined"):
            ca.validate_aim(("none", "rp"), {"rp"})

    def test_change_over_floor_treats_tiny_values_as_none(self):
        self.assertEqual(ca.change_over_floor({"delta": -0.00001, "noise_floor": 0.00003}), "0 x")
        self.assertEqual(ca.change_over_floor({"delta": 3.0, "noise_floor": 0.00003}), "+inf")
        self.assertEqual(ca.change_over_floor({"delta": -5.0, "noise_floor": 10.0}), "-0.5 x")

    def test_no_regression_boundary_is_the_floor(self):
        # A and A' spread over 7 instructions: a rise of exactly 7 passes, 8 fails.
        at = ca.analyze(manifest(self.A, [x + 7 for x in self.A], slopes_p=self.P, cells=ALL_ROUND_TRIPS),
                        None, ("none",))
        self.assertTrue(at["accepted"])
        above = ca.analyze(manifest(self.A, [x + 8 for x in self.A], slopes_p=self.P, cells=ALL_ROUND_TRIPS),
                           None, ("none",))
        self.assertFalse(above["accepted"])

    def test_no_regression_needs_the_round_trips(self):
        codec = "full_stack_encode_rp"
        only = ca.analyze(manifest([1412.0] * 6, [1412.0] * 6, workload=codec, runtime="none",
                                   slopes_p=[1412.0] * 6), None, ("none",))
        self.assertFalse(only["accepted"])
        self.assertTrue(any("rp/ct" in p for p in only["coverage_problems"]))

    def test_exact_codec_rows_tolerate_whole_program_jitter(self):
        # A' equal to A (floor 0) and B 0.00003 per iteration higher.
        m = manifest([1412.0] * 6, [1412.00003] * 6, workload="full_stack_encode_rp", runtime="none",
                     slopes_p=[1412.0] * 6)
        rounds = manifest(self.A, list(self.P), slopes_p=self.P, cells=ALL_ROUND_TRIPS)
        m["runs"] += rounds["runs"]
        s = ca.analyze(m, None, ("none",))
        self.assertEqual(self.row(s, "full_stack_encode_rp", "none")["verdict"], "PASS")
        self.assertTrue(s["accepted"])
        jumped = copy.deepcopy(m)
        for run in jumped["runs"]:
            if run["workload"] == "full_stack_encode_rp" and run["label"] == "B" and run["n"] == 1_000_000:
                run["instructions"] += 900_000  # +1 per iteration
        self.assertFalse(ca.analyze(jumped, None, ("none",))["accepted"])

    def test_function_changes_rank_by_size(self):
        rows = ca.function_changes({"f": 10.0, "g": 5.0, "h": 1.0}, {"f": 10.2, "g": 9.0, "k": 2.0})
        self.assertEqual([r[3] for r in rows], ["g", "k", "h"])
        self.assertEqual(rows[0][:3], (4.0, 5.0, 9.0))

    def test_incomplete_matrix_has_no_verdict(self):
        m = manifest(self.A, [x - 520 for x in self.A])
        m["completed"] = False
        with self.assertRaises(ValueError):
            ca.analyze(m)
        broken = copy.deepcopy(m)
        broken["completed"] = True
        broken["runs"][0]["instructions"] = None
        with self.assertRaises(ValueError):
            ca.analyze(broken)


class Runner(unittest.TestCase):
    """perf_counts.py's checks before any build, and its exit status."""

    def args(self, **kwargs):
        base = dict(noise=None, baseline="dev", candidate="topic", workloads="rp,wp,rpm", runtimes="ct,mt",
                    aim="rp", noise_floors=None, blocks=3, glibc_tunables="")
        base.update(kwargs)
        return argparse.Namespace(**base)

    def test_comparison_builds_the_baseline_twice(self):
        mode, refs, order, *_ = perf_counts.plan(self.args())
        self.assertEqual((mode, refs), ("compare", {"A": "dev", "P": "dev", "B": "topic"}))
        self.assertEqual(order, "ABPPBA" * 3)
        self.assertEqual(perf_counts.plan(self.args(noise="dev", baseline=None, candidate=None))[2], "APPA" * 3)

    def test_bad_plans_are_refused(self):
        for kwargs in ({"blocks": 2}, {"aim": "RP"}, {"workloads": "wp", "aim": "rp"}, {"runtimes": "ct,xt"},
                       {"workloads": "rp,typo"}, {"candidate": None}):
            with self.subTest(**kwargs), self.assertRaises(ValueError):
                perf_counts.plan(self.args(**kwargs))

    def test_quick_mode_refuses_floors_of_another_figure(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "floors.json"
            # A noise run from before #2555: its floors hold the allocator's
            # spread; and one from before its fix round, whose floors hold
            # the block copies' and atomic helpers'.
            earlier = "instructions without the harness's counting allocator and the system allocator (#2555)"
            for recorded in ({}, {"figure": earlier}):
                path.write_text(json.dumps({**recorded, "noise_floors": {"rp/ct": 128.2, "rp/mt": 57.8}}))
                with self.subTest(recorded=recorded), self.assertRaisesRegex(ValueError, "another figure"):
                    perf_counts.plan(self.args(workloads="rp", noise_floors=[path]))

    def test_older_runs_take_the_allocator_share_from_their_callgrind_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw = Path(tmp)
            for name in ("rp-ct-01-A1-1000", "rp-ct-04-A2-1000"):
                (raw / f"{name}.cg").write_text((DATA / "alloc.cg").read_text())
            current = ca.allocator_record_of((DATA / "alloc.cg").read_text())
            m = {"runs": [{"name": "rp-ct-01-A1-1000", "instructions": 975},
                          {"name": "rp-ct-02-B1-1000", "instructions": 975},
                          {"name": "rp-ct-03-P1-1000", "instructions": 1, **current, "allocator_ir": 7},
                          # Recorded by #2555's first version: functions only.
                          {"name": "rp-ct-04-A2-1000", "instructions": 975, "allocator_ir": 502,
                           "allocator_symbols": ["malloc"], "allocator_unlisted": []}]}
            perf_counts.fill_allocator(m, raw)
            self.assertEqual(m["runs"][0]["allocator_ir"], 613)
            self.assertIn("_int_malloc", m["runs"][0]["allocator_symbols"])
            self.assertNotIn("allocator_ir", m["runs"][1])  # no file: the analysis refuses it
            self.assertEqual(m["runs"][2]["allocator_ir"], 7)  # recorded with every field: kept
            self.assertEqual(m["runs"][3]["allocator_ir"], 613)  # taken again, copies and helpers too
            self.assertEqual(m["runs"][3]["allocator_copy_ir"], 105)

    def test_analyze_round_trip_over_runs_recorded_before_the_allocator_figure(self):
        # A completed matrix recorded before #2555 (A and A' with no
        # allocator fields; B with #2555's first figure, the functions only):
        # `analyze` takes the share from the callgrind files.
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            m = synthetic_matrix(out, "ABPPBA" * 3)
            for run in m["runs"]:
                if run["label"] == "B":
                    run.update(allocator_ir=1, allocator_symbols=list(ca.ALLOCATOR_REQUIRED), allocator_unlisted=[])
            (out / "manifest.json").write_text(json.dumps(m))
            with contextlib.redirect_stdout(io.StringIO()):
                summary = perf_counts.analyze(out, ("none",))
            row = summary["rows"][0]
            # Per request: the program 300, the allocator 150 in its
            # functions, 20 in block copies and 6 in libc's helpers.
            self.assertAlmostEqual(row["a_median"], 300.0, places=6)
            self.assertAlmostEqual(row["b_median"], 300.0, places=6)
            self.assertAlmostEqual(row["a_allocator"], 176.0, places=6)
            self.assertAlmostEqual(row["b_allocator"], 176.0, places=6)
            self.assertAlmostEqual(row["total_delta"], 0.0, places=6)
            self.assertIn("| rp | ct |", (out / "summary.md").read_text())

    def test_analyze_checks_the_figure_of_recorded_floors(self):
        # A quick comparison's manifest: floors recorded with it are used
        # only when it says they are of the figure gated now.
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            m = synthetic_matrix(out, "ABBA" * 3)
            m["recorded_floors"] = {"rp/ct": 8.0}
            (out / "manifest.json").write_text(json.dumps(m))
            with self.assertRaisesRegex(ValueError, "another figure"):
                perf_counts.analyze(out, ("none",))
            m["recorded_floors_figure"] = ca.GATED_FIGURE
            (out / "manifest.json").write_text(json.dumps(m))
            with contextlib.redirect_stdout(io.StringIO()):
                summary = perf_counts.analyze(out, ("none",))
            self.assertEqual(summary["rows"][0]["noise_floor"], 8.0)
            self.assertEqual(summary["rows"][0]["noise_floor_source"], "recorded noise run (quick mode)")

    def test_attribution_lists_the_allocator_alone(self):
        self_a = {"malloc": 1133.0, "_int_malloc": 522.0, "unlink_chunk.isra.0": 100.0, "memcpy": 800.0}
        self_b = {"malloc": 1133.0, "_int_malloc": 652.0, "unlink_chunk.isra.0": 140.0, "memcpy": 800.0}
        text = "\n".join(perf_counts.allocator_attribution({"malloc": 21.0}, {"malloc": 21.0}, self_a, self_b,
                                                           "rpm", "ct", "B"))
        self.assertIn("change +170.0", text)
        self.assertIn("Counted as the system allocator: `_int_malloc`, `unlink_chunk.isra.0`, `malloc`", text)
        self.assertNotIn("memcpy", text)
        self.assertEqual(perf_counts.function_cell("free"), "`free` (allocator)")
        self.assertEqual(perf_counts.function_cell("memcpy"), "`memcpy`")
        # The block copies and atomic helpers count with it.
        copy, helpers = perf_counts.COPY_PART, perf_counts.HELPER_PART
        text = "\n".join(perf_counts.allocator_attribution(
            {"malloc": 21.0}, {"malloc": 21.0}, self_a, self_b, "rpm", "ct", "B",
            {copy: 9.0}, {copy: 195.0, helpers: 100.0}, {copy: 10.0}, {copy: 240.0, helpers: 80.0}))
        self.assertIn("change +195.0", text)  # 170 + 45 - 20
        self.assertIn(f"| 9.00 | 10.00 | 195.00 | 240.00 | +45.00 | {copy} |", text)
        self.assertIn(f"| - | - | 100.00 | 80.00 | -20.00 | {helpers} |", text)

    def test_quick_mode_needs_every_floor(self):
        floors = DATA / "floors-rp.json"
        mode, refs, order, *_rest, recorded = perf_counts.plan(self.args(workloads="rp", noise_floors=[floors]))
        self.assertEqual(set(refs), {"A", "B"})
        self.assertEqual(recorded["rp/ct"], 15.5)
        with self.assertRaises(ValueError):
            perf_counts.plan(self.args(workloads="rp,wp", noise_floors=[floors]))

    def test_lock_comparison_sees_only_third_party_packages(self):
        lock = ('[[package]]\nname = "bacnet-types"\nversion = "0.15.0"\n\n'
                '[[package]]\nname = "tokio"\nversion = "1.54.1"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n')
        bumped_path = lock.replace('0.15.0', '0.16.0')
        bumped_tokio = lock.replace('1.54.1', '1.55.0')
        self.assertEqual(perf_counts.third_party(lock), perf_counts.third_party(bumped_path))
        self.assertNotEqual(perf_counts.third_party(lock), perf_counts.third_party(bumped_tokio))

    def test_lock_refresh_may_drop_unused_packages_but_not_add_or_move(self):
        registry = 'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
        lock = ('[[package]]\nname = "nix"\nversion = "0.31.3"\n' + registry +
                '\n[[package]]\nname = "tokio"\nversion = "1.54.1"\n' + registry)
        dropped_nix = lock.split("\n[[package]]")[0].replace("nix", "bytes").replace("0.31.3", "1.11.0")
        added, dropped = perf_counts.lock_drift(lock, '[[package]]\nname = "tokio"\nversion = "1.54.1"\n' + registry)
        self.assertEqual((added, [d[:2] for d in dropped]), ([], [("nix", "0.31.3")]))
        added, dropped = perf_counts.lock_drift(lock, lock.replace("1.54.1", "1.55.0"))
        self.assertEqual(([a[:2] for a in added], dropped), ([("tokio", "1.55.0")], []))
        added, _dropped = perf_counts.lock_drift(lock, lock + "\n" + dropped_nix)
        self.assertEqual([a[:2] for a in added], [("bytes", "1.11.0")])

    def test_baseline_compat_names_a_setup_file(self):
        harness = Path(__file__).resolve().parents[2] / "benchmarks/counts"
        self.assertEqual(perf_counts.compat_setup(harness, "v013").name, "v013_setup.rs")
        with self.assertRaisesRegex(ValueError, "v013"):
            perf_counts.compat_setup(harness, "v012")

    def test_compat_setup_goes_to_the_baseline_trees_only(self):
        harness = Path(__file__).resolve().parents[2] / "benchmarks/counts"
        with tempfile.TemporaryDirectory() as tmp:
            manifest = {"variants": {}}
            for label in ("A", "P", "B"):
                src = Path(tmp) / label / "benchmarks/counts/src"
                src.mkdir(parents=True)
                (src / "setup.rs").write_text("// the harness's own setup\n")
                manifest["variants"][label] = {"root": str(Path(tmp) / label)}
            perf_counts.install_compat(harness, "v013", manifest)
            compat = (harness / "compat/v013_setup.rs").read_text()
            for label in ("A", "P"):
                setup = Path(tmp) / label / "benchmarks/counts/src/setup.rs"
                self.assertEqual(setup.read_text(), compat)
                self.assertEqual(manifest["variants"][label]["setup_compat"]["name"], "v013")
            candidate = Path(tmp) / "B" / "benchmarks/counts/src/setup.rs"
            self.assertEqual(candidate.read_text(), "// the harness's own setup\n")
            self.assertNotIn("setup_compat", manifest["variants"]["B"])

    def test_baseline_compat_is_for_codec_workloads_only(self):
        mode, *_ = perf_counts.plan(self.args(workloads="codec", aim="none", baseline_compat="v013"))
        self.assertEqual(mode, "compare")
        for workloads in ("rp", "codec,wp", "all"):
            with self.subTest(workloads=workloads), self.assertRaisesRegex(ValueError, "codec workloads only"):
                perf_counts.plan(self.args(workloads=workloads, aim="none", baseline_compat="v013"))

    def test_quick_mode_refuses_floors_from_other_allocator_settings(self):
        tuned = "glibc.malloc.tcache_count=32"
        with self.assertRaisesRegex(ValueError, "allocator settings"):
            perf_counts.plan(self.args(workloads="rp", noise_floors=[DATA / "floors-rp.json"], glibc_tunables=tuned))
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "floors.json"
            path.write_text(json.dumps({"environment": {"GLIBC_TUNABLES": tuned}, "figure": ca.GATED_FIGURE,
                                        "noise_floors": {"rp/ct": 11.4, "rp/mt": 14.8}}))
            self.assertEqual(perf_counts.plan(self.args(workloads="rp", noise_floors=[path],
                                                        glibc_tunables=tuned))[-1]["rp/ct"], 11.4)
            with self.assertRaisesRegex(ValueError, "allocator settings"):
                perf_counts.plan(self.args(workloads="rp", noise_floors=[path]))

    def test_exit_status(self):
        self.assertEqual(perf_counts.exit_code({"mode": "compare", "accepted": False}), 1)
        self.assertEqual(perf_counts.exit_code({"mode": "compare", "accepted": True}), 0)
        self.assertEqual(perf_counts.exit_code({"mode": "noise"}), 0)


class Callers(unittest.TestCase):
    def test_semaphore_cost_is_split_by_nearest_matching_caller(self):
        functions = {
            "<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked'<tokio::sync::batch_semaphore::Semaphore>"
            "::release'<tokio::sync::mutex::MutexGuard<bacnet_client::tsm::Tsm> as Drop>::drop"
            "'bacnet_client::client::requests::send": 100,
            "<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked'2'<tokio::sync::mpsc::bounded::Semaphore>"
            "::add_permit'bacnet_server::server::dispatch": 30,
            "<tokio::sync::batch_semaphore::Acquire as Future>::poll'<tokio::sync::rwlock::RwLock<"
            "bacnet_objects::database::ObjectDatabase>>::read'bacnet_server::server::read": 50,
            "<tokio::sync::batch_semaphore::Acquire as Future>::poll'tokio::runtime::task::raw::poll": 7,
            "malloc'bacnet_client::client::requests::send": 999,
        }
        split = ca.split_by_caller(functions)
        self.assertEqual(split["2a: client (transaction table and other client locks)"], 100)
        self.assertEqual(split["channel permits (bounded mpsc)"], 30)  # nearer than the server frame
        self.assertEqual(split["2b: server (database and other server locks)"], 50)
        self.assertEqual(split["other or deeper than the caller depth"], 7)
        self.assertEqual(sum(split.values()), 187)  # malloc isn't semaphore cost

    def test_inlined_channel_receive_counts_as_the_channel(self):
        # The bounded channel's receive is inlined into the network layer's
        # APDU receiver: no mpsc frame between it and the semaphore.
        functions = {
            "<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked'<bacnet_network::layer::ingress::"
            "ApduReceiver>::poll_recv'<bacnet_server::server::BACnetServer<B>>::run": 139,
            "<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked'<bacnet_network::layer::ingress::"
            "ApduReceiver>::poll_recv'<bacnet_client::client::BACnetClient<B>>::dispatch": 5,
        }
        self.assertEqual(ca.split_by_caller(functions), {"channel permits (bounded mpsc)": 144})


class Profiles(unittest.TestCase):
    def test_calls_in_skips_recursion_and_harness_cost(self):
        edges = {("free", "free"): [5, 50], ("a::main", "free"): [2, 20],
                 ("<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc", "__aarch64_ldadd8_relax"): [2, 10],
                 ("<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc", "malloc"): [1, 99]}
        self.assertEqual(ca.calls_in(edges)["free"], 2)
        functions = {"<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc": 27, "malloc": 99}
        self.assertEqual(ca.harness_ir(functions, edges), 37)  # self + atomic helpers, not malloc

    def test_symbol_slopes_and_items(self):
        m = manifest([1.0] * 2, [1.0] * 2)
        per_run = {}
        for run in m["runs"]:
            n = run["n"]
            per_run[run["name"]] = {
                "<std::sys::pal::unix::time::Timespec>::now": 800 * n + 50,
                "<tokio::sync::batch_semaphore::Semaphore>::add_permits_locked": 700 * n,
                "setup_only": 999,
            }
            if run["ordinal"] == 1:
                per_run[run["name"]]["rare"] = 10 * n
        slopes = ca.symbol_slopes(per_run, m["runs"], "rp", "ct", "A")
        self.assertAlmostEqual(slopes["<std::sys::pal::unix::time::Timespec>::now"], 800.0)
        self.assertAlmostEqual(slopes["setup_only"], 0.0)
        # Present in one of two ordinals: the other ordinal counts as zero.
        self.assertAlmostEqual(slopes["rare"], 5.0)
        lines = {("/x/tokio-1.54.1/src/sync/batch_semaphore.rs", 3): 600.0, ("library/std/src/time.rs", 9): 20.0}
        items = ca.item_costs(slopes, lines)
        self.assertAlmostEqual(items["2a"]["by_symbol"], 700.0)
        self.assertAlmostEqual(items["2a"]["by_file"], 600.0)
        self.assertAlmostEqual(items["6"]["by_symbol"], 800.0)
        self.assertAlmostEqual(items["6"]["by_file"], 20.0)
        self.assertEqual(items["8"]["by_symbol"], 0)


if __name__ == "__main__":
    unittest.main()

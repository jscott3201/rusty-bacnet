#!/usr/bin/env python3
"""Tests for count_analysis.py: parsing, slopes, the noise floor and gates.

  python3 -m unittest discover -s scripts/perf
"""

import argparse
import copy
import unittest
from pathlib import Path

import count_analysis as ca
from count_analysis import median
import perf_counts

DATA = Path(__file__).resolve().parent / "testdata"


def runs_for(label, slopes, workload, runtime, calls=22.0, bytes_=2196.0, base=5_000_000, harness=0.0):
    """Runs whose slopes are exactly `slopes`: each ordinal's short run costs
    `base + slope * short`, its long run `base + slope * long`. `harness` is
    the counting allocator's share per request, added on top."""
    short, long = ca.lengths(workload)
    out = []
    for ordinal, slope in enumerate(slopes, 1):
        for n in (short, long):
            out.append({
                "name": f"{workload}-{runtime}-{label}{ordinal}-{n}",
                "workload": workload, "runtime": runtime, "label": label, "ordinal": ordinal, "n": n,
                "instructions": round(base + (slope + harness) * n), "harness_ir": round(harness * n),
                "allocs": round(calls * n), "bytes": round(bytes_ * n),
            })
    return out


def manifest(slopes_a, slopes_b, workload="rp", runtime="ct", calls=(22.0, 22.0), bytes_=(2196.0, 2196.0),
             mode="compare", slopes_p=None, cells=None, harness=0.0):
    """A completed manifest. A comparison has A, A' (P) and B; a noise run A
    and P (`slopes_b` are P's). `cells` lists extra (workload, runtime)
    pairs given the same slopes, for coverage."""
    runs = []
    labels = ("A", "P") if mode == "noise" else ("A", "P", "B")
    for w, r in [(workload, runtime)] + list(cells or []):
        for label in labels:
            if label == "A":
                values, rate, byte_rate = slopes_a, calls[0], bytes_[0]
            elif label == "P":
                values = slopes_b if mode == "noise" else (slopes_p or slopes_a)
                rate, byte_rate = calls[0], bytes_[0]
            else:
                values, rate, byte_rate = slopes_b, calls[1], bytes_[1]
            runs += runs_for(label, values, w, r, rate, byte_rate, harness=harness)
    variants = {label: {"sha": label * 40, "ref": label, "binaries": {"counts": "same"}} for label in labels}
    return {"mode": mode, "variants": variants, "runs": runs, "completed": True}


ALL_ROUND_TRIPS = [(w, r) for w in ca.ROUND_TRIP for r in ca.RUNTIMES if (w, r) != ("rp", "ct")]


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
        self.assertAlmostEqual(row["a_median_with_harness"], median(self.A) + 545.0, places=3)

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
        with self.assertRaises(ValueError):
            ca.analyze(m)
        floors = {f"{w}/{r}": 60.0 for w in ca.ROUND_TRIP for r in ca.RUNTIMES}
        s = ca.analyze(m, floors)
        self.assertEqual(self.row(s)["noise_floor_source"], "recorded noise run (quick mode)")
        self.assertTrue(s["accepted"])

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
                    aim="rp", noise_floors=None, blocks=3)
        base.update(kwargs)
        return argparse.Namespace(**base)

    def test_comparison_builds_the_baseline_twice(self):
        mode, refs, order, *_ = perf_counts.plan(self.args())
        self.assertEqual((mode, refs), ("compare", {"A": "dev", "P": "dev", "B": "topic"}))
        self.assertEqual(order, "ABPPBA" * 3)
        self.assertEqual(perf_counts.plan(self.args(noise="dev", baseline=None, candidate=None))[2], "APPA" * 3)

    def test_bad_plans_are_refused(self):
        for kwargs in ({"aim": "RP"}, {"workloads": "wp", "aim": "rp"}, {"runtimes": "ct,xt"},
                       {"workloads": "rp,typo"}, {"candidate": None}):
            with self.subTest(**kwargs), self.assertRaises(ValueError):
                perf_counts.plan(self.args(**kwargs))

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

    def test_exit_status(self):
        self.assertEqual(perf_counts.exit_code({"mode": "compare", "accepted": False}), 1)
        self.assertEqual(perf_counts.exit_code({"mode": "compare", "accepted": True}), 0)
        self.assertEqual(perf_counts.exit_code({"mode": "noise"}), 0)


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

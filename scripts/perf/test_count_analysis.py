#!/usr/bin/env python3
"""Tests for count_analysis.py: parsing, slopes, the noise floor and gates.

  python3 -m unittest discover -s scripts/perf -p 'test_*.py'
"""

import copy
import unittest
from pathlib import Path

import count_analysis as ca

DATA = Path(__file__).resolve().parent / "testdata"


def manifest(slopes_a, slopes_b, workload="rp", runtime="ct", calls=(22.0, 22.0), bytes_=(2196.0, 2196.0),
             mode="compare", base=5_000_000):
    """A completed manifest whose runs have exactly the given slopes.

    Each ordinal's short run costs `base + slope * short`, its long run
    `base + slope * long`, so the slope is recovered exactly.
    """
    short, long = ca.lengths(workload)
    runs = []
    for label, slopes, rate, byte_rate in (("A", slopes_a, calls[0], bytes_[0]), ("B", slopes_b, calls[1], bytes_[1])):
        for ordinal, slope in enumerate(slopes, 1):
            for n in (short, long):
                runs.append({
                    "name": f"{workload}-{runtime}-{label}{ordinal}-{n}",
                    "workload": workload, "runtime": runtime, "label": label, "ordinal": ordinal, "n": n,
                    "instructions": round(base + slope * n),
                    "allocs": round(rate * n), "bytes": round(byte_rate * n),
                })
    variants = {label: {"sha": label * 40, "ref": label, "binaries": {"counts": "same"}} for label in "AB"}
    return {"mode": mode, "variants": variants, "runs": runs, "completed": True}


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
    A = [40_000.0, 40_004.0, 40_002.0, 39_998.0, 40_001.0, 40_003.0]  # range 6

    def summary(self, b, **kwargs):
        floors = kwargs.pop("floors", None)
        aimed = kwargs.pop("aimed", ("rp",))
        return ca.analyze(manifest(self.A, b, **kwargs), floors, aimed)

    def test_noise_floor_is_the_range_of_both_builds(self):
        s = ca.analyze(manifest(self.A, [40_010.0] * 6, mode="noise"))
        self.assertAlmostEqual(s["rows"][0]["noise_floor"], 12.0, places=3)
        self.assertEqual(s["rows"][0]["verdict"], "floor")
        self.assertTrue(s["binaries_identical"])
        self.assertEqual(s["noise_floors"], {"rp/ct": s["rows"][0]["noise_floor"]})

    def test_saving_passes(self):
        s = self.summary([x - 520 for x in self.A])
        self.assertEqual(s["rows"][0]["verdict"], "PASS")
        self.assertTrue(s["accepted"])
        self.assertEqual(s["rows"][0]["noise_floor_source"], "A's own range")

    def test_saving_under_100_fails(self):
        s = self.summary([x - 99 for x in self.A])
        self.assertEqual(s["rows"][0]["verdict"], "FAIL")
        self.assertFalse(s["accepted"])

    def test_saving_must_beat_three_floors(self):
        # 150 saved, but a recorded floor of 60 needs more than 180.
        s = self.summary([x - 150 for x in self.A], floors={"rp/ct": 60.0})
        self.assertEqual(s["rows"][0]["noise_floor_source"], "recorded A vs A' run")
        self.assertEqual(s["rows"][0]["verdict"], "FAIL")
        s = self.summary([x - 181 for x in self.A], floors={"rp/ct": 60.0})
        self.assertEqual(s["rows"][0]["verdict"], "PASS")

    def test_every_pair_must_fall(self):
        b = [x - 520 for x in self.A]
        b[3] = self.A[3] + 1
        s = self.summary(b)
        self.assertEqual(s["rows"][0]["verdict"], "FAIL")
        self.assertFalse(s["rows"][0]["instructions_ok"])

    def test_allocation_tolerance(self):
        b = [x - 520 for x in self.A]
        ok = self.summary(b, calls=(22.0, 22.009))
        self.assertEqual(ok["rows"][0]["verdict"], "PASS")
        more = self.summary(b, calls=(22.0, 22.02))
        self.assertEqual(more["rows"][0]["verdict"], "FAIL")
        self.assertFalse(more["rows"][0]["allocations_ok"])
        bytes_up = self.summary(b, bytes_=(2196.0, 2196.5))
        self.assertEqual(bytes_up["rows"][0]["verdict"], "FAIL")

    def test_other_workloads_must_not_rise_beyond_the_floor(self):
        within = self.summary([x + 5 for x in self.A], workload="wp")
        self.assertEqual(within["rows"][0]["verdict"], "PASS")
        above = self.summary([x + 7 for x in self.A], workload="wp")
        self.assertEqual(above["rows"][0]["verdict"], "FAIL")
        aimed = self.summary([x - 520 for x in self.A], workload="wp", aimed=("rp", "wp"))
        self.assertEqual(aimed["rows"][0]["verdict"], "PASS")

    def test_codec_rows_are_reported_not_gated(self):
        s = self.summary([x + 1000 for x in self.A], workload="full_stack_encode_rp", runtime="none")
        self.assertEqual(s["rows"][0]["verdict"], "info")
        self.assertFalse(s["accepted"])  # nothing gated, nothing accepted

    def test_incomplete_runs_are_ignored_not_guessed(self):
        m = manifest(self.A, [x - 520 for x in self.A])
        broken = copy.deepcopy(m)
        broken["runs"][0]["instructions"] = None
        with self.assertRaises(ValueError):
            ca.analyze(broken)


class Profiles(unittest.TestCase):
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

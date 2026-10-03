#!/usr/bin/env python3
"""Unit tests for ledger_tables.py and the generator's ledger page:
python3 -m unittest discover -s scripts -p 'test_ledger_tables.py'"""

import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ledger_tables as lt

SCRIPTS = Path(__file__).resolve().parent


def row(row_id: str, evidence=None, status: str = "in-progress") -> dict:
    out = {"id": row_id, "standard_anchor": f"Clause {row_id[-1]}", "priority": "P1", "status": status}
    if evidence is not None:
        out["evidence"] = evidence
    return out


def ledger(*rows: dict) -> dict:
    return {"rows": list(rows)}


def page(*tables: list[str], body=None) -> str:
    """A page of hand-kept prose around marked tables that list `tables`."""
    lines = ["# Title", "", "Prose before."]
    for ids in tables:
        lines += ["", "## Section", "", *lt.marked(ids, body if body is not None else ["stale"]), "", "Prose after."]
    return "\n".join(lines) + "\n"


class RenderTests(unittest.TestCase):
    def test_row_line_joins_evidence_with_single_spaces(self):
        line = lt.row_line(row("ROW-A", ["First topic.", "Second topic."]))
        self.assertEqual(line, "| `ROW-A` | Clause A | P1 | `in-progress` | First topic. Second topic. |")

    def test_rewrites_only_between_markers(self):
        data = ledger(row("ROW-A", ["A."]), row("ROW-B", ["B."]), row("ROW-C"))
        out = lt.render(page(["ROW-A"], ["ROW-B"]), data)
        expected = page(["ROW-A"], ["ROW-B"])
        for row_id in ("A", "B"):
            table = [*lt.HEADER, f"| `ROW-{row_id}` | Clause {row_id} | P1 | `in-progress` | {row_id}. |"]
            expected = expected.replace("stale", "\n".join(table), 1)
        self.assertEqual(out, expected)
        self.assertEqual(lt.render(out, data), out)

    def test_hand_edit_between_markers_is_overwritten(self):
        data = ledger(row("ROW-A", ["A."]))
        current = lt.render(page(["ROW-A"]), data)
        edited = current.replace("| A. |", "| A, edited by hand. |")
        self.assertNotEqual(edited, current)
        self.assertEqual(lt.render(edited, data), current)

    def test_rows_keep_the_order_the_marker_lists(self):
        data = ledger(row("ROW-A", ["A."]), row("ROW-B", ["B."]))
        out = lt.render(page(["ROW-B", "ROW-A"]), data)
        self.assertLess(out.index("`ROW-B`"), out.index("`ROW-A`"))

    def test_page_without_markers_is_unchanged(self):
        text = "# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n"
        self.assertEqual(lt.render(text, ledger(row("ROW-A"))), text)

    def test_listing_one_more_row_adds_only_its_lines(self):
        base_data = ledger(row("ROW-A", ["A."]), row("ROW-B", ["B."]))
        base = lt.render(page(["ROW-A"]), dict(base_data, rows=base_data["rows"][:1]))
        grown = lt.render(page(["ROW-A", "ROW-B"]), base_data).split("\n")
        added = [line for line in grown if line not in base.split("\n")]
        self.assertEqual(added, ["ROW-B", "| `ROW-B` | Clause B | P1 | `in-progress` | B. |"])
        self.assertEqual(len(grown), len(base.split("\n")) + 2)


class ProblemTests(unittest.TestCase):
    def assertProblem(self, text: str, data: dict, fragment: str):
        with self.assertRaises(lt.LedgerTableError) as ctx:
            lt.render(text, data)
        self.assertTrue(any(fragment in p for p in ctx.exception.problems), ctx.exception.problems)

    def test_listed_rows_must_exist_and_hold_evidence(self):
        self.assertProblem(page(["ROW-X"]), ledger(), "not a ledger row")
        self.assertProblem(page(["ROW-A"]), ledger(row("ROW-A")), "has no evidence")

    def test_evidence_rows_must_be_listed_once(self):
        data = ledger(row("ROW-A", ["A."]), row("ROW-B", ["B."]))
        self.assertProblem(page(["ROW-A"]), data, "ROW-B: has evidence but no table lists it")
        self.assertProblem(page(["ROW-A", "ROW-B"], ["ROW-A"]), data, "listed by more than one table")

    def test_evidence_must_be_trimmed_non_empty_strings(self):
        for bad in ([], "A.", ["A.", 3], [""], [" A."]):
            with self.subTest(evidence=bad):
                self.assertProblem(page(["ROW-A"]), ledger(row("ROW-A", bad)), "ROW-A: evidence")

    def test_malformed_markers(self):
        data = ledger(row("ROW-A", ["A."]))
        unclosed = page(["ROW-A"]).replace(lt.END, "")
        self.assertProblem(unclosed, data, "without an END marker")
        self.assertProblem("text\n" + lt.END + "\n", data, "END marker without a BEGIN marker")
        blank_id = page(["ROW-A"]).replace("ROW-A\n-->", "ROW-A\n\n-->")
        self.assertProblem(blank_id, data, "one row ID per line")
        self.assertProblem(page([]), ledger(), "lists no row IDs")
        nested = page(["ROW-A"]).replace(lt.END, lt.BEGIN_LINE + "\nROW-A\n-->\n" + lt.END)
        self.assertProblem(nested, data, "without an END marker")


def load_generator():
    spec = importlib.util.spec_from_file_location("gen_ledger_tables", SCRIPTS / "generate-conformance-docs.py")
    gen = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gen)
    return gen


class GeneratorTests(unittest.TestCase):
    """generate-conformance-docs.py writes the page's tables and --check flags hand edits."""

    def run_generator(self, text: str, data: dict, *args: str) -> tuple[int, str, str]:
        gen = load_generator()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ledger.md"
            path.write_text(text, encoding="utf-8")
            out = io.StringIO()
            with (
                mock.patch.object(gen, "ROOT", Path(tmp)),
                mock.patch.object(gen, "LEDGER_PAGE", path),
                mock.patch.object(gen, "load_ledger", return_value=data),
                mock.patch.object(gen, "generated", return_value={}),
                mock.patch.object(gen.check_ledger_anchors, "check", return_value=0),
                mock.patch.object(sys, "argv", ["generate-conformance-docs.py", *args]),
                contextlib.redirect_stdout(out),
            ):
                code = gen.main()
            return code, out.getvalue(), path.read_text(encoding="utf-8")

    def test_check_flags_a_hand_edit_and_a_write_repairs_it(self):
        data = ledger(row("ROW-A", ["A."]))
        current = lt.render(page(["ROW-A"]), data)
        self.assertEqual(self.run_generator(current, data, "--check")[0], 0)
        edited = current.replace("`in-progress`", "`supported-with-clause-evidence`")
        code, out, _ = self.run_generator(edited, data, "--check")
        self.assertEqual((code, out), (1, "stale: ledger.md\n"))
        code, _, written = self.run_generator(edited, data)
        self.assertEqual((code, written), (0, current))

    def test_marker_problems_fail_both_modes(self):
        data = ledger(row("ROW-A", ["A."]), row("ROW-B", ["B."]))
        text = lt.render(page(["ROW-A", "ROW-B"]), data)
        for args in ((), ("--check",)):
            with self.subTest(args=args):
                code, out, written = self.run_generator(text, ledger(data["rows"][0]), *args)
                self.assertEqual(code, 1)
                self.assertIn("ledger.md: ROW-B: listed by a table but not a ledger row", out)
                self.assertEqual(written, text)

    def test_real_page_renders_unchanged(self):
        gen = load_generator()
        data = gen.load_ledger()
        text = gen.LEDGER_PAGE.read_text(encoding="utf-8")
        self.assertEqual(lt.render(text, data), text)
        self.assertEqual(gen.outputs(data)[gen.LEDGER_PAGE], text)


if __name__ == "__main__":
    unittest.main()

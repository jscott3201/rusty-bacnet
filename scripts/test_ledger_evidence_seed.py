#!/usr/bin/env python3
"""Unit tests for ledger_evidence_seed.py:
python3 -m unittest discover -s scripts -p 'test_ledger_evidence_seed.py'"""

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import ledger_evidence_seed as les
import ledger_tables as lt
from ledger_notes_split import MIN_CHUNK, split_notes

LONG = "This sentence is long enough on its own to close an entry, " + "padding " * 20 + "end."
assert len(LONG) >= MIN_CHUNK

# Three rows in the ledger's own layout: ROW-A's notes are one line followed by
# another key, ROW-B's notes are a multi-line array closing the row, and ROW-C
# is in no table.
SOURCE = "\n".join(
    [
        "{",
        ' "standard": "S",',
        ' "rows": [',
        "  {",
        '   "id": "ROW-A",',
        '   "standard_anchor": "Clause 1",',
        '   "priority": "P1",',
        '   "status": "in-progress",',
        '   "notes": ["Short note."],',
        '   "extra": {',
        '    "notes": "Not a row note."',
        "   }",
        "  },",
        "  {",
        '   "id": "ROW-B",',
        '   "standard_anchor": "Clause 2",',
        '   "priority": "P2",',
        '   "status": "supported-with-clause-evidence",',
        '   "notes": [',
        '    "First topic.",',
        '    "Second \\u00a7 topic."',
        "   ]",
        "  },",
        "  {",
        '   "id": "ROW-C",',
        '   "standard_anchor": "Clause 3",',
        '   "priority": "P3",',
        '   "status": "in-progress",',
        '   "notes": ["Unlisted."]',
        "  }",
        " ]",
        "}",
        "",
    ]
)


def line(row_id, anchor, priority, status, evidence):
    return f"| `{row_id}` | {anchor} | {priority} | `{status}` | {evidence} |"


A_EVIDENCE = 'Evidence for A with a "quote" and a back\\slash.'
B_EVIDENCE = f"{LONG} #12: second topic — with a dash."


def page(a_anchor="Clause 1", b_status="supported-with-clause-evidence", b_header=True) -> str:
    b = [line("ROW-B", "Clause 2", "P2", b_status, B_EVIDENCE)]
    return "\n".join(
        [
            "# Ledger",
            "",
            "## One",
            "",
            *lt.HEADER,
            line("ROW-A", a_anchor, "P1", "in-progress", A_EVIDENCE),
            "",
            "Prose between.",
            "",
            *(list(lt.HEADER) if b_header else []),
            *b,
            "",
            "Prose after.",
            "",
        ]
    )


class AlignTests(unittest.TestCase):
    def test_takes_anchor_priority_and_status_from_the_json(self):
        text, changed, headers = les.align(page(a_anchor="Old anchor", b_status="in-progress"), json.loads(SOURCE))
        self.assertEqual((text, changed, headers), (page(), 2, 0))

    def test_gives_a_headerless_row_line_its_header(self):
        text, changed, headers = les.align(page(b_header=False), json.loads(SOURCE))
        self.assertEqual((text, changed, headers), (page(), 0, 1))

    def test_rerun_is_a_no_op(self):
        self.assertEqual(les.align(page(), json.loads(SOURCE)), (page(), 0, 0))

    def test_unknown_row_is_refused(self):
        with self.assertRaises(les.SeedError):
            les.align(page().replace("ROW-A", "ROW-X"), json.loads(SOURCE))

    def test_malformed_table_line_is_refused(self):
        with self.assertRaises(les.SeedError):
            les.align(page().replace("| P1 |", "| high |"), json.loads(SOURCE))


class SeedTests(unittest.TestCase):
    def test_writes_evidence_after_notes_and_marks_the_tables(self):
        new_page, new_source, seeded, marked = les.seed(page(), SOURCE)
        self.assertEqual((seeded, marked), (2, 2))
        b_entries = split_notes(B_EVIDENCE)
        self.assertEqual(b_entries, [LONG, "#12: second topic — with a dash."])
        expected = (
            SOURCE.replace(
                '   "notes": ["Short note."],\n',
                '   "notes": ["Short note."],\n'
                '   "evidence": [\n'
                '    "Evidence for A with a \\"quote\\" and a back\\\\slash."\n'
                "   ],\n",
            ).replace(
                '    "Second \\u00a7 topic."\n   ]\n',
                '    "Second \\u00a7 topic."\n   ],\n'
                '   "evidence": [\n'
                f'    "{LONG}",\n'
                '    "#12: second topic — with a dash."\n'
                "   ]\n",
            )
        )
        self.assertEqual(new_source, expected)
        data = json.loads(new_source)
        self.assertNotIn("evidence", data["rows"][2])
        self.assertEqual(data["rows"][0]["extra"], {"notes": "Not a row note."})
        self.assertEqual(lt.render(new_page, data), new_page)
        lines = page().split("\n")
        expected_page = [
            *lines[:4],
            *lt.marked(["ROW-A"], lines[4:7]),
            *lines[7:10],
            *lt.marked(["ROW-B"], lines[10:13]),
            *lines[13:],
        ]
        self.assertEqual(new_page, "\n".join(expected_page))

    def test_rerun_is_a_no_op(self):
        new_page, new_source, _, _ = les.seed(page(), SOURCE)
        self.assertEqual(les.seed(new_page, new_source), (new_page, new_source, 0, 0))

    def test_refuses_an_unaligned_page(self):
        for unaligned in (page(a_anchor="Old anchor"), page(b_header=False)):
            with self.subTest(page=unaligned), self.assertRaises(les.SeedError) as ctx:
                les.seed(unaligned, SOURCE)
            self.assertTrue(all("run --align first" in p for p in ctx.exception.problems))

    def test_refuses_a_row_listed_twice_or_an_empty_cell(self):
        twice = page().replace("Prose after.", lt.HEADER[0] + "\n" + lt.HEADER[1] + "\n" + page().split("\n")[5])
        with self.assertRaises(les.SeedError):
            les.seed(twice, SOURCE)
        with self.assertRaises(les.SeedError):
            les.seed(page().replace(f"| {A_EVIDENCE} |", "|  |"), SOURCE)

    def test_a_row_already_holding_different_evidence_fails_verification(self):
        data = json.loads(SOURCE)
        data["rows"][0]["evidence"] = ["Something else."]
        with self.assertRaises(les.SeedError) as ctx:
            les.seed(page(), json.dumps(data, indent=1, ensure_ascii=False))
        self.assertIn("the generator would not reproduce the page from the seeded ledger", ctx.exception.problems)


class RealLedgerTests(unittest.TestCase):
    def test_align_then_seed_on_copies_of_the_real_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            ledger, page_path = Path(tmp) / "ledger.json", Path(tmp) / "ledger.md"
            ledger.write_text(les.LEDGER.read_text(encoding="utf-8"), encoding="utf-8")
            page_path.write_text(les.PAGE.read_text(encoding="utf-8"), encoding="utf-8")
            args = ["--ledger", str(ledger), "--page", str(page_path)]
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(les.main(["--align", *args]), 0)
                self.assertEqual(les.main(args), 0)
                seeded = (ledger.read_text(encoding="utf-8"), page_path.read_text(encoding="utf-8"))
                self.assertEqual(les.main(["--align", *args]), 0)
                self.assertEqual(les.main(args), 0)
            self.assertEqual((ledger.read_text(encoding="utf-8"), page_path.read_text(encoding="utf-8")), seeded)
            data = json.loads(seeded[0])
            self.assertEqual(lt.render(seeded[1], data), seeded[1])
            listed = [row_id for region in lt.regions(seeded[1].split("\n")) for row_id in region.ids]
            self.assertTrue(listed, "the real page should list rows once seeded")
            self.assertEqual(sorted(listed), sorted(row["id"] for row in data["rows"] if "evidence" in row))


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Seed each ledger row's evidence from the standard ledger page's tables (#1191).

The row tables of docs/conformance/standard-135-2020-ledger.md were kept by
hand. This script moves their Evidence text into the JSON, so that
generate-conformance-docs.py writes the tables from it (see ledger_tables.py).
It runs in two steps, each committed on its own:

1. `--align` rewrites the Anchor, Priority and Status cells of every table line
   to the row's values in the JSON, and puts the table header above a row line
   that has none. Only the page changes.
2. With no arguments, it stores each table line's Evidence text as the row's
   `evidence` array, written right after the row's notes and split one topic
   per entry the way ledger_notes_split.py splits notes. It then wraps each
   table in markers that list its rows. It writes nothing unless the generator
   reproduces the page byte for byte from the result and the JSON is unchanged
   apart from the new keys.

Tables that already have markers and rows that already hold evidence are left
alone, so both steps are safe to re-run. Rows that no table lists get no
evidence.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

import ledger_tables
from ledger_notes_split import LEDGER, split_notes

PAGE = LEDGER.parent / "standard-135-2020-ledger.md"

# One hand-kept table line: ID, Anchor, Priority, Status, Evidence.
ROW = re.compile(
    r"^\| `(?P<id>[^`]+)` \| (?P<anchor>.*?) \| (?P<priority>P\d) \| `(?P<status>[^`]+)` \| (?P<evidence>.*) \|$"
)
# A row's `"id"` line and the first line of a `"notes"` value in the ledger JSON.
ID_LINE = re.compile(r'^\s*"id":\s*"(?P<id>[^"\\]+)",?\s*$')
NOTES_LINE = re.compile(r'^(?P<indent>[ \t]*)"notes":')
ARRAY_CLOSE = re.compile(r"^\s*\],?\s*$")


class SeedError(ValueError):
    """The page or the ledger is not in a state this script can convert."""

    def __init__(self, problems: list[str]):
        super().__init__("\n".join(problems))
        self.problems = problems


@dataclass(frozen=True)
class Table:
    """An unmarked run of row lines `lines[first..=last]`, with the index of its
    header line, or None when the run has no header above it."""

    header: int | None
    first: int
    last: int


def unmarked_tables(lines: list[str]) -> list[Table]:
    """Every run of ledger row lines outside the markers, in page order."""
    inside: set[int] = set()
    for region in ledger_tables.regions(lines):
        inside.update(range(region.begin, region.end + 1))
    tables: list[Table] = []
    i = 0
    while i < len(lines):
        if i not in inside and lines[i : i + 2] == list(ledger_tables.HEADER):
            j = i + 2
            while j < len(lines) and lines[j].startswith("|"):
                if not ROW.match(lines[j]):
                    raise SeedError([f"line {j + 1}: a ledger table line must read | `ID` | Anchor | P<n> | `status` | Evidence |"])
                j += 1
            if j == i + 2:
                raise SeedError([f"line {i + 1}: a ledger table has no rows"])
            tables.append(Table(i, i + 2, j - 1))
            i = j
        elif i not in inside and ROW.match(lines[i]):
            j = i
            while j < len(lines) and ROW.match(lines[j]):
                j += 1
            tables.append(Table(None, i, j - 1))
            i = j
        else:
            i += 1
    return tables


def _cells(row: dict, evidence: str) -> str:
    return f"| `{row['id']}` | {row['standard_anchor']} | {row['priority']} | `{row['status']}` | {evidence} |"


def align(page: str, data: dict) -> tuple[str, int, int]:
    """`page` with every unmarked table line's Anchor, Priority and Status taken
    from the JSON and a header above each headerless run. Returns the new page,
    the number of row lines changed and the number of headers added."""
    rows = {row["id"]: row for row in data["rows"]}
    lines = page.split("\n")
    tables = unmarked_tables(lines)
    headerless = {table.first for table in tables if table.header is None}
    row_lines = {i for table in tables for i in range(table.first, table.last + 1)}
    out: list[str] = []
    changed = 0
    problems: list[str] = []
    for i, line in enumerate(lines):
        if i in headerless:
            out += ledger_tables.HEADER
        if i in row_lines:
            m = ROW.match(line)
            if m["id"] not in rows:
                problems.append(f"line {i + 1}: {m['id']} is not a ledger row")
                continue
            line = _cells(rows[m["id"]], m["evidence"])
            changed += line != lines[i]
        out.append(line)
    if problems:
        raise SeedError(problems)
    return "\n".join(out), changed, len(headerless)


def table_evidence(lines: list[str], tables: list[Table], data: dict) -> dict[str, str]:
    """Each listed row's Evidence text, after checking that the tables agree
    with the JSON and list each row once."""
    rows = {row["id"]: row for row in data["rows"]}
    listed = {row_id for region in ledger_tables.regions(lines) for row_id in region.ids}
    evidence: dict[str, str] = {}
    problems: list[str] = []
    for table in tables:
        if table.header is None:
            problems.append(f"line {table.first + 1}: row line without a table header; run --align first")
        for i in range(table.first, table.last + 1):
            m = ROW.match(lines[i])
            row = rows.get(m["id"])
            if row is None:
                problems.append(f"line {i + 1}: {m['id']} is not a ledger row")
            elif lines[i] != _cells(row, m["evidence"]):
                problems.append(f"line {i + 1}: {m['id']} Anchor, Priority or Status differs from the JSON; run --align first")
            elif m["id"] in evidence or m["id"] in listed:
                problems.append(f"line {i + 1}: {m['id']} is listed by more than one table")
            elif not m["evidence"].strip():
                problems.append(f"line {i + 1}: {m['id']} has an empty Evidence cell")
            else:
                evidence[m["id"]] = m["evidence"]
    if problems:
        raise SeedError(problems)
    return evidence


def insert_evidence(source: str, entries: dict[str, list[str]]) -> str:
    """Ledger source text with an `"evidence"` array written right after the
    `"notes"` value of each row named in `entries`, one entry per line."""
    lines = source.split("\n")
    out: list[str] = []
    row_id = None
    i = 0
    while i < len(lines):
        if m := ID_LINE.match(lines[i]):
            row_id = m["id"]
        notes = NOTES_LINE.match(lines[i])
        if not notes or row_id not in entries:
            out.append(lines[i])
            i += 1
            continue
        end = i
        if lines[i].rstrip().endswith("["):
            while not ARRAY_CLOSE.match(lines[end]):
                end += 1
                if end == len(lines):
                    raise SeedError([f"{row_id}: notes array never closes"])
        block = lines[i : end + 1]
        comma = block[-1].rstrip().endswith(",")
        if not comma:
            block[-1] = block[-1].rstrip() + ","
        indent = notes["indent"]
        body = [f"{indent} {json.dumps(entry, ensure_ascii=False)}," for entry in entries[row_id]]
        body[-1] = body[-1][:-1]
        out += [*block, f'{indent}"evidence": [', *body, f"{indent}]" + ("," if comma else "")]
        row_id = None  # one insertion per row
        i = end + 1
    return "\n".join(out)


def seed(page: str, source: str) -> tuple[str, str, int, int]:
    """Seed evidence and mark the tables. Returns the new page, the new ledger
    source, the number of rows seeded and the number of tables marked."""
    data = json.loads(source)
    lines = page.split("\n")
    tables = unmarked_tables(lines)
    evidence = table_evidence(lines, tables, data)
    out: list[str] = []
    start = 0
    for table in tables:
        ids = [ROW.match(lines[i])["id"] for i in range(table.first, table.last + 1)]
        out += lines[start : table.header]
        out += ledger_tables.marked(ids, lines[table.header : table.last + 1])
        start = table.last + 1
    new_page = "\n".join(out + lines[start:])
    has_evidence = {row["id"] for row in data["rows"] if "evidence" in row}
    entries = {row_id: split_notes(text) for row_id, text in evidence.items() if row_id not in has_evidence}
    new_source = insert_evidence(source, entries)
    _verify(data, json.loads(new_source), entries, new_page)
    return new_page, new_source, len(entries), len(tables)


def _verify(old: dict, new: dict, entries: dict[str, list[str]], page: str) -> None:
    """The ledger gains only the seeded `evidence` keys, each right after its
    row's notes, and the generator reproduces the marked page exactly."""
    problems: list[str] = []
    rows = []
    for row in new["rows"]:
        if row["id"] in entries:
            keys = [*row, None]
            if row.get("evidence") != entries[row["id"]] or "notes" not in row or keys[keys.index("notes") + 1] != "evidence":
                problems.append(f"{row['id']}: evidence was not written right after notes")
            row = {key: value for key, value in row.items() if key != "evidence"}
        rows.append(row)
    if dict(new, rows=rows) != old:
        problems.append("seeding changed the ledger beyond the new evidence keys")
    if not problems:
        try:
            if ledger_tables.render(page, new) != page:
                problems.append("the generator would not reproduce the page from the seeded ledger")
        except ledger_tables.LedgerTableError as err:
            problems += err.problems
    if problems:
        raise SeedError(problems)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--align", action="store_true", help="only take each table line's Anchor, Priority and Status from the JSON")
    parser.add_argument("--ledger", type=Path, default=LEDGER)
    parser.add_argument("--page", type=Path, default=PAGE)
    args = parser.parse_args(argv)
    page = args.page.read_text(encoding="utf-8")
    source = args.ledger.read_text(encoding="utf-8")
    try:
        if args.align:
            new_page, changed, headers = align(page, json.loads(source))
            print(f"aligned {changed} row line(s); added {headers} table header(s)")
        else:
            new_page, new_source, seeded, marked = seed(page, source)
            if new_source != source:
                args.ledger.write_text(new_source, encoding="utf-8")
            print(f"seeded evidence in {seeded} row(s); marked {marked} table(s)")
    except (SeedError, ledger_tables.LedgerTableError) as err:
        for problem in err.problems:
            print(problem)
        return 1
    if new_page != page:
        args.page.write_text(new_page, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())

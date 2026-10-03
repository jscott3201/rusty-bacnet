"""Row tables of the standard ledger page, generated from the JSON (#1191).

docs/conformance/standard-135-2020-ledger.md is written by hand apart from its
row tables. Each table sits between two markers. The BEGIN comment lists the
IDs of the rows the table shows, one per line and in table order:

    <!-- BEGIN ledger-rows: generated from these rows of bacnet-135-2020.json; ...
    BACNET-4-ARCHITECTURE
    -->
    | Row ID | Anchor | Priority | Status | Evidence |
    |---|---|---|---|---|
    | `BACNET-4-ARCHITECTURE` | Clause 4 | P1 | `in-progress` | ... |
    <!-- END ledger-rows -->

generate-conformance-docs.py rewrites the lines between the markers from each
listed row's standard_anchor, priority, status and evidence, and leaves the
rest of the page alone, so --check flags a hand edit between them. A row's
`evidence` is an array of entries, one topic each, printed joined with single
spaces. A row has `evidence` exactly when a table lists it: to show another
row, give it evidence and add its ID to a BEGIN comment. The key is temporary:
the #1208 condense batches fold it into the row's other keys and delete it.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

HEADER = ("| Row ID | Anchor | Priority | Status | Evidence |", "|---|---|---|---|---|")
BEGIN = "<!-- BEGIN ledger-rows"
BEGIN_LINE = f"{BEGIN}: generated from these rows of bacnet-135-2020.json; edit the JSON and regenerate"
BEGIN_CLOSE = "-->"
END = "<!-- END ledger-rows -->"


class LedgerTableError(ValueError):
    """The page's markers or the rows they list cannot be rendered."""

    def __init__(self, problems: list[str]):
        super().__init__("\n".join(problems))
        self.problems = problems


@dataclass(frozen=True)
class Region:
    """One marked table: `lines[begin]` opens the BEGIN comment, `lines[body]`
    is the first line after it and `lines[end]` is the END marker."""

    begin: int
    body: int
    end: int
    ids: tuple[str, ...]


def regions(lines: list[str]) -> list[Region]:
    """The marked tables of a page split into lines."""
    found: list[Region] = []
    problems: list[str] = []
    i = 0
    while i < len(lines):
        if lines[i] == END:
            problems.append(f"line {i + 1}: END marker without a BEGIN marker")
        elif lines[i].startswith(BEGIN):
            region = _region(lines, i, problems)
            if region is None:
                break
            found.append(region)
            i = region.end
        i += 1
    if problems:
        raise LedgerTableError(problems)
    return found


def _region(lines: list[str], begin: int, problems: list[str]) -> Region | None:
    """The marked table whose BEGIN comment opens at `lines[begin]`."""
    close = begin + 1
    while close < len(lines) and re.fullmatch(r"\S+", lines[close]) and lines[close] != BEGIN_CLOSE:
        close += 1
    if close == len(lines) or lines[close] != BEGIN_CLOSE:
        problems.append(f"line {begin + 1}: the BEGIN comment must list one row ID per line, then close with `-->`")
        return None
    ids = tuple(lines[begin + 1 : close])
    if not ids:
        problems.append(f"line {begin + 1}: the BEGIN comment lists no row IDs")
    end = close + 1
    while end < len(lines) and lines[end] != END and not lines[end].startswith(BEGIN):
        end += 1
    if end == len(lines) or lines[end] != END:
        problems.append(f"line {begin + 1}: BEGIN marker without an END marker")
        return None
    return Region(begin, close + 1, end, ids)


def evidence_problems(row: dict) -> list[str]:
    """Why a row's `evidence` cannot be printed, if it is malformed."""
    evidence = row.get("evidence")
    if not isinstance(evidence, list) or not evidence:
        return [f"{row['id']}: evidence must be an array holding at least one entry"]
    return [
        f"{row['id']}: evidence entry {entry!r} must be a non-empty string without outer whitespace"
        for entry in evidence
        if not isinstance(entry, str) or not entry or entry.strip() != entry
    ]


def row_line(row: dict) -> str:
    """One table line for a ledger row."""
    evidence = " ".join(row["evidence"])
    return f"| `{row['id']}` | {row['standard_anchor']} | {row['priority']} | `{row['status']}` | {evidence} |"


def table(rows: list[dict]) -> list[str]:
    """The lines between one pair of markers."""
    return [*HEADER, *(row_line(row) for row in rows)]


def render(page: str, data: dict) -> str:
    """`page` with every marked table rewritten from the ledger `data`.

    Raises LedgerTableError when a marker is malformed, a listed ID has no row
    or no evidence, a row is listed twice, or a row with evidence is not listed.
    """
    lines = page.split("\n")
    found = regions(lines)
    rows = {row["id"]: row for row in data["rows"]}
    problems: list[str] = []
    listed: set[str] = set()
    for region in found:
        for row_id in region.ids:
            if row_id in listed:
                problems.append(f"{row_id}: listed by more than one table")
            listed.add(row_id)
            if row_id not in rows:
                problems.append(f"{row_id}: listed by a table but not a ledger row")
            elif "evidence" not in rows[row_id]:
                problems.append(f"{row_id}: listed by a table but has no evidence")
            else:
                problems += evidence_problems(rows[row_id])
    for row_id, row in rows.items():
        if "evidence" in row and row_id not in listed:
            problems.append(f"{row_id}: has evidence but no table lists it")
    if problems:
        raise LedgerTableError(problems)
    out: list[str] = []
    start = 0
    for region in found:
        out += lines[start : region.body]
        out += table([rows[row_id] for row_id in region.ids])
        start = region.end
    out += lines[start:]
    return "\n".join(out)


def marked(ids: list[str], body: list[str]) -> list[str]:
    """`body` wrapped in markers that list `ids`."""
    return [BEGIN_LINE, *ids, BEGIN_CLOSE, *body, END]

#!/usr/bin/env python3
"""Generate draft BACnet conformance support documents from the ledger."""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "docs" / "conformance" / "bacnet-135-2020.json"
OUTPUTS = {
    "support": ROOT / "docs" / "conformance" / "support-summary.md",
    "pics": ROOT / "docs" / "conformance" / "pics-draft.md",
    "bibbs": ROOT / "docs" / "conformance" / "bibbs-draft.md",
}


def load_ledger() -> dict:
    with LEDGER.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def header(title: str) -> list[str]:
    return [
        f"# {title}",
        "",
        "> DRAFT internal support evidence. Generated from `docs/conformance/bacnet-135-2020.json`; this is not a BTL certification claim or formal PICS/BIBB declaration.",
        "",
    ]


def md_list(items: list[str]) -> str:
    return ", ".join(f"`{item}`" for item in items) if items else "-"


def support_summary(data: dict) -> str:
    rows = data["rows"]
    by_status = Counter(row["status"] for row in rows)
    by_priority = Counter(row["priority"] for row in rows)
    lines = header("BACnet Standard 135-2020 Support Summary")
    lines += [
        f"- Standard: {data['standard']}",
        f"- Reviewed at: {data['reviewed_at']}",
        f"- Implementation evidence SHA reviewed: `{data['repo_sha']}`",
        f"- Scope: {data['review_scope']}",
        f"- Addenda/errata: {data['addenda_errata_status']}",
        "",
        "## Counts",
        "",
        "| Dimension | Value | Count |",
        "|---|---|---|",
    ]
    for key, count in sorted(by_priority.items()):
        lines.append(f"| Priority | {key} | {count} |")
    for key, count in sorted(by_status.items()):
        lines.append(f"| Status | {key} | {count} |")
    lines += [
        "",
        "## Ledger Rows",
        "",
        "| ID | Anchor | Priority | Status | Public Claims |",
        "|---|---|---|---|---|",
    ]
    for row in rows:
        lines.append(
            f"| `{row['id']}` | {row['standard_anchor']} | {row['priority']} | {row['status']} | {len(row['public_claims'])} |"
        )
    lines += [
        "",
        "## Follow-Up Source",
        "",
        "Rows not marked `supported-with-clause-evidence` are the initial follow-up backlog. Later PRs should split broad family rows into smaller clause-backed rows before strengthening public support claims.",
        "",
    ]
    return "\n".join(lines)


def pics_draft(data: dict) -> str:
    rows = data["rows"]
    data_links = [r for r in rows if r["standard_anchor"] in {"Annex J.2", "Annex J", "Annex J.4/J.5", "Annex J.5", "Annex J.7.5", "Annex J.8", "Annex U", "Clause 7", "Clause 9.3", "Annex AB.2", "Annex AB.2.4", "Annex AB.3.4", "Annex AB.5", "Annex AB.6.2", "Annex AB.6.3", "Annex AB.7"}]
    lines = header("Draft BACnet PICS Support Evidence")
    lines += [
        "This draft summarizes implementation evidence that may feed a future formal Protocol Implementation Conformance Statement. It intentionally stays below a certification claim.",
        "",
        "## Data Link And Network Rows",
        "",
        "| ID | Anchor | Status | Code Anchors |",
        "|---|---|---|---|",
    ]
    for row in data_links:
        lines.append(f"| `{row['id']}` | {row['standard_anchor']} | {row['status']} | {md_list(row['code_anchors'])} |")
    lines += [
        "",
        "## PICS/Profile Rows",
        "",
        "| ID | Anchor | Status | Notes |",
        "|---|---|---|---|",
    ]
    for row in rows:
        if row["id"] in {"BACNET-A-PICS", "BACNET-L-PROFILES", "BACNET-12-OBJECT-MODEL"}:
            lines.append(f"| `{row['id']}` | {row['standard_anchor']} | {row['status']} | {row['notes']} |")
    lines.append("")
    return "\n".join(lines)


def bibbs_draft(data: dict) -> str:
    groups: dict[str, list[dict]] = defaultdict(list)
    for row in data["rows"]:
        if row["id"] == "BACNET-K-BIBBS" or "service" in row["requirement_summary"].lower() or "tsm" in row["requirement_summary"].lower():
            groups[row["priority"]].append(row)
    lines = header("Draft BACnet BIBB Support Evidence")
    lines += [
        "This draft is a ledger-derived starting point for future Annex K BIBB mapping. Detailed BIBB claims require service-specific positive and negative tests.",
        "",
    ]
    for priority in sorted(groups):
        lines += [f"## {priority}", "", "| ID | Anchor | Status | Tests |", "|---|---|---|---|"]
        for row in groups[priority]:
            tests = row["positive_tests"] + row["negative_tests"]
            lines.append(f"| `{row['id']}` | {row['standard_anchor']} | {row['status']} | {md_list(tests)} |")
        lines.append("")
    return "\n".join(lines)


def generated(data: dict) -> dict[Path, str]:
    return {
        OUTPUTS["support"]: support_summary(data),
        OUTPUTS["pics"]: pics_draft(data),
        OUTPUTS["bibbs"]: bibbs_draft(data),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="fail if generated docs are stale")
    args = parser.parse_args()

    data = load_ledger()
    stale: list[Path] = []
    for path, content in generated(data).items():
        content = content.rstrip() + "\n"
        if args.check:
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                stale.append(path)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
    if stale:
        for path in stale:
            print(f"stale: {path.relative_to(ROOT)}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

//! Row tables of the standard ledger page, written from each row's `evidence`.
//!
//! `scripts/generate-conformance-docs.py` writes every table between
//! `ledger-rows` markers from the rows its BEGIN comment lists, and `--check`
//! flags a hand edit (#1191). These tests hold the structure: a row has
//! `evidence` exactly when one table lists it, and its table line shows the
//! row's anchor, priority and status from the JSON.
use super::*;

const BEGIN: &str = "<!-- BEGIN ledger-rows";
const BEGIN_CLOSE: &str = "-->";
const END: &str = "<!-- END ledger-rows -->";

/// Each marked table's listed row IDs and the lines between its markers.
fn marked_tables(page: &str) -> Vec<(Vec<&str>, Vec<&str>)> {
    let mut tables = Vec::new();
    let mut lines = page.lines();
    while let Some(line) = lines.next() {
        if line.starts_with(BEGIN) {
            let ids = lines.by_ref().take_while(|l| *l != BEGIN_CLOSE).collect();
            let body = lines.by_ref().take_while(|l| *l != END).collect();
            tables.push((ids, body));
        }
    }
    tables
}

#[test]
fn ledger_evidence_entries_are_trimmed_strings() {
    let data = ledger();
    for row in data["rows"].as_array().expect("rows should be an array") {
        let Some(evidence) = row.get("evidence") else {
            continue;
        };
        let entries = evidence
            .as_array()
            .unwrap_or_else(|| panic!("{} evidence must be an array", row["id"]));
        assert!(!entries.is_empty(), "{} evidence is empty", row["id"]);
        for entry in entries {
            let entry = entry.as_str().unwrap_or_default();
            assert!(
                !entry.is_empty() && entry.trim() == entry,
                "{} evidence entries must be non-empty strings without outer whitespace: {entry:?}",
                row["id"]
            );
        }
    }
}

#[test]
fn ledger_tables_list_each_evidence_row_once_with_its_json_cells() {
    let data = ledger();
    let rows = rows_by_id(&data);
    let mut listed = BTreeSet::new();
    for (ids, body) in marked_tables(STANDARD_LEDGER) {
        assert_eq!(
            body.len(),
            ids.len() + 2,
            "a marked table holds its header, its delimiter and one line per listed row: {ids:?}"
        );
        for (id, line) in ids.iter().zip(&body[2..]) {
            assert!(listed.insert(*id), "{id} is listed by more than one table");
            let row = rows
                .get(*id)
                .unwrap_or_else(|| panic!("{id} is listed by a table but is not a ledger row"));
            let cells = format!(
                "| `{id}` | {} | {} | `{}` | ",
                row["standard_anchor"].as_str().unwrap_or_default(),
                row["priority"].as_str().unwrap_or_default(),
                row["status"].as_str().unwrap_or_default()
            );
            assert!(
                line.starts_with(&cells),
                "{id}'s table line should show its JSON anchor, priority and status"
            );
        }
    }
    let with_evidence: BTreeSet<&str> = rows
        .iter()
        .filter(|(_, row)| row.get("evidence").is_some())
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(
        listed, with_evidence,
        "a row has evidence exactly when a ledger table lists it"
    );
}

#[test]
fn marked_tables_read_listed_ids_and_table_lines() {
    let page = "prose\n<!-- BEGIN ledger-rows: note\nA\nB\n-->\n| h |\n|---|\n| a |\n| b |\n<!-- END ledger-rows -->\nprose\n";
    assert_eq!(
        marked_tables(page),
        vec![(vec!["A", "B"], vec!["| h |", "|---|", "| a |", "| b |"])]
    );
}

//! Cross-check of the property-states choices we name against ASHRAE's XD
//! data (#2472).
//!
//! The choice tags above 254 travel through the extended-value choice, so
//! the stack names the ones the file lists (258 and 259 in revision 24) and
//! shows any other as a number. The file's `choice 0-BACnetPropertyStates`
//! block lists every alternative by tag, name and datatype. Our table in
//! `constructed/property_states.rs` must hold exactly the file's tags above
//! 254, spelled the same, each with the datatype the file gives, and the
//! enumeration behind each choice must be the one the file names. A misspelled
//! or stale entry, or a tag the file adds, fails.

use std::collections::BTreeMap;

use super::xd_crosscheck::{parse_xd, spelled_as, workspace_root};
use crate::constructed::{NAMED_EXTENDED_CHOICES, NamedExtendedChoice};

/// The file's alternatives of the property-states choice, by tag: name and
/// `Element[:datatype]`.
fn xd_alternatives(text: &str) -> BTreeMap<u32, (String, String)> {
    let mut found = BTreeMap::new();
    let mut inside = false;
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split(' ').collect();
        if words[0] == "choice" {
            inside = words.get(1) == Some(&"0-BACnetPropertyStates");
            continue;
        }
        if words[0] == "enum" || words[0] == "bits" {
            inside = false;
        }
        if !inside {
            continue;
        }
        assert_eq!(words.len(), 3, "not an alternative: {line}");
        let tag: u32 = words[0].parse().unwrap_or_else(|_| panic!("tag: {line}"));
        assert!(
            found
                .insert(tag, (words[1].to_owned(), words[2].to_owned()))
                .is_none(),
            "tag {tag} repeats"
        );
    }
    found
}

#[test]
fn named_extended_choices_agree_with_ashrae_xd() {
    let root = workspace_root();
    if !root.join("crates/bacnet-types/Cargo.toml").is_file() {
        println!("XD choice cross-check skipped: not built inside the repository");
        return;
    }
    let path = root.join("testdata/ashrae-xd/xd-data-types.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let alternatives = xd_alternatives(&text);
    assert!(
        alternatives.len() > 50,
        "the file's choice block has only {} alternatives",
        alternatives.len()
    );

    // The file's tags above 254 and ours are the same set.
    let theirs: Vec<(u32, &str, &str)> = alternatives
        .iter()
        .filter(|(tag, _)| **tag > 254)
        .map(|(tag, (name, datatype))| (*tag, name.as_str(), datatype.as_str()))
        .collect();
    assert_eq!(
        theirs.iter().map(|(t, _, _)| *t).collect::<Vec<_>>(),
        NAMED_EXTENDED_CHOICES
            .iter()
            .map(|c| c.tag)
            .collect::<Vec<_>>(),
        "extended choice tags differ from the file's"
    );
    for (choice, (tag, name, datatype)) in NAMED_EXTENDED_CHOICES.iter().zip(&theirs) {
        assert_eq!(choice.tag, *tag);
        assert!(
            spelled_as(&choice.name.replace('-', "_"), name),
            "choice {tag}: ours {:?}, the file {name:?}",
            choice.name
        );
        assert_eq!(
            format!("Enumerated:0-{}", choice.datatype),
            *datatype,
            "choice {tag}"
        );
    }

    // Each choice's value names are the file's table for its datatype.
    let tables = parse_xd(&text);
    for choice in NAMED_EXTENDED_CHOICES {
        let members = tables
            .get(&format!("0-{}", choice.datatype))
            .unwrap_or_else(|| panic!("the file has no table {}", choice.datatype));
        assert!(!members.is_empty());
        for (name, value) in members {
            let ours = choice
                .value_name(*value)
                .unwrap_or_else(|| panic!("{} names no value {value}", choice.name));
            assert!(spelled_as(ours, name), "{}: {ours} vs {name}", choice.name);
            assert_eq!(choice.value_from_name(name), Some(*value));
        }
    }
}

#[test]
fn names_resolve_both_ways() {
    for choice in NAMED_EXTENDED_CHOICES {
        assert_eq!(NamedExtendedChoice::from_tag(choice.tag), Some(choice));
        assert_eq!(NamedExtendedChoice::from_name(choice.name), Some(choice));
    }
    assert_eq!(NamedExtendedChoice::from_tag(260), None);
    assert_eq!(NamedExtendedChoice::from_name("response-includes"), None);
}

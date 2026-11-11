//! Cross-check of the built-in objects' conformance codes against ASHRAE's
//! machine-readable BACnet object types (#2452).
//!
//! `testdata/ashrae-xd/xd-object-types.txt` is a stripped copy of ASHRAE's XD
//! object-types file (each object type's properties with their datatype and
//! flags; Apache-2.0), regenerated with `scripts/xd_strip.py`. It is a
//! cross-check, not a source: each object's metadata rows stay hand-written.
//!
//! A row of the file is `O` when it says `optional=true`, otherwise `W` when
//! it says `writable=true`, and otherwise `R`. The rows of every catalogue
//! representative, and of a B/IP and a B/IPv6 Network Port, are compared
//! with the file's rows for their type:
//!
//! - a row both have: our base [`PropertyConformance`] against the file's code;
//! - a required row of the file: the representative must have it, since a
//!   required property is present on every object of the type;
//! - a row of ours: the file must list the property on the type.
//!
//! The file's optional rows a representative lacks are not differences: an
//! object may leave an optional property out. Each accepted difference is an
//! entry in `xd_conformance_allowlist.rs`; the test fails on a difference
//! without an entry, and on an entry that no longer applies or names a pair
//! that now agrees.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::NetworkPortObject;
use super::xd_conformance_allowlist::{Allowed, Category, allowed};
use crate::property_metadata::PropertyConformance;
use crate::traits::BACnetObject;

/// A conformance code: `R`, `W` or `O`.
type Code = &'static str;

/// The file's object types, by our object type, each with its codes by
/// property identifier.
type XdCodes = BTreeMap<ObjectType, BTreeMap<u32, Code>>;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Our name, in kebab form.
fn kebab(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

fn object_name(object_type: ObjectType) -> String {
    object_type
        .name()
        .map_or_else(|| object_type.to_raw().to_string(), kebab)
}

fn property_name(property: PropertyIdentifier) -> String {
    property
        .name()
        .map_or_else(|| property.to_raw().to_string(), kebab)
}

/// Letters and digits only, lowercased: `0-MultistateInputObject` and
/// `MULTI_STATE_INPUT` meet as `multistateinput`.
fn folded(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn object_type_for(xd_name: &str) -> Option<ObjectType> {
    let bare = xd_name.strip_prefix("0-")?.strip_suffix("Object")?;
    ObjectType::ALL_NAMED
        .iter()
        .find(|(name, _)| folded(name) == folded(bare))
        .map(|&(_, object_type)| object_type)
}

/// Parse the stripped object-types file into each row's conformance code.
fn parse_codes(text: &str) -> XdCodes {
    let mut objects = XdCodes::new();
    let mut current = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split(' ').collect();
        if words[0] == "object" {
            let object_type = object_type_for(words[1])
                .unwrap_or_else(|| panic!("line {}: no object type of ours", n + 1));
            assert!(objects.insert(object_type, BTreeMap::new()).is_none());
            current = Some(object_type);
            continue;
        }
        let id: u32 = words[0]
            .parse()
            .unwrap_or_else(|_| panic!("line {}: not a property row", n + 1));
        let flag = |key: &str| {
            words[3..]
                .iter()
                .find_map(|w| w.strip_prefix(key).and_then(|w| w.strip_prefix('=')))
        };
        let code = match (flag("optional"), flag("writable")) {
            (Some("true"), _) => "O",
            (_, Some("true")) => "W",
            _ => "R",
        };
        let object = current.unwrap_or_else(|| panic!("line {}: row before an object", n + 1));
        assert!(objects.get_mut(&object).unwrap().insert(id, code).is_none());
    }
    objects
}

fn our_code(conformance: PropertyConformance) -> Code {
    match conformance {
        PropertyConformance::RequiredRead => "R",
        PropertyConformance::RequiredWrite => "W",
        PropertyConformance::Optional => "O",
    }
}

/// One difference between a representative's rows and the file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Diff {
    /// Both have the row, with different codes.
    Code {
        object: String,
        property: String,
        ours: Code,
        xd: Code,
    },
    /// The file makes the property required; the representative lacks it.
    Missing { object: String, property: String },
    /// The representative has a row the file doesn't list on the type.
    OursOnly { object: String, property: String },
}

impl Diff {
    fn key(&self) -> (&str, &str) {
        match self {
            Self::Code {
                object, property, ..
            }
            | Self::Missing { object, property }
            | Self::OursOnly { object, property } => (object, property),
        }
    }
}

fn allows(entry: &Allowed, diff: &Diff) -> bool {
    let (object, property) = diff.key();
    let kind_matches = match (entry.category, diff) {
        (Category::LaterAddition, Diff::Missing { .. })
        | (Category::OursOnly, Diff::OursOnly { .. }) => true,
        (Category::LaterChange | Category::ToFix, Diff::Code { ours, xd, .. }) => {
            entry.ours == *ours && entry.xd == *xd
        }
        (Category::ToFix, Diff::Missing { .. } | Diff::OursOnly { .. }) => entry.ours.is_empty(),
        _ => false,
    };
    kind_matches && entry.objects.contains(&object) && entry.properties.contains(&property)
}

/// The catalogue's representatives, and the IP Network Ports: a port's rows
/// depend most on its configuration.
fn objects() -> Vec<Box<dyn BACnetObject>> {
    let mut objects = super::supported_representatives();
    objects.push(Box::new(
        NetworkPortObject::new_bip(2, "NP-BIP", Default::default()).unwrap(),
    ));
    objects.push(Box::new(
        NetworkPortObject::new_bip6(3, "NP-BIP6", Default::default()).unwrap(),
    ));
    objects
}

/// The differences, and how many rows both sides have.
fn diffs() -> (BTreeSet<Diff>, usize) {
    let path = workspace_root().join("testdata/ashrae-xd/xd-object-types.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let xd = parse_codes(&text);
    let mut diffs = BTreeSet::new();
    let mut compared = 0;
    for representative in objects() {
        let object_type = representative.object_identifier().object_type();
        let object = object_name(object_type);
        let theirs = xd
            .get(&object_type)
            .unwrap_or_else(|| panic!("no object type {object} in the file"));
        let rows = representative.property_metadata();
        for row in rows.iter() {
            let property = property_name(row.property_identifier);
            let ours = our_code(row.conformance);
            match theirs.get(&row.property_identifier.to_raw()) {
                None => {
                    diffs.insert(Diff::OursOnly {
                        object: object.clone(),
                        property,
                    });
                }
                Some(&xd) if xd != ours => {
                    diffs.insert(Diff::Code {
                        object: object.clone(),
                        property,
                        ours,
                        xd,
                    });
                }
                Some(_) => compared += 1,
            }
        }
        for (&id, &code) in theirs {
            let property = PropertyIdentifier::from_raw(id);
            if code != "O" && !rows.iter().any(|r| r.property_identifier == property) {
                diffs.insert(Diff::Missing {
                    object: object.clone(),
                    property: property_name(property),
                });
            }
        }
    }
    (diffs, compared)
}

#[test]
fn conformance_codes_agree_with_ashrae_xd_except_where_allowed() {
    // The data isn't in the published crate: built outside the repository,
    // there is nothing to compare against. Inside it, a missing file fails.
    if !workspace_root()
        .join("crates/bacnet-objects/Cargo.toml")
        .is_file()
    {
        println!("XD conformance cross-check skipped: not built inside the repository");
        return;
    }
    let (diffs, compared) = diffs();
    let allowed = allowed();
    let mut report = String::new();
    for d in diffs
        .iter()
        .filter(|d| !allowed.iter().any(|e| allows(e, d)))
    {
        writeln!(report, "not on the allow-list: {d:?}").unwrap();
    }
    for entry in &allowed {
        if entry.why.trim().is_empty() {
            writeln!(report, "allow-list entry without a reason: {entry:?}").unwrap();
        }
        let agree: Vec<String> = entry
            .pairs()
            .filter(|(o, p)| {
                !diffs
                    .iter()
                    .any(|d| allows(entry, d) && d.key() == (o.as_str(), p.as_str()))
            })
            .map(|(o, p)| format!("{o} {p}"))
            .collect();
        if !agree.is_empty() {
            writeln!(
                report,
                "allow-list entry names pairs that no longer differ {agree:?}: {entry:?}"
            )
            .unwrap();
        }
    }
    let mut by_category: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for entry in &allowed {
        let slot = by_category
            .entry(format!("{:?}", entry.category))
            .or_default();
        slot.0 += 1;
        slot.1 += diffs.iter().filter(|d| allows(entry, d)).count();
    }
    println!(
        "XD conformance cross-check: {compared} rows agree, {} differences",
        diffs.len()
    );
    for (category, (entries, count)) in &by_category {
        println!("  {category}: {entries} entries, {count} differences");
    }
    assert!(report.is_empty(), "{report}");
}

/// Every row a built-in object serves has a value type, standard or its
/// own. The value-type cross-check in bacnet-types accepts 2020 properties
/// with no standard value type as `not-modelled` only because nothing here
/// serves one.
#[test]
fn every_served_row_has_a_value_type() {
    let mut untyped = Vec::new();
    for object in objects() {
        let object_type = object.object_identifier().object_type();
        for row in object.property_metadata().iter() {
            if row.effective_value_type(object_type).is_none() {
                untyped.push(format!("{object_type:?} {:?}", row.property_identifier));
            }
        }
    }
    assert!(untyped.is_empty(), "rows without a value type: {untyped:?}");
}

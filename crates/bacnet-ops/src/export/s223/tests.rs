//! The Turtle writer: escaping, options, and the small structural check
//! of the text it writes ([`super::testing`]), written by hand so the
//! crate needs no RDF library. `scripts/check-223.py` validates the golden
//! files against the ontology's shapes.

use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;

pub(super) use super::testing::check_structure;
use super::testing::identifier_ok;
use super::turtle::literal;
use super::*;

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Device text can't end a literal or a line: quotes, backslashes and
/// controls are escaped, and everything outside printable ASCII goes by
/// its code point, astral characters by eight digits.
#[test]
fn literals_escape_device_text() {
    assert_eq!(literal("Zone Temp"), "\"Zone Temp\"");
    assert_eq!(literal(r#"a"b\c"#), r#""a\"b\\c""#);
    assert_eq!(literal("l1\nl2\r\t"), r#""l1\nl2\r\t""#);
    assert_eq!(literal("\u{7}\u{1b}[2J"), r#""\u0007\u001B[2J""#);
    assert_eq!(literal("Temp\u{e9}rature"), r#""Temp\u00E9rature""#);
    assert_eq!(literal("\u{202E}x"), r#""\u202Ex""#);
    assert_eq!(literal("\u{1F525}"), r#""\U0001F525""#);
    assert_eq!(literal("\u{7f}"), r#""\u007F""#);
}

/// U+0000 can't be in a string literal: it is written as U+FFFD, and the
/// header counts each one.
#[test]
fn nul_becomes_the_replacement_character() {
    assert_eq!(literal("a\0b"), r#""a\uFFFDb""#);
    let mut ai = InventoryObject::new(oid(ObjectType::ANALOG_INPUT, 1));
    ai.name = Some("Zone\0Temp".into());
    ai.description = Some("\0".into());
    let inventory = Inventory {
        device: 7,
        objects: vec![ai],
        ..Inventory::default()
    };
    let mut text = String::new();
    let counts = write_turtle(&inventory, &TurtleOptions::default(), &mut text).unwrap();
    // The name goes out twice (label and object-name), the description once.
    assert_eq!(counts.replaced, 3);
    assert!(
        text.contains("# 3 NUL characters in device text written as U+FFFD."),
        "{text}"
    );
    assert!(!text.contains('\0'));
    check_structure(&text);
}

/// An inventory with no object (none of the selected types) is a valid,
/// empty model whose header says so.
#[test]
fn no_object_of_the_selected_types() {
    let inventory = Inventory {
        device: 7,
        ..Inventory::default()
    };
    let mut text = String::new();
    let counts = write_turtle(&inventory, &TurtleOptions::default(), &mut text).unwrap();
    assert_eq!(counts, Counts::default());
    assert!(
        text.contains("# The device has no object of the selected types"),
        "{text}"
    );
    assert!(check_structure(&text).is_empty());
}

/// The shape's identifier pattern, as the test re-implements it.
#[test]
fn identifier_pattern() {
    for good in ["analog-input,3", "device,1234", "600,10"] {
        assert!(identifier_ok(good), "{good}");
    }
    for bad in [
        "analog-input,0",
        "device,0",
        "analog-input,03",
        "a b,1",
        ",1",
        "x,",
        "x",
    ] {
        assert!(!identifier_ok(bad), "{bad}");
    }
}

/// A write priority is 1 to 16; a base IRI is absolute, ends in `/` or
/// `#`, and holds nothing Turtle refuses inside `<...>`.
#[test]
fn options_are_checked() {
    assert!(WritePriority::new(0).is_err());
    assert!(WritePriority::new(17).is_err());
    assert_eq!(WritePriority::new(1).unwrap().get(), 1);
    assert_eq!(WritePriority::new(16).unwrap().get(), 16);
    for good in [
        "http://example.org/bacnet/",
        "urn:x:site#",
        "https://a.b/c-d/",
        "http://x/a%20b%2F/",
        "http://x/site#",
    ] {
        assert!(TurtleOptions::new(good, None).is_ok(), "{good}");
    }
    for (bad, says) in [
        ("example.org/bacnet/", "no scheme"),
        ("1http://x/", "no scheme"),
        ("", "no scheme"),
        ("http://example.org/bacnet", "must end in / or #"),
        ("http://example.org/a b/", "which an IRI can't"),
        ("http://example.org/a>b/", "which an IRI can't"),
        ("http://example.org/\"/", "which an IRI can't"),
        ("http://example.org/a`b/", "which an IRI can't"),
        ("http://x/a#b#", "more than one #"),
        ("http://x/%zz/", "malformed percent escape at byte 9"),
        ("http://x/%4/", "malformed percent escape"),
        ("http://x/a%", "malformed percent escape"),
        ("http://x/caf\u{e9}/", "non-ASCII character '\u{e9}'"),
    ] {
        let error = TurtleOptions::new(bad, None).unwrap_err();
        assert!(error.contains(says), "{bad}: {error}");
    }
    assert_eq!(TurtleOptions::default().base_iri(), DEFAULT_BASE_IRI);
    assert!(InventoryOptions::new(vec![]).is_err());
    assert!(InventoryOptions::new(vec![ObjectType::TREND_LOG]).is_err());
    assert_eq!(InventoryOptions::default().types, EXPORTED_TYPES);
}

/// The checker itself refuses what it should: an unterminated statement,
/// an undeclared prefix, a raw control character, instance 0.
#[test]
fn the_check_refuses_broken_text() {
    let good = "@prefix d: <http://x/> .\nd:a d:b \"c\" ;\n    d:e d:f .\n";
    assert_eq!(check_structure(good), ["d:a"]);
    for bad in [
        "@prefix d: <http://x/> .\nd:a d:b \"c\"\n",
        "@prefix d: <http://x/> .\nd:a e:b \"c\" .\n",
        "@prefix d: <http://x/> .\nd:a d:b \"c\u{7}\" .\n",
        "@prefix bacnet: <http://x/> .\nbacnet:a bacnet:object-identifier \"ai,0\" .\n",
        "@prefix d: <http://x/> .\nd:a d:b \"\\q\" .\n",
    ] {
        let result = std::panic::catch_unwind(|| check_structure(bad));
        assert!(result.is_err(), "{bad}");
    }
}

/// The counts in the header, and the minted kind capped for a device that
/// claims far more states than it names.
#[test]
fn counts_and_the_state_cap() {
    let mut huge = InventoryObject::new(oid(ObjectType::MULTI_STATE_INPUT, 1));
    huge.states = Some(u32::MAX);
    let mut unread = InventoryObject::new(oid(ObjectType::ANALOG_INPUT, 2));
    unread.error = Some("timeout".into());
    let trend = InventoryObject::new(oid(ObjectType::TREND_LOG, 1));
    let mut mapped = InventoryObject::new(oid(ObjectType::ANALOG_VALUE, 0));
    mapped.units = Some(EngineeringUnits::PERCENT);
    let unmapped = InventoryObject::new(oid(ObjectType::ANALOG_OUTPUT, 4));
    let inventory = Inventory {
        device: 7,
        objects: vec![huge, unread, trend, mapped, unmapped],
        ..Inventory::default()
    };
    let mut text = String::new();
    let counts = write_turtle(&inventory, &TurtleOptions::default(), &mut text).unwrap();
    assert_eq!(
        counts,
        Counts {
            exported: 3,
            skipped: 2,
            unmapped_units: 1,
            instance_zero: 1,
            replaced: 0,
        }
    );
    assert!(
        text.contains(
            "# 3 objects exported, 2 skipped, 1 with an unmapped unit, 1 identifier left out"
        ),
        "{text}"
    );
    assert!(
        text.contains("d:multi-state-input-1.states.1024 a"),
        "{text}"
    );
    assert!(
        !text.contains("d:multi-state-input-1.states.1025 "),
        "{text}"
    );
    check_structure(&text);
    assert_eq!(to_turtle(&inventory, &TurtleOptions::default()), text);
}

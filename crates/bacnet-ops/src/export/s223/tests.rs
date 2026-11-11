//! The Turtle writer: escaping, options, and the small structural check
//! of the text it writes ([`super::testing`]), written by hand so the
//! crate needs no RDF library. `scripts/check-223.py` validates the golden
//! files against the ontology's shapes.

use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
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
    let ai = Point::new(
        oid(ObjectType::ANALOG_INPUT, 1),
        "Zone\0Temp",
        Analog::new(EngineeringUnits::DEGREES_CELSIUS, Access::ReadOnly),
    )
    .with_description("\0");
    let inventory = Inventory::from(PointList::new(DeviceInfo::new(7), vec![ai]));
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
    let inventory = Inventory::new(DeviceInfo::new(7));
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

/// Every identifier string the export writes, every object type named or
/// by number, reads back through the parser every front end shares.
#[test]
fn identifiers_read_back_as_input() {
    for raw in 0..=1023 {
        let object_type = ObjectType::from_raw(raw);
        for instance in [0, 1, 3, ObjectIdentifier::MAX_INSTANCE] {
            let text = super::turtle::identifier(object_type, instance);
            assert_eq!(
                crate::parse::parse_object_specifier(&text),
                Ok((object_type, instance)),
                "{text}"
            );
        }
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
    assert!(
        EXPORTED_TYPES.contains(&ObjectType::INTEGER_VALUE),
        "read by default (#2456)"
    );
    let refused = InventoryOptions::new(vec![ObjectType::POSITIVE_INTEGER_VALUE]).unwrap_err();
    assert!(
        refused.ends_with("multi-state inputs, outputs and values, and integer values"),
        "{refused}"
    );
    assert_eq!(InventoryOptions::default().detail, Detail::Export);
    let full = InventoryOptions::new(vec![ObjectType::ANALOG_INPUT])
        .unwrap()
        .with_detail(Detail::Full);
    assert_eq!(full.detail, Detail::Full);
    assert_eq!(full.types, [ObjectType::ANALOG_INPUT]);
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
    let huge = Point::new(
        oid(ObjectType::MULTI_STATE_INPUT, 1),
        "huge",
        MultiState::new(u32::MAX, Access::ReadOnly),
    );
    let unread = Entry::Failed {
        object: oid(ObjectType::ANALOG_INPUT, 2),
        property: None,
        error: "timeout".into(),
    };
    let trend = Point::new(
        oid(ObjectType::TREND_LOG, 1),
        "trend",
        Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
    );
    let mapped = Point::new(
        oid(ObjectType::ANALOG_VALUE, 0),
        "mapped",
        Analog::new(EngineeringUnits::PERCENT, Access::ReadOnly),
    );
    let unmapped = Point::new(
        oid(ObjectType::ANALOG_OUTPUT, 4),
        "unmapped",
        Analog::new(
            EngineeringUnits::from_raw(300),
            Access::Commandable {
                relinquish_default: None,
            },
        ),
    );
    let mut inventory = Inventory::new(DeviceInfo::new(7));
    inventory.entries = vec![
        Entry::Point(huge),
        unread,
        Entry::Point(trend),
        Entry::Point(mapped),
        Entry::Point(unmapped),
    ];
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

/// Each entry the export can't write is skipped where it was, with a
/// comment saying which case it is, in text of ours only (the error's
/// device text never reaches it): a type the export doesn't map, whether
/// a point (a list built by hand can hold one) or a failed entry (as the
/// reader gives one); a read that failed; a property the type requires
/// that couldn't be read; a kind that isn't the type's family.
#[test]
fn each_skip_says_why() {
    let failed = |object_type, instance, property| Entry::Failed {
        object: oid(object_type, instance),
        property,
        error: "PROPERTY:UNKNOWN_PROPERTY\n@prefix evil".into(),
    };
    let mismatched = Point::new(
        oid(ObjectType::ANALOG_INPUT, 1),
        "mismatched",
        Binary::new(Access::ReadOnly),
    );
    let trend = Point::new(
        oid(ObjectType::TREND_LOG, 1),
        "trend",
        Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
    );
    let fits = Point::new(
        oid(ObjectType::BINARY_INPUT, 1),
        "fits",
        Binary::new(Access::ReadOnly),
    );
    let mut inventory = Inventory::new(DeviceInfo::new(7));
    inventory.entries = vec![
        Entry::Point(mismatched),
        Entry::Point(trend),
        failed(ObjectType::TREND_LOG, 2, None),
        failed(ObjectType::ANALOG_VALUE, 3, None),
        failed(
            ObjectType::MULTI_STATE_VALUE,
            4,
            Some(PropertyIdentifier::NUMBER_OF_STATES),
        ),
        failed(
            ObjectType::BINARY_OUTPUT,
            5,
            Some(PropertyIdentifier::OBJECT_NAME),
        ),
        Entry::Point(fits),
    ];
    let mut text = String::new();
    let counts = write_turtle(&inventory, &TurtleOptions::default(), &mut text).unwrap();
    assert_eq!((counts.exported, counts.skipped), (1, 6), "{text}");
    for line in [
        "# analog-input 1: skipped, its kind doesn't fit its type.",
        "# trend-log 1: skipped, its type isn't mapped.",
        "# trend-log 2: skipped, its type isn't mapped.",
        "# analog-value 3: skipped, its read failed.",
        "# multi-state-value 4: skipped, no readable number-of-states, which its type requires.",
        "# binary-output 5: skipped, no readable object-name, which its type requires.",
    ] {
        assert!(text.lines().any(|l| l == line), "{line}\n{text}");
    }
    assert!(
        !text.contains("d:analog-input-1.") && !text.contains("evil"),
        "{text}"
    );
    check_structure(&text);
}

/// A point list and an inventory carry the same points: a list made an
/// inventory gives the list back, and a failed entry is left out of it.
#[test]
fn an_inventory_and_its_point_list() {
    let device = DeviceInfo::new(7)
        .with_name("plant")
        .with_description("Plant room");
    let point = Point::new(
        oid(ObjectType::BINARY_VALUE, 1),
        "occupied",
        Binary::new(Access::Writable).with_active_text("Occupied"),
    );
    let list = PointList::new(device.clone(), vec![point.clone()]);
    let mut inventory = Inventory::from(list.clone());
    assert_eq!(inventory.point_list(), list);
    let failed = Entry::Failed {
        object: oid(ObjectType::ANALOG_INPUT, 2),
        property: Some(PropertyIdentifier::OBJECT_NAME),
        error: "object-name: OBJECT:UNKNOWN_OBJECT".into(),
    };
    inventory.entries.insert(0, failed.clone());
    assert_eq!(inventory.point_list(), list);
    assert_eq!(inventory.points().collect::<Vec<_>>(), [&point]);
    assert_eq!(failed.object(), oid(ObjectType::ANALOG_INPUT, 2));
    assert_eq!(failed.error(), Some("object-name: OBJECT:UNKNOWN_OBJECT"));
    assert_eq!(failed.point(), None);
    let entry = Entry::Point(point.clone());
    assert_eq!((entry.object(), entry.error()), (point.object, None));
    assert_eq!(entry.point(), Some(&point));
    // A Writable value is written as a value that isn't commandable, in
    // Turtle as well formed as a read device's.
    let text = to_turtle(&inventory, &TurtleOptions::default());
    assert!(
        text.contains("d:binary-value-1.present-value a s223:EnumerableProperty ;"),
        "{text}"
    );
    check_structure(&text);
}

/// An Integer Value is written as an analog value is (#2456): actuatable
/// when commandable, plainly quantifiable when not, with its QUDT terms or,
/// for a unit the table doesn't map, the unknown quantity kind, counted in
/// the header, and at instance 0 without its identifier string, as any
/// object. A kind that isn't its type's family is skipped either way: an
/// analog kind on an Integer Value, an integer kind on an Analog Value.
#[test]
fn an_integer_value_maps_as_an_analog_value_does() {
    let integer = |instance, name: &str, units, access| {
        Entry::Point(Point::new(
            oid(ObjectType::INTEGER_VALUE, instance),
            name,
            Integer::new(units, access),
        ))
    };
    let commanded = Access::Commandable {
        relinquish_default: None,
    };
    let analog_kind = Point::new(
        oid(ObjectType::INTEGER_VALUE, 4),
        "analog kind",
        Analog::new(EngineeringUnits::PERCENT, Access::ReadOnly),
    );
    let integer_kind = Point::new(
        oid(ObjectType::ANALOG_VALUE, 5),
        "integer kind",
        Integer::new(EngineeringUnits::PERCENT, Access::ReadOnly),
    );
    let mut inventory = Inventory::new(DeviceInfo::new(7));
    inventory.entries = vec![
        integer(1, "count", EngineeringUnits::NO_UNITS, commanded),
        integer(
            2,
            "total",
            EngineeringUnits::KILOWATT_HOURS,
            Access::Writable,
        ),
        integer(
            0,
            "vendor",
            EngineeringUnits::from_raw(300),
            Access::ReadOnly,
        ),
        Entry::Point(analog_kind),
        Entry::Point(integer_kind),
    ];
    let options = TurtleOptions::new(DEFAULT_BASE_IRI, Some(WritePriority::new(9).unwrap()));
    let mut text = String::new();
    let counts = write_turtle(&inventory, &options.unwrap(), &mut text).unwrap();
    assert_eq!(
        (
            counts.exported,
            counts.skipped,
            counts.unmapped_units,
            counts.instance_zero
        ),
        (3, 2, 1, 1),
        "{text}"
    );
    for line in [
        "d:integer-value-1.present-value a s223:QuantifiableActuatableProperty ;",
        "    qudt:hasUnit unit:UNITLESS ;",
        "d:integer-value-2.present-value a s223:QuantifiableProperty ;",
        "    qudt:hasUnit unit:KiloW-HR ;",
        "    qudt:hasQuantityKind quantitykind:Energy ;",
        "d:integer-value-0.present-value a s223:QuantifiableProperty ;",
        "    # instance 0: no object-identifier, which the draft's pattern refuses",
        "    rdfs:comment \"BACnet units 300: no QUDT mapping in this export\" ;",
        "    qudt:hasQuantityKind quantitykind:Unknown ;",
        "    bacnet:object-identifier \"integer-value,1\" ;",
        "# integer-value 4: skipped, its kind doesn't fit its type.",
        "# analog-value 5: skipped, its kind doesn't fit its type.",
    ] {
        assert!(text.lines().any(|l| l == line), "{line}\n{text}");
    }
    assert_eq!(text.matches("priority-for-writing").count(), 1, "{text}");
    check_structure(&text);
}

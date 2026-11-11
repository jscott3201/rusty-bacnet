//! Golden Turtle files rendered from fixed inventories, one per case the
//! design note names (#2436). `scripts/check-223.py` validates the same
//! files against the vendored ontology's shapes.
//!
//! `BACNET_S223_BLESS=1 cargo nextest run -p bacnet-ops golden` rewrites
//! them after a deliberate change to the output.

use std::path::PathBuf;

use bacnet_types::enums::{EngineeringUnits, ObjectType};

use super::tests::{check_structure, oid};
use super::*;

/// An object with `name`, nothing else read yet.
fn named(object_type: ObjectType, instance: u32, name: &str) -> InventoryObject {
    InventoryObject {
        name: Some(name.into()),
        ..InventoryObject::new(oid(object_type, instance))
    }
}

fn device(instance: u32, objects: Vec<InventoryObject>) -> Inventory {
    Inventory {
        device: instance,
        device_name: Some("AHU-1 Controller".into()),
        objects,
        stopped: None,
    }
}

/// Each golden case: its file, its inventory and its options.
fn cases() -> Vec<(&'static str, Inventory, TurtleOptions)> {
    let plain = TurtleOptions::default();
    let priority =
        TurtleOptions::new(DEFAULT_BASE_IRI, Some(WritePriority::new(8).unwrap())).unwrap();

    let mut zone = named(ObjectType::ANALOG_INPUT, 3, "Zone Temp");
    zone.description = Some("Zone temperature, room 101".into());
    zone.units = Some(EngineeringUnits::DEGREES_FAHRENHEIT);

    let mut fan = named(ObjectType::BINARY_OUTPUT, 1, "Supply Fan Start");
    fan.priority_array = true;

    let mut mode = named(ObjectType::MULTI_STATE_VALUE, 2, "Fan Mode");
    mode.states = Some(3);
    mode.state_texts = vec!["Off".into(), String::new(), "High".into()];
    mode.priority_array = true;

    let mut setpoint = named(ObjectType::ANALOG_VALUE, 1, "Zone Setpoint");
    setpoint.units = Some(EngineeringUnits::DEGREES_FAHRENHEIT);
    setpoint.priority_array = true;
    let mut flow = named(ObjectType::ANALOG_VALUE, 2, "Flow Total");
    flow.units = Some(EngineeringUnits::CUBIC_FEET_PER_MINUTE);
    let occupied = named(ObjectType::BINARY_VALUE, 3, "Occupied");

    let mut first = named(ObjectType::ANALOG_INPUT, 0, "Outside Air");
    first.units = Some(EngineeringUnits::DEGREES_CELSIUS);

    let mut days = named(ObjectType::ANALOG_VALUE, 5, "Heating Degree Days");
    days.units = Some(EngineeringUnits::DEGREE_DAYS_CELSIUS);
    let unread = named(ObjectType::ANALOG_INPUT, 6, "Units Unread");
    let mut proprietary = named(ObjectType::ANALOG_INPUT, 7, "Vendor Units");
    proprietary.units = Some(EngineeringUnits::from_raw(300));

    let mut hostile = named(
        ObjectType::ANALOG_INPUT,
        9,
        "Say \"hi\" \\ to\nRoom\t101\u{7}\u{1b}[2J",
    );
    hostile.description = Some("Temp\u{e9}rature \u{202E}evil\u{2066} \u{1F525}\0".into());
    hostile.units = Some(EngineeringUnits::DEGREES_CELSIUS);
    let mut hostile_states = named(ObjectType::MULTI_STATE_INPUT, 4, "Mode \"A\"");
    hostile_states.states = Some(2);
    hostile_states.state_texts = vec!["\"off\" .".into(), "on\r\n# not a comment".into()];
    let mut hostile_device = device(1234, vec![hostile, hostile_states]);
    hostile_device.device_name = Some("Bldg \u{2028}\"B\" \\ \u{feff}".into());

    let mut failed = named(ObjectType::ANALOG_INPUT, 1, "Unreadable");
    failed.error = Some("PROPERTY:UNKNOWN_PROPERTY\n# not a comment".into());
    let trend = named(ObjectType::TREND_LOG, 1, "Not Mapped");
    let mut skipped = device(
        1234,
        vec![
            failed,
            trend,
            named(ObjectType::BINARY_INPUT, 2, "Filter Alarm"),
        ],
    );
    skipped.stopped = Some("analog-input,9: timeout\n@prefix".into());

    vec![
        ("analog-input.ttl", device(1234, vec![zone]), plain.clone()),
        (
            "binary-output-priority.ttl",
            device(1234, vec![fan]),
            priority.clone(),
        ),
        (
            "multi-state-value.ttl",
            device(1234, vec![mode]),
            plain.clone(),
        ),
        (
            "value-priority-array.ttl",
            device(1234, vec![setpoint, flow, occupied]),
            priority,
        ),
        ("instance-zero.ttl", device(0, vec![first]), plain.clone()),
        (
            "unmapped-unit.ttl",
            device(1234, vec![days, unread, proprietary]),
            plain.clone(),
        ),
        ("hostile-strings.ttl", hostile_device, plain.clone()),
        ("skipped.ttl", skipped, plain),
    ]
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/s223")
        .join(name)
}

/// Each case renders exactly its golden file, which passes the structural
/// check; with `BACNET_S223_BLESS=1` the files are rewritten instead.
#[test]
fn golden_files_match() {
    let bless = std::env::var_os("BACNET_S223_BLESS").is_some_and(|v| v == "1");
    for (name, inventory, options) in cases() {
        let text = to_turtle(&inventory, &options);
        check_structure(&text);
        let path = golden(name);
        if bless {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}; bless with BACNET_S223_BLESS=1", path.display()));
        assert_eq!(text, want, "{name} changed; bless with BACNET_S223_BLESS=1");
    }
}

/// What the cases are about, pinned beside the golden text so a bless
/// can't quietly lose it.
#[test]
fn golden_cases_hold_what_they_test() {
    let texts: Vec<(&str, String)> = cases()
        .into_iter()
        .map(|(name, inventory, options)| (name, to_turtle(&inventory, &options)))
        .collect();
    let text = |name: &str| &texts.iter().find(|(n, _)| *n == name).unwrap().1;

    let ai = text("analog-input.ttl");
    assert!(ai.contains("a s223:QuantifiableObservableProperty ;"));
    assert!(ai.contains("qudt:hasUnit unit:DEG_F ;"));
    assert!(ai.contains("qudt:hasQuantityKind quantitykind:Temperature ;"));
    assert!(!ai.contains("hasValue"), "a reference, never a value");

    let bo = text("binary-output-priority.ttl");
    assert!(bo.contains("a s223:EnumeratedActuatableProperty ;"));
    assert!(bo.contains("s223:hasEnumerationKind s223:Binary-Logical ;"));
    assert!(bo.contains("bacnet:priority-for-writing \"8\"^^xsd:nonNegativeInteger ."));

    let msv = text("multi-state-value.ttl");
    assert!(msv.contains("d:multi-state-value-2.states.2 a s223:Class"));
    assert!(
        msv.contains("rdfs:label \"state 2\" ."),
        "an empty text is named by number"
    );
    assert!(!msv.contains("states.4"));

    let values = text("value-priority-array.ttl");
    assert!(
        values.contains("d:analog-value-1.present-value a s223:QuantifiableActuatableProperty")
    );
    assert!(values.contains("d:analog-value-2.present-value a s223:QuantifiableProperty"));
    assert!(values.contains("d:binary-value-3.present-value a s223:EnumerableProperty"));
    assert_eq!(
        values.matches("priority-for-writing").count(),
        1,
        "only the commandable one"
    );

    let zero = text("instance-zero.ttl");
    assert!(!zero.contains("bacnet:object-identifier"));
    assert!(!zero.contains("bacnet:device-identifier"));
    assert!(zero.contains("bacnet:object-name \"Outside Air\""));
    assert!(zero.contains("2 identifiers left out for instance 0"));

    let unmapped = text("unmapped-unit.ttl");
    assert!(!unmapped.contains("qudt:hasUnit"), "no unit is claimed");
    assert_eq!(
        unmapped
            .matches("qudt:hasQuantityKind quantitykind:Unknown ;")
            .count(),
        3,
        "the draft wants a quantity kind on each"
    );
    assert!(unmapped.contains("BACnet units degree-days-celsius: no QUDT mapping"));
    assert!(unmapped.contains("BACnet units 300: no QUDT mapping"));
    assert!(unmapped.contains("3 with an unmapped unit"));

    let hostile = text("hostile-strings.ttl");
    assert!(hostile.is_ascii());
    assert!(hostile.contains("\\U0001F525\\uFFFD\""), "NUL as U+FFFD");
    assert!(hostile.contains("# 1 NUL character in device text written as U+FFFD."));
    assert!(hostile.contains(r#"rdfs:label "Say \"hi\" \\ to\nRoom\t101\u0007\u001B[2J" ;"#));
    assert!(hostile.contains("\\u202Eevil\\u2066 \\U0001F525"));

    let skipped = text("skipped.ttl");
    assert!(skipped.contains("# analog-input 1: skipped, its read failed."));
    assert!(skipped.contains("# trend-log 1: skipped, its type isn't mapped."));
    assert!(skipped.contains("1 object exported, 2 skipped"));
    assert!(
        skipped
            .contains("# Incomplete: the reading stopped early (analog-input,9: timeout?@prefix).")
    );
}

/// The vendored ontology still declares the namespaces the writer emits
/// for the external reference: a refresh that moves the `bacnet:` vocabulary
/// (to a later edition's, say) fails here until the writer moves with it.
/// A plain text search; the repository has no RDF parser.
#[test]
fn vendored_ontology_declares_the_written_namespaces() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/open223/223p.ttl");
    let ontology =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let written = |prefix: &str| {
        super::turtle::PREFIXES
            .iter()
            .find(|(p, _)| *p == prefix)
            .map(|(_, iri)| *iri)
            .unwrap()
    };
    for prefix in ["bacnet", "s223"] {
        let declaration = format!("@prefix {prefix}: <{}> .", written(prefix));
        assert!(
            ontology.lines().any(|line| line == declaration),
            "{} no longer declares {declaration}",
            path.display()
        );
    }
    // The external-reference shape's paths are in that vocabulary.
    let shape = ontology
        .split("s223:BACnetExternalReferenceShape a sh:NodeShape")
        .nth(1)
        .expect("the external-reference shape");
    let shape = shape.split("\n\n").next().unwrap();
    for property in [
        "device-identifier",
        "device-name",
        "object-identifier",
        "object-name",
        "property-identifier",
        "priority-for-writing",
    ] {
        assert!(
            shape.contains(&format!("sh:path bacnet:{property}")),
            "the shape has no bacnet:{property} path"
        );
    }
}

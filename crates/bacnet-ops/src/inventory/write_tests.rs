//! The inventory's JSON and CSV against golden files
//! (`crates/bacnet-ops/tests/inventory/`), and the CSV's quoting.
//!
//! `BACNET_INVENTORY_BLESS=1 cargo nextest run -p bacnet-ops golden`
//! rewrites the files after a deliberate change to the output.

use std::path::PathBuf;
use std::time::Duration;

use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::record::record;
use super::test_lab::{oid, refused, row, text};
use super::*;
use crate::client::ReadVia;
use crate::format::RawRow;
use crate::reading::readings;

fn address() -> DeviceAddress {
    "192.168.1.10:47808".parse().unwrap()
}

fn point(object_type: ObjectType, instance: u32, rows: Vec<RawRow>) -> PointRecord {
    record(&address(), 1234, oid(object_type, instance), rows)
}

fn flags() -> RawRow {
    let bits = PropertyValue::BitString {
        unused_bits: 4,
        data: vec![0b0100_0000],
    };
    row(P::STATUS_FLAGS, bits)
}

/// An inventory with a record of each shape the writers meet: units and
/// a value, state texts of both kinds, commas, quotes, line feeds and
/// bars in device text, a problem, a bare record of an unknown type, and
/// a reading that stopped.
fn sample() -> Inventory {
    let device = oid(ObjectType::DEVICE, 1234);
    let summary = vec![
        row(P::OBJECT_NAME, text("AHU-1, \"East\"")),
        row(P::VENDOR_NAME, text("Acme")),
        row(P::VENDOR_IDENTIFIER, PropertyValue::Unsigned(42)),
        refused(P::MODEL_NAME),
    ];
    let summary = summary.into_iter().map(|row| (device, None, row)).collect();
    let zone = vec![
        row(P::OBJECT_NAME, text("Zone Temp")),
        row(P::DESCRIPTION, text("Room 101, east")),
        row(P::UNITS, PropertyValue::Enumerated(64)),
        row(P::PRESENT_VALUE, PropertyValue::Real(72.5)),
        flags(),
    ];
    let fan = vec![
        row(P::OBJECT_NAME, text("Fan \"A\"")),
        row(P::DESCRIPTION, text("line one\nline two")),
        row(P::ACTIVE_TEXT, text("On")),
        row(P::INACTIVE_TEXT, text("Off|Standby")),
        row(P::PRIORITY_ARRAY, PropertyValue::Null),
        row(P::RELINQUISH_DEFAULT, PropertyValue::Enumerated(0)),
        row(P::PRESENT_VALUE, PropertyValue::Enumerated(1)),
    ];
    let states = vec![text("Off"), text("Low, slow"), text("High\\er")];
    let mode = vec![
        row(P::OBJECT_NAME, text("Mode")),
        row(P::NUMBER_OF_STATES, PropertyValue::Unsigned(3)),
        row(P::STATE_TEXT, PropertyValue::List(states)),
        row(P::PRESENT_VALUE, PropertyValue::Unsigned(2)),
    ];
    let setpoint = vec![
        row(P::OBJECT_NAME, text("=Setpoint")),
        refused(P::DESCRIPTION),
        refused(P::UNITS),
        refused(P::PRIORITY_ARRAY),
        row(P::COV_INCREMENT, PropertyValue::Real(0.5)),
        row(P::MIN_PRES_VALUE, PropertyValue::Real(-40.0)),
        row(P::MAX_PRES_VALUE, PropertyValue::Real(120.0)),
    ];
    let vendor = vec![row(P::OBJECT_NAME, text("Caf\u{e9} \u{1F525}"))];
    // Device text a spreadsheet would run: a name and a description, a
    // state text that leads its cell, and a string value. The numbers
    // beside them start with a minus sign and are left alone.
    let formula = vec![
        row(P::OBJECT_NAME, text("+SUM(A1)")),
        row(P::DESCRIPTION, text("-2+3")),
        row(P::UNITS, PropertyValue::Enumerated(62)),
        row(P::MIN_PRES_VALUE, PropertyValue::Real(-40.0)),
        row(P::PRESENT_VALUE, PropertyValue::Real(-12.5)),
    ];
    let at_sign = vec![
        row(P::OBJECT_NAME, text("@cmd")),
        row(P::DESCRIPTION, text("\tindented")),
        row(P::INACTIVE_TEXT, text("=off")),
        row(P::ACTIVE_TEXT, text("on")),
    ];
    let string = vec![
        row(P::OBJECT_NAME, text("Note")),
        row(P::PRESENT_VALUE, text("=HYPERLINK(\"x\")")),
    ];
    Inventory {
        address: address(),
        device: DeviceSummary {
            device: 1234,
            readings: readings(summary),
            via: ReadVia::Rpm,
        },
        cov: CovSupport::Offered,
        points: vec![
            point(ObjectType::ANALOG_INPUT, 1, zone),
            point(ObjectType::BINARY_VALUE, 3, fan),
            point(ObjectType::MULTI_STATE_VALUE, 2, mode),
            point(ObjectType::ANALOG_VALUE, 9, setpoint),
            point(ObjectType::from_raw(700), 1, vendor),
            point(ObjectType::TREND_LOG, 1, Vec::new()),
            point(ObjectType::ANALOG_VALUE, 10, formula),
            point(ObjectType::BINARY_INPUT, 4, at_sign),
            point(ObjectType::CHARACTERSTRING_VALUE, 1, string),
            // A formula behind a leading space.
            point(
                ObjectType::TREND_LOG,
                2,
                vec![row(P::OBJECT_NAME, text(" =1+1"))],
            ),
        ],
        unread: vec![oid(ObjectType::ANALOG_INPUT, 2)],
        stopped: Some(OpsError::at(
            &address(),
            Error::Timeout(Duration::from_secs(6)),
        )),
    }
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/inventory")
        .join(name)
}

/// The JSON and the CSV of the sample are exactly their golden files;
/// with `BACNET_INVENTORY_BLESS=1` the files are rewritten instead.
#[test]
fn golden_files_match() {
    let bless = std::env::var_os("BACNET_INVENTORY_BLESS").is_some_and(|v| v == "1");
    let inventory = sample();
    let json = format!("{}\n", inventory.to_json());
    for (name, text) in [
        ("sample.json", json),
        ("sample.csv", inventory.to_csv(&CsvOptions::new())),
    ] {
        let path = golden(name);
        if bless {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}; bless with BACNET_INVENTORY_BLESS=1",
                path.display()
            )
        });
        assert_eq!(
            text, want,
            "{name} changed; bless with BACNET_INVENTORY_BLESS=1"
        );
    }
}

/// What the JSON is for, pinned beside the golden text so a bless can't
/// quietly lose it: the field names, a value's two forms, the object and
/// units shapes, a problem as data, and device text exactly.
#[test]
fn the_json_holds_typed_fields() {
    let json: serde_json::Value = serde_json::from_str(&sample().to_json()).unwrap();
    let keys = |value: &serde_json::Value| -> Vec<String> {
        value.as_object().unwrap().keys().cloned().collect()
    };
    let mut top = keys(&json);
    top.sort();
    assert_eq!(
        top,
        ["address", "cov", "device", "points", "stopped", "unread"]
    );
    assert_eq!(json["address"], "192.168.1.10:47808");
    assert_eq!(json["cov"], "offered");
    assert_eq!(json["device"]["instance"], 1234);
    assert_eq!(json["device"]["name"], "AHU-1, \"East\"");
    let properties = &json["device"]["properties"];
    assert_eq!(properties["vendor-identifier"]["decoded"], 42);
    assert_eq!(
        properties["model-name"]["error"],
        "PROPERTY:UNKNOWN_PROPERTY"
    );
    assert_eq!(json["stopped"], "request timed out after 6s");
    assert_eq!(json["unread"][0]["type"], "analog-input");

    let zone = &json["points"][0];
    let mut fields = keys(zone);
    fields.sort();
    let expected = [
        "commandable",
        "cov_increment",
        "description",
        "device",
        "device_instance",
        "max_present_value",
        "min_present_value",
        "name",
        "number_of_states",
        "object",
        "present_value",
        "problems",
        "relinquish_default",
        "state_texts",
        "status_flags",
        "units",
    ];
    assert_eq!(fields, expected, "every record has every field");
    assert_eq!(zone["device"], "192.168.1.10:47808");
    assert_eq!(zone["device_instance"], 1234);
    let object = serde_json::json!({"type": "analog-input", "type_number": 0, "instance": 1});
    assert_eq!(zone["object"], object);
    let units = serde_json::json!({"value": 64, "name": "degrees-fahrenheit", "symbol": "°F"});
    assert_eq!(zone["units"], units);
    assert_eq!(zone["present_value"]["value"], "72.5 °F");
    assert_eq!(zone["present_value"]["decoded"], 72.5);
    assert_eq!(zone["status_flags"]["decoded"]["names"][0], "fault");
    assert_eq!(zone["commandable"], false);
    assert!(zone["relinquish_default"].is_null() && zone["number_of_states"].is_null());

    let fan = &json["points"][1];
    assert_eq!(fan["name"], "Fan \"A\"");
    assert_eq!(fan["description"], "line one\nline two");
    assert_eq!(fan["state_texts"]["inactive"], "Off|Standby");
    assert_eq!(fan["commandable"], true);
    assert_eq!(fan["present_value"]["decoded"]["name"], "active");
    let mode = &json["points"][2];
    assert_eq!(mode["state_texts"]["states"][1], "Low, slow");
    assert_eq!(mode["number_of_states"], 3);
    let setpoint = &json["points"][3];
    let problem = serde_json::json!({"property": "units", "error": "PROPERTY:UNKNOWN_PROPERTY"});
    assert_eq!(setpoint["problems"], serde_json::json!([problem]));
    assert_eq!(setpoint["cov_increment"]["decoded"], 0.5);
    let bare = &json["points"][5];
    assert!(bare["name"].is_null() && bare["units"].is_null());
    assert_eq!(bare["problems"], serde_json::json!([]));
}

/// The CSV's header is the documented one, every row has its sixteen
/// fields, and a field holding a comma, a quote or a line break is quoted
/// with its quotes doubled; a bar inside a state text is escaped.
#[test]
fn the_csv_quotes_what_it_must() {
    let csv = sample().to_csv(&CsvOptions::new());
    let mut lines = csv.split('\n');
    assert_eq!(lines.next(), Some(CSV_HEADER));
    assert_eq!(CSV_HEADER.split(',').count(), 16);
    assert!(csv.ends_with('\n'));
    let rows = parse(&csv);
    assert_eq!(rows.len(), 1 + 10);
    assert!(rows.iter().all(|row| row.len() == 16), "{rows:?}");
    let zone = &rows[1];
    assert_eq!(zone[..3], ["192.168.1.10:47808", "1234", "analog-input:1"]);
    assert_eq!(
        zone[3..6],
        ["Zone Temp", "Room 101, east", "degrees-fahrenheit"]
    );
    assert_eq!((zone[8].as_str(), zone[13].as_str()), ("false", "72.5 °F"));
    assert!(csv.contains(",\"Room 101, east\","), "a comma: {csv}");
    let fan = &rows[2];
    assert_eq!(fan[3], "Fan \"A\"");
    assert!(csv.contains(",\"Fan \"\"A\"\"\","), "quotes doubled: {csv}");
    assert_eq!(fan[4], "line one\nline two");
    assert_eq!(fan[6], "Off\\|Standby|On", "inactive, then active");
    assert_eq!(fan[13], "active (On)");
    let mode = &rows[3];
    assert_eq!(mode[6], "Off|Low, slow|High\\\\er");
    assert_eq!(mode[7], "3");
    let setpoint = &rows[4];
    assert_eq!(setpoint[3], "'=Setpoint", "guarded");
    assert_eq!(setpoint[11], "-40", "a number is left alone");
    assert_eq!(setpoint[15], "units: PROPERTY:UNKNOWN_PROPERTY");
    assert_eq!(rows[6][3..8], ["", "", "", "", ""], "nothing read: empty");

    // A carriage return is quoted too, kept as it is.
    let rows = vec![row(P::OBJECT_NAME, text("a\r\nb"))];
    let mut out = String::new();
    point(ObjectType::TREND_LOG, 1, rows)
        .write_csv(&mut out, &CsvOptions::new())
        .unwrap();
    assert!(out.contains(",\"a\r\nb\","), "{out:?}");
}

/// The rows of `csv`, each its fields, as RFC 4180 reads them.
fn parse(csv: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = csv.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quoted, ch) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, ch) => field.push(ch),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (false, ch) => field.push(ch),
        }
    }
    rows
}

/// A cell of device text that a spreadsheet would run as a formula (it
/// starts with `=`, `+`, `-`, `@`, a tab or a carriage return) is written
/// behind an apostrophe: names, descriptions and the state texts' cell.
/// Numbers and the cells of our own wording are not, and
/// [`CsvOptions::exact`] writes everything as the device sent it. The
/// JSON is always exact.
#[test]
fn the_csv_guards_device_text_that_reads_as_a_formula() {
    let inventory = sample();
    let guarded = parse(&inventory.to_csv(&CsvOptions::new()));
    let exact = parse(&inventory.to_csv(&CsvOptions::new().with_exact(true)));
    assert_ne!(guarded, exact);
    let (formula, at_sign, string, spaced) = (7, 8, 9, 10);
    assert_eq!(guarded[spaced][3], "' =1+1", "before the original text");
    assert_eq!(exact[spaced][3], " =1+1");
    assert_eq!(guarded[formula][3..5], ["'+SUM(A1)", "'-2+3"]);
    assert_eq!(exact[formula][3..5], ["+SUM(A1)", "-2+3"]);
    for rows in [&guarded, &exact] {
        assert_eq!(rows[formula][11], "-40 °C", "Min_Pres_Value");
        assert_eq!(rows[formula][13], "-12.5 °C", "Present_Value");
        assert_eq!(rows[formula][2], "analog-value:10");
    }
    assert_eq!(guarded[at_sign][3..5], ["'@cmd", "'\tindented"]);
    assert_eq!(
        guarded[at_sign][6], "'=off|on",
        "the joined cell's first character"
    );
    assert_eq!(exact[at_sign][6], "=off|on");
    // A string value's text is written in quotes, so its cell starts with
    // one and reads as text either way.
    for rows in [&guarded, &exact] {
        assert_eq!(rows[string][13], "\"=HYPERLINK(\"x\")\"", "a string value");
    }
    // Each guarded character, and an ordinary one beside them.
    for (lead, guard) in [
        ('=', true),
        ('+', true),
        ('-', true),
        ('@', true),
        ('\t', true),
        ('\r', true),
        ('A', false),
        ('\'', false),
    ] {
        // Behind leading spaces and line breaks too, which don't hide it. A
        // tab or a carriage return there is itself such white space, and
        // counts only as the cell's very first character.
        let spaced = format!(" \n {lead}x");
        let guard_spaced = guard && !lead.is_whitespace();
        let rows = vec![row(P::OBJECT_NAME, text(&spaced))];
        let record = point(ObjectType::TREND_LOG, 1, rows);
        let mut out = String::new();
        record.write_csv(&mut out, &CsvOptions::new()).unwrap();
        let expected = if guard_spaced {
            format!("'{spaced}")
        } else {
            spaced.clone()
        };
        assert_eq!(parse(&out)[0][3], expected, "{lead:?} after whitespace");
        let name = format!("{lead}x");
        let record = point(
            ObjectType::TREND_LOG,
            1,
            vec![row(P::OBJECT_NAME, text(&name))],
        );
        let mut out = String::new();
        record.write_csv(&mut out, &CsvOptions::new()).unwrap();
        let cell = parse(&out)[0][3].clone();
        let expected = if guard {
            format!("'{name}")
        } else {
            name.clone()
        };
        assert_eq!(cell, expected, "{lead:?}");
        let mut out = String::new();
        let exact = CsvOptions::new().with_exact(true);
        record.write_csv(&mut out, &exact).unwrap();
        assert_eq!(parse(&out)[0][3], name, "{lead:?} exact");
    }
    let json: serde_json::Value = serde_json::from_str(&inventory.to_json()).unwrap();
    assert_eq!(json["points"][formula - 1]["name"], "+SUM(A1)");
}

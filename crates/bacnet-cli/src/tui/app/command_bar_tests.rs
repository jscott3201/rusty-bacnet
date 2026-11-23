//! The command bar's parsing: targets, writes, watches and the rest.

use bacnet_ops::Scope;
use bacnet_ops::target::routed_station;
use bacnet_types::enums::{ObjectType, Segmentation};

use super::*;
use crate::tui::message::DeviceRow;

/// A line read with a device table holding device 100.
fn parse(text: &str) -> Result<Line, String> {
    let mut devices = DeviceTable::default();
    devices.upsert(device_row(100, None));
    super::parse(text, &devices)
}

/// A table row for `instance`, routed at `station` when given.
fn device_row(instance: u32, station: Option<(u16, &[u8])>) -> DeviceRow {
    DeviceRow {
        instance,
        at: match station {
            Some((network, mac)) => DeviceAddress::new(network, mac).unwrap(),
            None => format!("10.0.0.{}:47808", instance % 250).parse().unwrap(),
        },
        address: match station {
            Some((network, mac)) => routed_station(network, mac).unwrap(),
            None => format!("10.0.0.{}:47808", instance % 250),
        },
        via: station.map(|_| "10.0.1.1:47808".into()),
        network: station.map(|(network, _)| network),
        vendor_id: 260,
        max_apdu: 480,
        segmentation: Segmentation::NO_SEGMENTATION,
        last_seen: std::time::Instant::now(),
    }
}

/// A routed station names the device the table lists there, in any
/// case of hex; one not in the table, or on another network, is an
/// error that says so (#1914).
#[test]
fn a_routed_station_names_the_device_the_table_lists_there() {
    let mut devices = DeviceTable::default();
    devices.upsert(device_row(2001, Some((2, &[5]))));
    devices.upsert(device_row(2002, Some((2, &[0x7F, 0, 0, 1, 0xBA, 0xC0]))));
    devices.upsert(device_row(100, None));
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let watch = |device, network| {
        Ok(Line::Watch(Point {
            device,
            network: Some(network),
            object: ai1,
            property: PropertyIdentifier::PRESENT_VALUE,
        }))
    };
    assert_eq!(super::parse("watch 2:0x05 ai:1", &devices), watch(2001, 2));
    assert_eq!(
        super::parse("w 2:0x7f000001bac0 ai:1", &devices),
        watch(2002, 2)
    );
    let Ok(Line::Writem(specs)) = super::parse("writem 2:0x05 av:1 pv 4", &devices) else {
        panic!("expected writem");
    };
    assert_eq!(specs[0].device, 2001);
    // A MAC is matched octet for octet: 2:0x0005 is two octets, not
    // station 5.
    for missing in [
        "watch 2:0x06 ai:1",
        "watch 3:0x05 ai:1",
        "writem 2:0x06 av:1 pv 4",
    ] {
        let error = super::parse(missing, &devices).unwrap_err();
        assert!(error.contains("no device in the device table"), "{error}");
    }
    // Two octets are no station 5: the refusal is the address rules' own,
    // not a hint to run a Who-Is.
    let error = super::parse("watch 2:0x0005 ai:1", &devices).unwrap_err();
    assert!(error.contains("not a station"), "{error}");
    // One row per address: another instance announced at the station
    // takes its row, and the station names that one now (#2560).
    devices.upsert(device_row(2003, Some((2, &[5]))));
    assert_eq!(super::parse("watch 2:0x05 ai:1", &devices), watch(2003, 2));
}

/// `writem` with a contested instance names its claimants, so the operator
/// can name one by its address (#2560); `dnet:N` counts only the claims on
/// that network, so one elsewhere leaves it one device.
#[test]
fn writem_names_the_claimants_of_a_contested_instance() {
    let mut devices = DeviceTable::default();
    devices.upsert(device_row(7, None));
    devices.upsert(device_row(7, Some((2, &[5]))));
    let error = super::parse("writem 7 av:1 pv 4", &devices).unwrap_err();
    assert_eq!(
        error,
        "'7': 2 devices in the device table claim that instance (10.0.0.7:47808, 2:0x05); \
         name one by its address"
    );
    let written_at = |line: &str, devices: &DeviceTable| match super::parse(line, devices) {
        Ok(Line::Writem(specs)) => (specs[0].device, specs[0].address.to_string()),
        other => panic!("{line}: {other:?}"),
    };
    assert_eq!(
        written_at("writem 2:7 av:1 pv 4", &devices),
        (7, "2:0x05".into())
    );
    assert_eq!(
        written_at("writem 10.0.0.7:47808 av:1 pv 4", &devices),
        (7, "10.0.0.7:47808".into())
    );
    devices.upsert(device_row(7, Some((2, &[6]))));
    let error = super::parse("writem 2:7 av:1 pv 4", &devices).unwrap_err();
    assert!(error.contains("(2:0x05, 2:0x06)"), "{error}");
    let error = super::parse("writem 3:7 av:1 pv 4", &devices).unwrap_err();
    assert!(error.contains("run a Who-Is first"), "{error}");
}

/// `find` takes the rest of the line as typed, apostrophes and runs of
/// spaces included; only a name that is one quoted word is unwrapped,
/// and `find ""` closes the list like a bare `find`.
#[test]
fn find_takes_the_rest_of_the_line_as_typed() {
    let find = |line: &str| match parse(line) {
        Ok(Line::Find(name, Scope::Local)) => name,
        other => panic!("{line}: {other:?}"),
    };
    assert_eq!(find("find Joe's office"), "Joe's office");
    assert_eq!(find("find  Supply  Air Temp "), "Supply  Air Temp");
    assert_eq!(find(r#"find "Zone  Temp""#), "Zone  Temp");
    assert_eq!(find(r#"find 'Zone "A"'"#), r#"Zone "A""#);
    assert_eq!(find(r#"find "a\"b""#), r#"a"b"#);
    assert_eq!(find(r#"find """#), "");
    assert_eq!(find("find"), "");
    // More than one quoted word, or an unbalanced one, stays as typed.
    assert_eq!(find(r#"find "a" "b""#), r#""a" "b""#);
    assert_eq!(find(r#"find "Zone 1"#), r#""Zone 1"#);
    assert_eq!(find(r"find C:\temp"), r"C:\temp");
    assert_eq!(find("\"find\" x  y"), "x  y");
    // A name that starts with two dashes is quoted.
    assert_eq!(find(r#"find "--global""#), "--global");
}

/// `find` sends its Who-Has to this network alone; `--dnet N` or
/// `--global` first names one remote network or every network, as the
/// one-shot `find` takes them (#2596). One flag, then a name; network 0
/// and 65535 are refused naming the option to use, and so is another
/// flag.
#[test]
fn find_takes_a_scope_flag_before_the_name() {
    assert_eq!(
        parse("find Zone Temp"),
        Ok(Line::Find("Zone Temp".into(), Scope::Local))
    );
    assert_eq!(
        parse("find --global Zone Temp"),
        Ok(Line::Find("Zone Temp".into(), Scope::Global))
    );
    assert_eq!(
        parse(r#"find --dnet 5 "Zone  Temp""#),
        Ok(Line::Find("Zone  Temp".into(), Scope::Network(5)))
    );
    for (line, says) in [
        ("find --dnet 0 Zone", "leave --dnet out"),
        ("find --dnet 65535 Zone", "--global"),
        ("find --dnet x Zone", "1 to 65534"),
        ("find --global", "usage: find"),
        (r#"find --global """#, "usage: find"),
        ("find --global --dnet 5 Zone", "usage: find"),
        ("find --target 10.0.0.9 Zone", "usage: find"),
    ] {
        let refused = parse(line).unwrap_err();
        assert!(refused.contains(says), "{line}: {refused}");
    }
}

/// A quoted VALUE is a CharacterString, spaces and all, with any
/// priority after the quote; a quote left open is refused (#1800).
#[test]
fn writem_takes_quoted_values() {
    let values = |line: &str| -> Vec<(Option<String>, Option<u8>)> {
        match parse(line) {
            Ok(Line::Writem(specs)) => specs.into_iter().map(|s| (s.value, s.priority)).collect(),
            other => panic!("{line}: {other:?}"),
        }
    };
    let string = |text: &str| Some(format!("\"{text}\""));
    assert_eq!(
        values(r#"writem 100 av:1 description "two words""#),
        [(string("two words"), None)]
    );
    assert_eq!(
        values(r#"writem 100 av:1 description 'two  words' av:2 pv 5@9"#),
        [(string("two  words"), None), (Some("5".into()), Some(9))]
    );
    // The priority goes after the quote; an @ inside it is text.
    assert_eq!(
        values(r#"writem 100 av:1 description "a@b"@8"#),
        [(string("a@b"), Some(8))]
    );
    assert_eq!(
        values(r#"writem 100 av:1 description """#),
        [(string(""), None)]
    );
    assert_eq!(
        values(r#"writem 100 av:1 description "say \"hi\"""#),
        [(string(r#"say "hi""#), None)]
    );
    // A quoted part anywhere makes the word a string.
    assert_eq!(
        values(r#"writem 100 av:1 description zone' 'four"#),
        [(string("zone four"), None)]
    );
    // An escape makes the word a string too, escaped quotes and all,
    // with the priority after the last escape.
    assert_eq!(
        values(r#"writem 100 av:1 description \"x\""#),
        [(string(r#""x""#), None)]
    );
    assert_eq!(
        values(r"writem 100 av:1 description Zone\ 2@9 av:2 pv 4"),
        [(string("Zone 2"), Some(9)), (Some("4".into()), None)]
    );
    // A quoted number is a string, which a numeric property refuses
    // when the write is planned.
    assert_eq!(
        values("writem 100 av:1 pv '72.5'"),
        [(string("72.5"), None)]
    );
    assert!(parse(r"writem 100 av:1 description Zone\ 2@17").is_err());
    // A backslash that escapes nothing is plain text, not a string.
    assert!(
        parse(r"writem 100 av:1 description C:\temp")
            .unwrap_err()
            .contains("cannot parse value")
    );
    // Each value reads back as the string typed.
    let Some(text) = string("two words") else {
        unreachable!()
    };
    assert_eq!(
        bacnet_ops::parse::parse_value(&text),
        Ok(bacnet_types::primitives::PropertyValue::CharacterString(
            "two words".into()
        ))
    );
    assert!(
        parse(r#"writem 100 av:1 description "x"@17"#)
            .unwrap_err()
            .contains("priority")
    );
    assert!(parse(r#"writem 100 av:1 description "x"@"#).is_err());
    assert!(
        parse(r#"writem 100 av:1 description "x"@0"#)
            .unwrap_err()
            .contains("priority")
    );
    let open = parse(r#"writem 100 av:1 description "two words"#).unwrap_err();
    assert!(open.contains("unclosed \" quote"), "{open}");
    // Unquoted, a space still splits the value from the next group.
    assert!(parse("writem 100 av:1 description two words").is_err());
}

#[test]
fn watch_lines_name_a_device_an_object_and_maybe_a_property() {
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(
        parse("watch 100 ai:1"),
        Ok(Line::Watch(Point {
            device: 100,
            network: None,
            object: ai1,
            property: PropertyIdentifier::PRESENT_VALUE
        }))
    );
    assert_eq!(
        parse("w 2:100 analog-input:1 sf"),
        Ok(Line::Watch(Point {
            device: 100,
            network: Some(2),
            object: ai1,
            property: PropertyIdentifier::STATUS_FLAGS
        }))
    );
    // The coming BACnet RDF encoding's `type,instance` names the same
    // object.
    for object in ["analog-input,1", "ai,1", "0,1"] {
        assert_eq!(
            parse(&format!("watch 100 {object}")),
            parse("watch 100 ai:1"),
            "{object}"
        );
    }
    assert_eq!(parse("csv on"), Ok(Line::Csv(true)));
    assert_eq!(
        parse("bbmd 10.0.0.1"),
        Ok(Line::Bbmd(Some("10.0.0.1:47808".parse().unwrap())))
    );
    assert!(parse("bbmd 10.0.0").is_err());
    assert_eq!(parse("bbmd"), Ok(Line::Bbmd(None)));
    assert!(
        parse("watch 10.0.0.5 ai:1")
            .unwrap_err()
            .contains("a device from the device table")
    );
    assert!(parse("watch 100 ai:1 object-list[2]").is_err());
    assert!(parse("watch 100 ai:x").is_err());
    assert_eq!(parse("find"), Ok(Line::Find(String::new(), Scope::Local)));
    let ms = Duration::from_millis;
    assert_eq!(
        parse("mstp rotation 1500"),
        Ok(Line::Mstp(Setting::Rotation(ms(1500))))
    );
    assert_eq!(
        parse("mstp hold 250"),
        Ok(Line::Mstp(Setting::Hold(ms(250))))
    );
    assert_eq!(
        parse("mstp listen 30"),
        Ok(Line::Mstp(Setting::Listen(Duration::from_secs(30))))
    );
    assert!(parse("mstp hold 0").is_err());
    assert!(parse("mstp listen 4000").is_err());
    assert!(parse("mstp poll 5").is_err());
    assert!(parse("finder x").unwrap_err().contains("unknown command"));
    assert!(
        parse("read 100 ai:1")
            .unwrap_err()
            .contains("unknown command")
    );
}

#[test]
fn mstp_scan_commands_require_an_explicit_bounded_window() {
    for text in ["mstp scan 1", "mstp scan 3600", "mstp scan cancel"] {
        assert!(parse(text).is_ok(), "{text}: {:?}", parse(text));
    }
    for text in [
        "mstp scan",
        "mstp scan 0",
        "mstp scan 3601",
        "mstp scan -1",
        "mstp scan 1.5",
        "mstp scan cancel now",
    ] {
        assert!(parse(text).is_err(), "{text}");
    }
}

#[test]
fn writem_lines_take_object_property_value_groups() {
    let Ok(Line::Writem(specs)) = parse("writem 100 ao:1 pv 42@8 av:1 description \"x\"") else {
        panic!("expected writem");
    };
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].value.as_deref(), Some("42"));
    assert_eq!(specs[0].priority, Some(8));
    assert_eq!(specs[1].property, PropertyIdentifier::DESCRIPTION);
    assert_eq!(specs[1].priority, None);
    assert!(parse("writem 100 ao:1 pv").is_err());
    assert!(parse("writem 100 ao:1 pv 42@17").is_err());
    assert!(parse("writem 100").is_err());
    // An OBJECT, or an `object:` value, in the coming BACnet RDF
    // encoding's `type,instance` form.
    assert_eq!(
        parse("writem 100 analog-output,1 pv 42@8"),
        parse("writem 100 ao:1 pv 42@8")
    );
    let Ok(Line::Writem(specs)) = parse("writem 100 nc,1 pv object:ai,1") else {
        panic!("expected writem");
    };
    assert_eq!(
        specs[0].object,
        ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap()
    );
    assert_eq!(specs[0].value.as_deref(), Some("object:ai,1"));
}

/// The table row a parsed line came from. A new `Line` fails to
/// compile here until it names its row.
fn row_of(line: &Line) -> &'static str {
    match line {
        Line::Watch(_) => "watch",
        Line::Csv(_) => "csv",
        Line::Writem(_) => "writem",
        Line::Bbmd(_) => "bbmd",
        Line::Find(..) => "find",
        Line::Mstp(_) | Line::Scan(_) => "mstp",
        Line::Manage(Action::Dcc { .. }) => "dcc",
        Line::Manage(Action::Reinit { .. }) => "reinit",
        Line::PathLimit(_) | Line::SelectedPathLimit(_) => "path-limit",
    }
}

/// Every parsed command has a row, so a hint and a help line; every
/// row parses; and the hint for a line is its own row's.
#[test]
fn every_command_has_its_hint_from_the_table_the_parser_reads() {
    let samples = [
        "watch 100 ai:1",
        "w 100 ai:1 pv",
        "csv on",
        "writem 100 ao:1 pv 42@8",
        "bbmd",
        "bbmd 10.0.0.1",
        "find Supply Temp",
        "find --global Supply Temp",
        "mstp hold 250",
        "mstp listen 30",
        "dcc disable-initiation 30",
        "control enable",
        "reinit warmstart",
        "path-limit 10.0.0.254:47808/5=128",
        "path-limit mstp:3/7 clear",
        "path-limit 100 480",
        "path-limit . clear",
    ];
    let mut rows = std::collections::BTreeSet::new();
    for sample in samples {
        let line = parse(sample).unwrap_or_else(|e| panic!("{sample}: {e}"));
        let row = row_of(&line);
        assert!(command(row).is_some(), "{row} has a row");
        let hint = matching(sample);
        assert_eq!(hint.len(), 1, "{sample}");
        assert_eq!(hint[0].name, row, "{sample}");
        rows.insert(row);
    }
    let names: std::collections::BTreeSet<&str> = COMMANDS.iter().map(|s| s.name).collect();
    assert_eq!(rows, names, "every row is parsed by a sample");
    for syntax in COMMANDS {
        for form in syntax.usage {
            assert_eq!(form.split_whitespace().next(), Some(syntax.name), "{form}");
        }
        assert!(usage().contains(syntax.usage[0]));
    }
    // A wrong form names its command's usage.
    assert_eq!(parse("csv maybe"), Err("usage: csv on|off".into()));
}

#[test]
fn hints_follow_what_is_typed() {
    let names = |text: &str| -> Vec<&str> { matching(text).iter().map(|s| s.name).collect() };
    assert_eq!(names("").len(), COMMANDS.len());
    assert_eq!(names("w"), ["watch", "writem"]);
    assert_eq!(names("w "), ["watch"]);
    assert_eq!(names("wr"), ["writem"]);
    assert_eq!(names("mstp hold"), ["mstp"]);
    assert!(names("zz").is_empty());
    assert!(names("zz 1").is_empty());
}

/// Each `find` form says where its Who-Has goes, and a passive session,
/// which sends none, says the scoped forms are not for it (#2596).
#[test]
fn each_find_form_says_where_it_asks() {
    let find = command("find").unwrap();
    let said = |form: &str, passive| find.description(form, passive);
    assert_eq!(said("find NAME", false), find.what);
    assert!(said("find --dnet N NAME", false).contains("remote network N"));
    assert!(said("find --global NAME", false).contains("every network"));
    assert!(said("find NAME", true).contains("observed"));
    for form in ["find --dnet N NAME", "find --global NAME"] {
        assert!(!said(form, false).contains("alone"), "{form}");
        assert!(said(form, true).contains("passive"), "{form}");
    }
}

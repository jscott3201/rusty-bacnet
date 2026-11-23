//! Indexed reads, array sizes, unmodelled algorithms and the pinned element
//! kinds (the review rounds of #2078 and #2079).

use super::*;

#[test]
fn indexed_reads_are_one_element_and_whole_lists_show_every_element() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::ANALOG_INPUT, &texts, Floats::Exact);
    let stamp = [0x19, 0x07];
    assert_eq!(format.value(P::EVENT_TIME_STAMPS, &stamp), "[seq 7]");
    assert_eq!(
        format.value_at(P::EVENT_TIME_STAMPS, Some(2), &stamp),
        "seq 7"
    );
    assert_eq!(
        format.data_at(P::EVENT_TIME_STAMPS, Some(2), &stamp),
        Datum::Constructed {
            element: "time-stamp".into(),
            text: "seq 7".into(),
            fields: None,
        }
    );
    // Index 0 is the array's size, an Unsigned.
    assert_eq!(
        format.value_at(P::EVENT_TIME_STAMPS, Some(0), &[0x21, 0x03]),
        "3"
    );
    let many: Vec<u8> = (0..10u8).flat_map(|n| [0x21, n]).collect();
    assert!(format.value(P::DESCRIPTION, &many).ends_with("(10 items)"));
    assert_eq!(
        format.whole_lists().value(P::DESCRIPTION, &many),
        "[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]"
    );
}

/// An indexed Weekly_Schedule read names the day it holds, in the text and
/// the typed data (the review of #2078).
#[test]
fn indexed_schedule_days_are_named_for_their_place_in_the_week() {
    use bacnet_encoding::constructed::encode_daily_schedule;
    use bacnet_types::constructed::BACnetTimeValue;
    let day = bytes(|buf| {
        encode_daily_schedule(
            buf,
            &[BACnetTimeValue {
                time: hm(7, 0),
                value: PropertyValue::Real(21.5),
            }],
        )
        .unwrap()
    });
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::SCHEDULE, &texts, Floats::Exact);
    for (index, name) in [(1, "Mon"), (3, "Wed"), (7, "Sun")] {
        let shown = format!("{name} 07:00 = 21.5");
        assert_eq!(
            format.value_at(P::WEEKLY_SCHEDULE, Some(index), &day),
            shown
        );
        assert_eq!(
            format.data_at(P::WEEKLY_SCHEDULE, Some(index), &day),
            Datum::Constructed {
                element: "daily-schedule".into(),
                text: shown,
                fields: None,
            }
        );
    }
}

/// Index 0 of an array is its size: a count, never named as a state.
#[test]
fn an_array_size_is_a_plain_count() {
    let texts = StateTexts {
        states: vec!["Low".into(), "Medium".into(), "High".into()],
        ..StateTexts::default()
    };
    let format = ValueFormat::new(O::MULTI_STATE_OUTPUT, &texts, Floats::Exact);
    // A size of 2 would read as state 2, Medium, if taken for a state.
    let size = [0x21, 2];
    assert_eq!(format.value_at(P::PRIORITY_ARRAY, Some(0), &size), "2");
    assert_eq!(
        format.data_at(P::PRIORITY_ARRAY, Some(0), &size),
        Datum::Unsigned(2)
    );
    // An element of the array is still a state.
    assert_eq!(
        format.value_at(P::PRIORITY_ARRAY, Some(8), &[0x21, 2]),
        "2 (Medium)"
    );
}

/// An Event_Parameters alternative under a tag the standard doesn't assign
/// shows its fields by context tag instead of hex, and an empty MAC reads as
/// a broadcast.
#[test]
fn unassigned_algorithms_show_their_fields() {
    // Tag 30: [0] 30, [1] 10, [2] 300.
    let parameter = BACnetEventParameter::Opaque {
        tag: 30,
        data: vec![0x09, 30, 0x19, 10, 0x2A, 0x01, 0x2C],
    };
    let octets = bytes(|buf| encode_event_parameter(buf, &parameter).unwrap());
    assert_eq!(
        text(O::EVENT_ENROLLMENT, P::EVENT_PARAMETERS, &octets),
        "30: [0] [raw 1e], [1] [raw 0a], [2] [raw 01 2c]"
    );
    let broadcast = bytes(|buf| {
        encode_recipient(
            buf,
            &BACnetRecipient::Address(BACnetAddress {
                network_number: 5,
                mac_address: MacAddr::new(),
            }),
        )
        .unwrap()
    });
    assert_eq!(
        text(O::DEVICE, P::AUDIT_NOTIFICATION_RECIPIENT, &broadcast),
        "network 5 broadcast"
    );
}

/// The JSON names of the element kinds are pinned: renaming an encoding
/// variant must not change them.
#[test]
fn element_kinds_have_fixed_names() {
    let names: Vec<&str> = ConstructedElement::ALL
        .iter()
        .map(|element| constructed::element_name(*element))
        .collect();
    assert_eq!(
        names,
        [
            "name-value",
            "destination",
            "port-permission",
            "read-access-specification",
            "read-access-result",
            "action-list",
            "device-object-reference",
            "authentication-factor-format",
            "authentication-policy",
            "stage-limit-value",
            "access-rule",
            "device-object-property-reference",
            "object-property-reference",
            "property-access-result",
            "recipient",
            "daily-schedule",
            "special-event",
            "calendar-entry",
            "date-range",
            "time-stamp",
            "cov-subscription",
            "value-source",
            "scale",
            "prescale",
            "address-binding",
            "shed-level",
            "authentication-factor",
            "event-parameter",
            "bdt-entry",
            "fdt-entry",
            "host-n-port",
        ]
    );
}

/// #2710: FD_BBMD_Address and BACnet_IP_Global_Address show the host and
/// port as a BDT row shows its BBMD; a value that isn't one host and port
/// keeps the generic view.
#[test]
fn network_port_host_n_ports_show_as_addresses() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::NETWORK_PORT, &texts, Floats::Exact);
    for property in [P::FD_BBMD_ADDRESS, P::BACNET_IP_GLOBAL_ADDRESS] {
        let show = |octets: &str| format.value(property, &hex_octets(octets));
        assert_eq!(show("0e1cc00002020f1abac0"), "192.0.2.2:47808");
        assert_eq!(show("0e2c006875620f197b"), "hub:123");
        assert_eq!(show("0e080f1900"), "none");
        assert_eq!(
            show("0e1d1020010db80000000000000000000000020f1abac0"),
            "[2001:db8::2]:47808"
        );
        assert_eq!(
            format.data(property, &hex_octets("0e1cc00002020f1abac0")),
            Datum::Constructed {
                element: "host-n-port".into(),
                text: "192.0.2.2:47808".into(),
                fields: None,
            }
        );
        // A host and port with octets after it isn't one value.
        assert!(!show("0e1cc00002020f1abac021").contains("192.0.2.2:47808"));
    }
}

/// #2690: a Network Port's BDT and FDT show one entry per row, as
/// `read-bdt` and `read-fdt` print their addresses; an empty table and a
/// value that isn't the entries keep the generic view.
#[test]
fn network_port_tables_show_their_entries() {
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::NETWORK_PORT, &texts, Floats::Exact);
    let bdt = |octets: &str| {
        let octets = hex_octets(octets);
        format.value(P::BBMD_BROADCAST_DISTRIBUTION_TABLE, &octets)
    };
    // 192.0.2.2:47808 with a mask; a name without one; no host; IPv6.
    assert_eq!(
        bdt("0e0e1cc00002020f1abac00f1cffffff00"),
        "[192.0.2.2:47808 mask 255.255.255.0]"
    );
    assert_eq!(
        bdt("0e0e2c006875620f197b0f0e0e080f19000f"),
        "[hub:123, none]"
    );
    assert_eq!(
        bdt("0e0e1d1020010db80000000000000000000000020f1abac00f"),
        "[[2001:db8::2]:47808]"
    );
    let fdt = hex_octets("0d06c0000203bac0193c2932");
    assert_eq!(
        format.value(P::BBMD_FOREIGN_DEVICE_TABLE, &fdt),
        "[192.0.2.3:47808 ttl 60s remaining 50s]"
    );
    assert_eq!(
        format.data(P::BBMD_FOREIGN_DEVICE_TABLE, &fdt),
        Datum::List(vec![Datum::Constructed {
            element: "fdt-entry".into(),
            text: "192.0.2.3:47808 ttl 60s remaining 50s".into(),
            fields: None,
        }])
    );
    assert_eq!(format.value(P::BBMD_FOREIGN_DEVICE_TABLE, &[]), "(empty)");
    // A mask of three octets isn't an entry: the tags are shown instead.
    assert!(!bdt("0e0e1cc00002020f1abac00f1bffffff").contains("mask"));
    // The same octets on another object type aren't a table.
    let other = ValueFormat::new(O::ANALOG_INPUT, &texts, Floats::Exact);
    assert!(
        !other
            .value(P::BBMD_FOREIGN_DEVICE_TABLE, &fdt)
            .contains("ttl")
    );
}

fn hex_octets(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// #2098: a DateTime Value's Priority_Array is sixteen elements, a set slot
/// the date-time choice under context tag 1, shown as a date-time in the
/// text, the per-slot element and the typed data. The same octets on
/// another object type stay raw.
#[test]
fn a_date_time_priority_slot_reads_as_a_date_time() {
    use bacnet_encoding::primitives::encode_priority_date_time;
    use bacnet_types::primitives::Date;
    let date = Date {
        year: 126,
        month: 10,
        day: 25,
        day_of_week: Date::UNSPECIFIED,
    };
    let time = Time {
        hour: 7,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    let mut buf = BytesMut::new();
    for slot in 1..=16 {
        if slot == 8 {
            encode_priority_date_time(&mut buf, &date, &time);
        } else {
            buf.extend_from_slice(&[0x00]);
        }
    }
    let values = decode_all(&buf).unwrap();
    assert_eq!(values.len(), 16);

    let texts = StateTexts::default();
    let format = ValueFormat::new(O::DATETIME_VALUE, &texts, Floats::Exact).whole_lists();
    let shown = format.value(P::PRIORITY_ARRAY, &buf);
    assert!(shown.contains("null, 2026-10-25 07:30, null"), "{shown}");
    assert_eq!(
        format.element(P::PRIORITY_ARRAY, &values[7]),
        "2026-10-25 07:30"
    );
    // An indexed read of the slot.
    let slot = &buf[7..19];
    assert_eq!(
        format.value_at(P::PRIORITY_ARRAY, Some(8), slot),
        "2026-10-25 07:30"
    );
    let Datum::List(items) = data(O::DATETIME_VALUE, P::PRIORITY_ARRAY, &buf) else {
        panic!("a whole Priority_Array is a list");
    };
    assert_eq!(items.len(), 16);
    assert_eq!(
        items[7],
        Datum::List(vec![
            Datum::Date {
                date: "2026-10-25".into()
            },
            Datum::Time {
                time: "07:30".into()
            },
        ])
    );
    assert_eq!(items[0], Datum::Null);
    // Not a date-time value type: the slot is shown as its tags (#2618).
    assert!(matches!(
        data(O::ANALOG_VALUE, P::PRIORITY_ARRAY, slot),
        Datum::Constructed { element, text, .. }
            if element == "tagged" && text == "[1] (2026-10-25, 07:30)"
    ));
}

/// #2618: a value with no decoded form that holds a context tag (a vendor
/// property, a Lighting_Command) is shown as its tags, its application
/// values decoded; its data is a `tagged` element keeping the octets.
#[test]
fn a_value_with_context_tags_shows_its_tags() {
    // [0] 2, [1] ( REAL 1.5, "hi", [2] ( TRUE ) ), Unsigned 7.
    let octets = [
        0x09, 0x02, 0x1E, 0x44, 0x3F, 0xC0, 0, 0, 0x73, 0x00, b'h', b'i', 0x2E, 0x11, 0x2F, 0x1F,
        0x21, 7,
    ];
    let texts = StateTexts::default();
    let format = ValueFormat::new(O::from_raw(200), &texts, Floats::Exact);
    let vendor = P::from_raw(1000);
    let shown = "[0] [raw 02], [1] (1.5, \"hi\", [2] (true)), 7";
    assert_eq!(format.value(vendor, &octets), shown);
    let value = Value::new(&format, vendor, None, octets.to_vec(), None);
    assert_eq!(value.text, shown);
    assert_eq!(
        serde_json::to_value(&value.data).unwrap(),
        serde_json::json!({
            "element": "tagged",
            "text": shown,
            "fields": {"data": {"raw": "09021e443fc00000730068692e112f1f2107"}},
        })
    );
    // It may quote a device's string.
    assert!(value.may_hold_device_text());
    // A standard property without a decoded form reads the same way.
    let command = [0x09, 0x02];
    let lighting = ValueFormat::new(O::LIGHTING_OUTPUT, &texts, Floats::Exact);
    assert_eq!(
        lighting.value(P::LIGHTING_COMMAND, &command),
        "[0] [raw 02]"
    );
    // Octets whose framing breaks stay hex, whole; an application value
    // that doesn't decode beside a context tag, as its own octets.
    assert_eq!(
        format.value(vendor, &[0x09, 0x02, 0x2E, 0x21]),
        "[raw 09 02 2e 21]"
    );
    assert_eq!(
        format.value(vendor, &[0x09, 0x02, 0x43, 1, 2, 3, 0x2E, 0x2F]),
        "[0] [raw 02], [raw 43 01 02 03], [2] ()"
    );
}

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
        ]
    );
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
    // Not a date-time value type: the octets are shown as they came.
    assert!(matches!(
        data(O::ANALOG_VALUE, P::PRIORITY_ARRAY, slot),
        Datum::Raw { .. }
    ));
}

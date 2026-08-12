//! Intrinsic-reporting property wiring on Binary Input, Output, and Value
//! objects, including event-history coverage for #235.
//!
//! Distribution is exercised at wire level in the server tests. These tests
//! pin the object-level commissioning surface — before #229, Time_Delay
//! and Notify_Type had no arms at all and Event_Enable was readable but not
//! writable, so the transition bits were stuck at (F, F, F).

use super::*;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState};

fn assert_protocol_error(result: Result<(), Error>, code: ErrorCode) {
    match result.unwrap_err() {
        Error::Protocol {
            class,
            code: actual,
        } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(actual, code.to_raw() as u32);
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

fn assert_property_read_error(result: Result<PropertyValue, Error>, code: ErrorCode) {
    match result.unwrap_err() {
        Error::Protocol {
            class,
            code: actual,
        } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(actual, code.to_raw() as u32);
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

/// Assert a refused write came back as PROPERTY / WRITE_ACCESS_DENIED, the error
/// the generic write macro raises deliberately. Checking only `is_err` would also
/// accept UNKNOWN_PROPERTY, which is what a property falling through to the
/// catch-all arm returns — a silent loss of the denial, not a denial.
fn assert_write_access_denied(
    result: Result<(), Error>,
    property: PropertyIdentifier,
    label: &str,
) {
    match result.expect_err(&format!("{label}: {property:?} write must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{label}: {property:?} error class"
            );
            assert_eq!(
                code,
                ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
                "{label}: {property:?} error code"
            );
        }
        other => panic!("{label}: {property:?} expected WRITE_ACCESS_DENIED, got {other:?}"),
    }
}

/// Every property that commissions intrinsic reporting must survive a network
/// write and read back, and be advertised in both `property_list` and
/// `is_writable_property` so the PICS matches dispatch.
fn assert_event_properties_round_trip(object: &mut dyn BACnetObject, label: &str) {
    for (property, value) in [
        (
            PropertyIdentifier::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::all().to_bacnet()],
            },
        ),
        (
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyValue::Enumerated(1),
        ),
        (
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(42),
        ),
        (PropertyIdentifier::TIME_DELAY, PropertyValue::Unsigned(7)),
    ] {
        object
            .write_property(property, None, value.clone(), None)
            .unwrap_or_else(|e| panic!("{label}: {property:?} write rejected: {e:?}"));
        assert_eq!(
            object.read_property(property, None).unwrap(),
            value,
            "{label}: {property:?} must read back what was written"
        );
        assert!(
            object.property_list().contains(&property),
            "{label}: {property:?} must be advertised in Property_List"
        );
        assert!(
            object.is_writable_property(property),
            "{label}: {property:?} must be advertised writable"
        );
    }

    // A single set bit must land on the transition it names, not on whichever
    // bit a mask error happens to select. TO_NORMAL is wire bit 2 (0x20).
    object
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_NORMAL.to_bacnet()],
            },
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0x20],
        },
        "{label}: TO_NORMAL alone must read back as wire bit 2"
    );

    // Acked_Transitions is readable but never writable: the alarm-
    // acknowledgment process maintains it from event-state transitions and
    // acknowledgment indications, and an indication ORs the bit in where a
    // property write would assign — a write could fabricate and erase
    // acknowledgments alike.
    assert_write_access_denied(
        object.write_property(
            PropertyIdentifier::ACKED_TRANSITIONS,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_OFFNORMAL.to_bacnet()],
            },
            None,
        ),
        PropertyIdentifier::ACKED_TRANSITIONS,
        label,
    );
    assert!(!object.is_writable_property(PropertyIdentifier::ACKED_TRANSITIONS));
    assert!(object
        .property_list()
        .contains(&PropertyIdentifier::ACKED_TRANSITIONS));

    // Event_State is maintained by the detector, never assignable from the
    // network — a write arm would let a client fake an alarm state.
    assert_write_access_denied(
        object.write_property(
            PropertyIdentifier::EVENT_STATE,
            None,
            PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
            None,
        ),
        PropertyIdentifier::EVENT_STATE,
        label,
    );
    assert!(!object.is_writable_property(PropertyIdentifier::EVENT_STATE));
}

#[test]
fn bi_event_properties_round_trip_and_match_pics() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_event_properties_round_trip(&mut bi, "BI");
}

#[test]
fn bv_event_properties_round_trip_and_match_pics() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    assert_event_properties_round_trip(&mut bv, "BV");
}

#[test]
fn binary_alarm_value_round_trips_and_matches_pics() {
    for object in [
        &mut BinaryInputObject::new(1, "BI-1").unwrap() as &mut dyn BACnetObject,
        &mut BinaryValueObject::new(1, "BV-1").unwrap() as &mut dyn BACnetObject,
    ] {
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
        object
            .write_property(
                PropertyIdentifier::ALARM_VALUE,
                None,
                PropertyValue::Enumerated(0),
                None,
            )
            .unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert!(object
            .property_list()
            .contains(&PropertyIdentifier::ALARM_VALUE));
        assert!(object.is_writable_property(PropertyIdentifier::ALARM_VALUE));
        assert!(!object
            .property_list()
            .contains(&PropertyIdentifier::ALARM_VALUES));
        assert_protocol_error(
            object.write_property(
                PropertyIdentifier::ALARM_VALUE,
                None,
                PropertyValue::Unsigned(1),
                None,
            ),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0),
            "a rejected wrong-type write must preserve Alarm_Value"
        );
        assert_protocol_error(
            object.write_property(
                PropertyIdentifier::ALARM_VALUE,
                None,
                PropertyValue::Enumerated(2),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }

    assert_protocol_error(
        BinaryInputObject::new(2, "BI-2")
            .unwrap()
            .read_property(PropertyIdentifier::ALARM_VALUES, None)
            .map(|_| ()),
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

#[test]
fn fresh_binary_input_is_default_armed_for_active() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_present_value(1);
    let outcome = bi
        .evaluate_intrinsic_reporting()
        .expect("ACTIVE is the default Alarm_Value");
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
}

#[test]
fn binary_event_history_is_listed_readable_and_read_only() {
    let timestamps = PropertyValue::List(vec![
        PropertyValue::Unsigned(0),
        PropertyValue::Unsigned(0),
        PropertyValue::Unsigned(0),
    ]);
    let messages = PropertyValue::List(vec![
        PropertyValue::CharacterString(String::new()),
        PropertyValue::CharacterString(String::new()),
        PropertyValue::CharacterString(String::new()),
    ]);
    for (mut object, label) in [
        (
            Box::new(BinaryInputObject::new(1, "BI-1").unwrap()) as Box<dyn BACnetObject>,
            "BI",
        ),
        (
            Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()) as Box<dyn BACnetObject>,
            "BO",
        ),
        (
            Box::new(BinaryValueObject::new(1, "BV-1").unwrap()) as Box<dyn BACnetObject>,
            "BV",
        ),
    ] {
        for (property, expected) in [
            (PropertyIdentifier::EVENT_TIME_STAMPS, &timestamps),
            (PropertyIdentifier::EVENT_MESSAGE_TEXTS, &messages),
        ] {
            assert!(object.property_list().contains(&property), "{label}");
            assert_eq!(object.read_property(property, None).unwrap(), *expected);
            assert_eq!(
                object.read_property(property, Some(0)).unwrap(),
                PropertyValue::Unsigned(3)
            );
            let second = match property {
                p if p == PropertyIdentifier::EVENT_TIME_STAMPS => PropertyValue::Unsigned(0),
                _ => PropertyValue::CharacterString(String::new()),
            };
            assert_eq!(object.read_property(property, Some(2)).unwrap(), second);
            assert_property_read_error(
                object.read_property(property, Some(4)),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
            assert_write_access_denied(
                object.write_property(property, None, expected.clone(), None),
                property,
                label,
            );
            assert!(!object.is_writable_property(property));
        }
    }
}

macro_rules! assert_binary_history_reset {
    ($object:expr, $label:literal) => {{
        let mut object = $object;
        object.event_history.time_stamps[0] = BACnetTimeStamp::SequenceNumber(91);
        object.event_history.message_texts[1] = "seeded".into();
        object
            .write_property(
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                None,
                PropertyValue::Boolean(false),
                None,
            )
            .unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(0); 3]),
            "{} timestamp reset",
            $label
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::CharacterString(String::new()); 3]),
            "{} message reset",
            $label
        );
    }};
}

#[test]
fn binary_detection_disable_resets_each_event_history() {
    assert_binary_history_reset!(BinaryInputObject::new(1, "BI-1").unwrap(), "BI");
    assert_binary_history_reset!(BinaryOutputObject::new(1, "BO-1").unwrap(), "BO");
    assert_binary_history_reset!(BinaryValueObject::new(1, "BV-1").unwrap(), "BV");
}

// ──────────────────────────────────────────────────────────────────────────
// #255 — Notify_Type production validation and Event_Enable bit-string
// width validation on the shared write macro.
// ──────────────────────────────────────────────────────────────────────────

/// BACnetNotifyType is a closed {alarm(0), event(1), ack-notification(2)}
/// production (Clause 21). An out-of-production write is PROPERTY /
/// VALUE_OUT_OF_RANGE (Clause 15.9.1.3) and leaves the stored value untouched.
#[test]
fn bi_notify_type_rejects_out_of_production_values() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_eq!(
        bi.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );

    for out_of_production in [3u32, 99, u32::MAX] {
        assert_protocol_error(
            bi.write_property(
                PropertyIdentifier::NOTIFY_TYPE,
                None,
                PropertyValue::Enumerated(out_of_production),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            bi.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(0),
            "a refused Notify_Type write must leave the value untouched ({out_of_production})"
        );
    }
    for in_production in [0u32, 1, 2] {
        bi.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(in_production),
            None,
        )
        .expect("named Notify_Type values must be accepted");
    }
}

/// BACnetEventTransitionBits is a 3-bit production (Clause 21); its canonical
/// encoding is one content octet with 5 unused bits. A write declaring any
/// other shape is PROPERTY / INVALID_DATA_ENCODING (Clause 15.9.1.3), not a
/// value to mask and normalize.
#[test]
fn bi_event_enable_rejects_noncanonical_bit_strings() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let canonical = bi
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();

    for (unused_bits, data) in [
        (0u8, vec![0xFFu8]),     // 8-bit string where the production defines 3
        (5u8, vec![0xFF, 0xFF]), // two content octets
        (4u8, vec![0xF0u8]),     // half-octet string
        (5u8, vec![]),           // no content octet
    ] {
        assert_protocol_error(
            bi.write_property(
                PropertyIdentifier::EVENT_ENABLE,
                None,
                PropertyValue::BitString { unused_bits, data },
                None,
            ),
            ErrorCode::INVALID_DATA_ENCODING,
        );
        assert_eq!(
            bi.read_property(PropertyIdentifier::EVENT_ENABLE, None)
                .unwrap(),
            canonical,
            "a refused Event_Enable write must leave the value untouched"
        );
    }

    // The canonical full-width shape stays accepted.
    bi.write_property(
        PropertyIdentifier::EVENT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0], // to-offnormal + to-normal, MSB-first
        },
        None,
    )
    .unwrap();
    assert_eq!(
        bi.read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0],
        }
    );
}

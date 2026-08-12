//! Intrinsic-reporting property wiring on Multi-state Input, Output, and Value
//! objects, including event-history coverage for #235.
//!
//! New multi-state tests live here rather than inline in the object files, which
//! is where this module tree keeps them — `multistate/tests/` is declared by
//! `multistate/mod.rs` and already holds `objects.rs` and `state_text.rs`.

use super::super::*;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState};
use bacnet_types::error::Error;

/// Commission `Event_Enable` the way a client does: a Clause 20.2.10 bit string
/// through `write_property`, never a direct field assignment.
fn write_event_enable(object: &mut dyn BACnetObject, bits: EventTransitionBits) {
    object
        .write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![bits.to_bacnet()],
            },
            None,
        )
        .unwrap();
}

/// `Event_Enable` names one bit per transition, so a fixture that sets all three
/// cannot tell a correct bit from a wrong one — an inverted or off-by-one mask
/// passes it. This drives Multi-state Input into OFFNORMAL with a single bit set
/// at a time: TO_OFFNORMAL distributes, and TO_FAULT — a bit that exists but
/// names a different transition — does not.
///
/// Multi-state Input remains the compact object-level vehicle; these detector
/// semantics are independent of the network decoder (#182).
#[test]
fn msi_event_enable_bit_selects_which_transition_distributes() {
    for (bits, distributes) in [
        (EventTransitionBits::TO_OFFNORMAL, true),
        (EventTransitionBits::TO_FAULT, false),
        (EventTransitionBits::TO_NORMAL, false),
        (EventTransitionBits::empty(), false),
        (EventTransitionBits::all(), true),
    ] {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        msi.set_alarm_values(vec![2]);
        write_event_enable(&mut msi, bits);
        msi.set_present_value(2);

        let outcome = msi
            .evaluate_intrinsic_reporting()
            .expect("an alarm value in Present_Value must report a transition");
        assert_eq!(
            outcome.change.to,
            EventState::OFFNORMAL,
            "Event_Enable {bits} must not change which transition occurs"
        );
        assert_eq!(
            outcome.distribute, distributes,
            "Event_Enable {bits}: TO_OFFNORMAL distribution"
        );
    }
}

/// The return-to-normal half of the same gate, which every case above misses
/// because they all target OFFNORMAL. A gate keyed to a fixed bit rather than to
/// the transition being reported — `event_enable & 0x01` where
/// `& transition_bit` belongs — answers one of these two rows wrongly.
#[test]
fn msi_event_enable_bit_selects_whether_the_return_to_normal_distributes() {
    for (bits, distributes) in [
        (EventTransitionBits::TO_NORMAL, true),
        (EventTransitionBits::TO_OFFNORMAL, false),
    ] {
        let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
        msi.set_alarm_values(vec![2]);
        write_event_enable(&mut msi, bits);

        msi.set_present_value(2);
        let entry = msi
            .evaluate_intrinsic_reporting()
            .expect("an alarm value must report the entry into OFFNORMAL");
        assert_eq!(entry.change.to, EventState::OFFNORMAL);

        msi.set_present_value(1);
        let returned = msi
            .evaluate_intrinsic_reporting()
            .expect("leaving the alarm value must report the return to NORMAL");
        assert_eq!(returned.change.from, EventState::OFFNORMAL);
        assert_eq!(returned.change.to, EventState::NORMAL);
        assert_eq!(
            returned.distribute, distributes,
            "Event_Enable {bits}: TO_NORMAL distribution"
        );
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
        (
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyValue::Unsigned(9),
        ),
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

    // Time_Delay_Normal mirrors Time_Delay's write validation: Unsigned within
    // the u32 span or refused with the Clause 15.9.1.3 pairings, and a refused
    // write leaves the stored value untouched.
    assert_property_error(
        object.write_property(
            PropertyIdentifier::TIME_DELAY_NORMAL,
            None,
            PropertyValue::Enumerated(9),
            None,
        ),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_property_error(
        object.write_property(
            PropertyIdentifier::TIME_DELAY_NORMAL,
            None,
            PropertyValue::Unsigned(u32::MAX as u64 + 1),
            None,
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(9),
        "{label}: refused Time_Delay_Normal writes must leave the value untouched"
    );

    // A single set bit must land on the transition it names, not on whichever
    // bit a mask error happens to select. The literal is the Clause 20.2.10 wire
    // encoding: TO_NORMAL is wire bit 2, so an LSB-first regression reads 0x04.
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
fn msi_event_properties_round_trip_and_match_pics() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    assert_event_properties_round_trip(&mut msi, "MSI");
}

#[test]
fn msv_event_properties_round_trip_and_match_pics() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    assert_event_properties_round_trip(&mut msv, "MSV");
}

#[test]
fn mso_event_properties_round_trip_and_match_pics() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    assert_event_properties_round_trip(&mut mso, "MSO");
}

/// Clause 13.3: "If no value is available for this parameter, then it takes on
/// the value of the pTimeDelay parameter." An object that was never written a
/// Time_Delay_Normal reads back the effective (fallback) delay.
#[test]
fn multistate_time_delay_normal_defaults_to_time_delay_when_unwritten() {
    for (mut object, label) in [
        (
            Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()) as Box<dyn BACnetObject>,
            "MSI",
        ),
        (
            Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()) as Box<dyn BACnetObject>,
            "MSO",
        ),
        (
            Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()) as Box<dyn BACnetObject>,
            "MSV",
        ),
    ] {
        assert_eq!(
            object
                .read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
                .unwrap(),
            object
                .read_property(PropertyIdentifier::TIME_DELAY, None)
                .unwrap(),
            "{label}: unwritten Time_Delay_Normal reads back Time_Delay"
        );
        object
            .write_property(
                PropertyIdentifier::TIME_DELAY,
                None,
                PropertyValue::Unsigned(11),
                None,
            )
            .unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
                .unwrap(),
            PropertyValue::Unsigned(11),
            "{label}: the fallback tracks Time_Delay"
        );
    }
}

#[test]
fn multistate_event_history_is_listed_readable_and_read_only() {
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
            Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()) as Box<dyn BACnetObject>,
            "MSI",
        ),
        (
            Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()) as Box<dyn BACnetObject>,
            "MSO",
        ),
        (
            Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()) as Box<dyn BACnetObject>,
            "MSV",
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

macro_rules! assert_multistate_history_reset {
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
fn multistate_detection_disable_resets_each_event_history() {
    assert_multistate_history_reset!(MultiStateInputObject::new(1, "MSI-1", 3).unwrap(), "MSI");
    assert_multistate_history_reset!(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap(), "MSO");
    assert_multistate_history_reset!(MultiStateValueObject::new(1, "MSV-1", 3).unwrap(), "MSV");
}

fn assert_property_error(result: Result<(), Error>, code: ErrorCode) {
    // Thin adapter keeps write assertions on the shared protocol-error helper.
    assert_property_read_error(result.map(|_| PropertyValue::Null), code);
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

#[test]
fn multistate_alarm_values_round_trip_and_match_pics() {
    for object in [
        &mut MultiStateInputObject::new(1, "MSI-1", 3).unwrap() as &mut dyn BACnetObject,
        &mut MultiStateValueObject::new(1, "MSV-1", 3).unwrap() as &mut dyn BACnetObject,
    ] {
        let value = PropertyValue::List(vec![
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(99),
        ]);
        object
            .write_property(PropertyIdentifier::ALARM_VALUES, None, value.clone(), None)
            .unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            value
        );
        assert!(object
            .property_list()
            .contains(&PropertyIdentifier::ALARM_VALUES));
        assert!(object.is_writable_property(PropertyIdentifier::ALARM_VALUES));
        assert_property_error(
            object.write_property(
                PropertyIdentifier::ALARM_VALUES,
                None,
                PropertyValue::Unsigned(2),
                None,
            ),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_property_error(
            object.write_property(
                PropertyIdentifier::ALARM_VALUES,
                None,
                PropertyValue::List(vec![PropertyValue::Enumerated(2)]),
                None,
            ),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            value,
            "a rejected wrong-element write must preserve the prior list"
        );
        assert_property_error(
            object.write_property(
                PropertyIdentifier::ALARM_VALUES,
                None,
                PropertyValue::List(vec![PropertyValue::Unsigned(u32::MAX as u64 + 1)]),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            value,
            "an overflowing element must leave the prior list intact"
        );
        let boundary = PropertyValue::List(
            (0..MAX_ALARM_VALUES)
                .map(|value| PropertyValue::Unsigned(value as u64))
                .collect(),
        );
        object
            .write_property(
                PropertyIdentifier::ALARM_VALUES,
                None,
                boundary.clone(),
                None,
            )
            .unwrap();
        let overlong = PropertyValue::List(
            (0..=MAX_ALARM_VALUES)
                .map(|value| PropertyValue::Unsigned(value as u64))
                .collect(),
        );
        match object
            .write_property(PropertyIdentifier::ALARM_VALUES, None, overlong, None)
            .unwrap_err()
        {
            Error::Protocol { class, code } => {
                assert_eq!(class, ErrorClass::RESOURCES.to_raw() as u32);
                assert_eq!(code, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY.to_raw() as u32);
            }
            other => panic!("expected resource-cap error, got {other:?}"),
        }
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            boundary,
            "an overlong list must leave the accepted boundary list intact"
        );
        assert_property_error(
            object.write_property(
                PropertyIdentifier::ALARM_VALUES,
                Some(1),
                PropertyValue::List(vec![PropertyValue::Unsigned(2)]),
                None,
            ),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::ALARM_VALUES, None)
                .unwrap(),
            boundary,
            "an indexed LIST write must leave the prior list intact"
        );
        object
            .write_property(
                PropertyIdentifier::ALARM_VALUES,
                None,
                PropertyValue::List(vec![]),
                None,
            )
            .unwrap();
    }
}

#[test]
fn unsupported_fault_and_output_alarm_surfaces_are_unknown() {
    let objects: [&dyn BACnetObject; 3] = [
        &MultiStateInputObject::new(1, "MSI-1", 3).unwrap(),
        &MultiStateValueObject::new(1, "MSV-1", 3).unwrap(),
        &MultiStateOutputObject::new(1, "MSO-1", 3).unwrap(),
    ];
    for object in objects {
        assert_property_read_error(
            object.read_property(PropertyIdentifier::FAULT_VALUES, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert!(!object
            .property_list()
            .contains(&PropertyIdentifier::FAULT_VALUES));
        assert!(!object.is_writable_property(PropertyIdentifier::FAULT_VALUES));
    }
    let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    assert_property_read_error(
        mso.read_property(PropertyIdentifier::ALARM_VALUES, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert!(!mso
        .property_list()
        .contains(&PropertyIdentifier::ALARM_VALUES));
    assert!(!mso
        .property_list()
        .contains(&PropertyIdentifier::FAULT_VALUES));
}

#[test]
fn recommissioning_alarm_values_while_offnormal_returns_to_normal() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    msi.set_alarm_values(vec![2]);
    msi.set_present_value(2);
    assert_eq!(
        msi.evaluate_intrinsic_reporting().unwrap().change.to,
        EventState::OFFNORMAL
    );
    msi.write_property(
        PropertyIdentifier::ALARM_VALUES,
        None,
        PropertyValue::List(vec![PropertyValue::Unsigned(3)]),
        None,
    )
    .unwrap();
    let returned = msi.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(returned.change.from, EventState::OFFNORMAL);
    assert_eq!(returned.change.to, EventState::NORMAL);
}

// ──────────────────────────────────────────────────────────────────────────
// #255 — Notify_Type production validation and Event_Enable bit-string
// width validation on the shared write macro.
// ──────────────────────────────────────────────────────────────────────────

/// BACnetNotifyType is a closed {alarm(0), event(1), ack-notification(2)}
/// production (Clause 21). An out-of-production write is PROPERTY /
/// VALUE_OUT_OF_RANGE (Clause 15.9.1.3: "The value provided is outside the
/// range of values that the property can take on") and leaves the stored
/// value untouched.
#[test]
fn mso_notify_type_rejects_out_of_production_values() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );

    for out_of_production in [3u32, 99, u32::MAX] {
        assert_property_error(
            mso.write_property(
                PropertyIdentifier::NOTIFY_TYPE,
                None,
                PropertyValue::Enumerated(out_of_production),
                None,
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            mso.read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(0),
            "a refused Notify_Type write must leave the value untouched ({out_of_production})"
        );
    }
    for in_production in [0u32, 1, 2] {
        mso.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(in_production),
            None,
        )
        .expect("named Notify_Type values must be accepted");
    }
}

/// BACnetEventTransitionBits is a 3-bit production (Clause 21); its canonical
/// encoding is one content octet with 5 unused bits, which is what the read
/// path emits. A write declaring any other shape is PROPERTY /
/// INVALID_DATA_ENCODING (Clause 15.9.1.3: "The encoding is not valid for the
/// datatype of the property") — including an 8-bit string that would
/// previously have been silently masked to three bits.
#[test]
fn mso_event_enable_rejects_noncanonical_bit_strings() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    let canonical = mso
        .read_property(PropertyIdentifier::EVENT_ENABLE, None)
        .unwrap();

    for (unused_bits, data) in [
        (0u8, vec![0xFFu8]),     // 8-bit string where the production defines 3
        (5u8, vec![0xFF, 0xFF]), // two content octets
        (4u8, vec![0xF0u8]),     // half-octet string
        (5u8, vec![]),           // no content octet
    ] {
        assert_property_error(
            mso.write_property(
                PropertyIdentifier::EVENT_ENABLE,
                None,
                PropertyValue::BitString { unused_bits, data },
                None,
            ),
            ErrorCode::INVALID_DATA_ENCODING,
        );
        assert_eq!(
            mso.read_property(PropertyIdentifier::EVENT_ENABLE, None)
                .unwrap(),
            canonical,
            "a refused Event_Enable write must leave the value untouched"
        );
    }

    // The canonical full-width shape stays accepted.
    mso.write_property(
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
        mso.read_property(PropertyIdentifier::EVENT_ENABLE, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xA0],
        }
    );
}

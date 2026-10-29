//! Logged values named from a Log_DeviceObjectProperty reference, and the
//! fallbacks when it names nothing usable.

use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use std::ops::ControlFlow;

use super::{LogColumns, LoggedProperty, MAX_STATE_TEXT_OBJECTS};
use crate::format::{Datum, Floats, StateTexts, ValueFormat};

fn reference(
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(object_type, instance).unwrap(),
        property.to_raw(),
    )
}

/// References encoded back to back, as a whole read of the property
/// carries them.
fn encoded(references: &[BACnetDeviceObjectPropertyReference]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for reference in references {
        encode_device_object_property_reference(&mut buf, reference);
    }
    buf.to_vec()
}

fn logged(column: Option<LoggedProperty>, value: &PropertyValue) -> String {
    let texts = StateTexts::default();
    ValueFormat::new(ObjectType::TREND_LOG, &texts, Floats::Exact).logged(column, value)
}

fn logged_data(column: Option<LoggedProperty>, value: &PropertyValue) -> Datum {
    let texts = StateTexts::default();
    ValueFormat::new(ObjectType::TREND_LOG, &texts, Floats::Exact).logged_data(column, value)
}

fn status_flags(bits: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![bits << 4],
    }
}

/// An Event_State log names its enumerations and a Status_Flags log its
/// bits, in text and as data.
#[test]
fn event_state_and_status_flags_logs_are_named() {
    let columns = LogColumns::decode(&encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::EVENT_STATE,
    )]));
    let event_state = columns.column(0);
    assert_eq!(
        event_state,
        Some(LoggedProperty {
            object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            property: PropertyIdentifier::EVENT_STATE,
            index: None,
            device: None,
        })
    );
    let offnormal = PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw());
    assert_eq!(logged(event_state, &offnormal), "offnormal");
    assert_eq!(
        logged_data(event_state, &offnormal),
        Datum::Enumerated {
            value: 2,
            name: Some("offnormal".into()),
        }
    );

    let columns = LogColumns::decode(&encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::STATUS_FLAGS,
    )]));
    let flags = columns.column(0);
    // In alarm and out of service: the first and last of the four bits.
    assert_eq!(
        logged(flags, &status_flags(0b1001)),
        "in-alarm | out-of-service"
    );
    assert_eq!(logged(flags, &status_flags(0)), "none");
    assert_eq!(
        logged_data(flags, &status_flags(0b1000)),
        Datum::Bits {
            bits: "1000".into(),
            names: Some(vec!["in-alarm".into()]),
        }
    );
}

/// The monitored object's type names a binary object's state.
#[test]
fn a_binary_present_value_log_names_its_state() {
    let columns = LogColumns::decode(&encoded(&[reference(
        ObjectType::BINARY_VALUE,
        3,
        PropertyIdentifier::PRESENT_VALUE,
    )]));
    let column = columns.column(0);
    assert_eq!(logged(column, &PropertyValue::Enumerated(1)), "active");
    assert_eq!(logged(column, &PropertyValue::Enumerated(0)), "inactive");
    // A number past the two states names nothing.
    assert_eq!(logged(column, &PropertyValue::Enumerated(7)), "enum 7");
}

/// With no column named, and for a value whose datatype the property
/// doesn't name, the value prints as the log gives it.
#[test]
fn unnamed_and_mismatched_values_print_as_logged() {
    let offnormal = PropertyValue::Enumerated(2);
    assert_eq!(logged(None, &offnormal), "enum 2");
    assert_eq!(logged(None, &status_flags(0b1000)), "bits(1000)");
    assert_eq!(
        logged_data(None, &offnormal),
        Datum::Enumerated {
            value: 2,
            name: None,
        }
    );

    let state = LoggedProperty {
        object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property: PropertyIdentifier::EVENT_STATE,
        index: None,
        device: None,
    };
    let event_state = Some(state);
    assert_eq!(logged(event_state, &PropertyValue::Real(72.5)), "72.5");
    assert_eq!(
        logged(event_state, &status_flags(0b1000)),
        "bits(1000)",
        "Event_State has no named bits"
    );
    let flags = Some(LoggedProperty {
        property: PropertyIdentifier::STATUS_FLAGS,
        ..state
    });
    assert_eq!(logged(flags, &offnormal), "enum 2");
    assert_eq!(
        logged_data(flags, &offnormal),
        Datum::Enumerated {
            value: 2,
            name: None,
        }
    );
}

/// A Trend Log Multiple's array names each member's column; an empty
/// element names nothing, and octets that don't decode name no column.
#[test]
fn columns_follow_the_members_and_fall_back_on_bad_octets() {
    let empty = reference(
        ObjectType::ANALOG_INPUT,
        ObjectIdentifier::WILDCARD_INSTANCE,
        PropertyIdentifier::PRESENT_VALUE,
    );
    let columns = LogColumns::decode(&encoded(&[
        reference(
            ObjectType::ANALOG_INPUT,
            1,
            PropertyIdentifier::PRESENT_VALUE,
        ),
        empty.clone(),
        reference(
            ObjectType::BINARY_INPUT,
            2,
            PropertyIdentifier::PRESENT_VALUE,
        ),
    ]));
    assert!(!columns.is_empty());
    assert_eq!(
        columns.column(0).map(|column| column.object.object_type()),
        Some(ObjectType::ANALOG_INPUT)
    );
    assert_eq!(columns.column(1), None);
    assert_eq!(
        logged(columns.column(2), &PropertyValue::Enumerated(1)),
        "active"
    );
    assert_eq!(columns.column(3), None, "past the last member");

    assert!(LogColumns::decode(&encoded(&[empty])).is_empty());
    assert!(LogColumns::decode(&[]).is_empty());
    let mut cut = encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::EVENT_STATE,
    )]);
    cut.pop();
    assert_eq!(LogColumns::decode(&cut), LogColumns::default());
    // A good reference before the cut one names nothing either: the
    // columns would no longer line up with the members.
    let mut tail = encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::STATUS_FLAGS,
    )]);
    tail.extend_from_slice(&cut);
    assert_eq!(LogColumns::decode(&tail), LogColumns::default());
    // An application-tagged value is no reference.
    assert_eq!(LogColumns::decode(&[0x91, 0x02]), LogColumns::default());
}

/// A reference to another device names its column as a local one does,
/// with its array index; one whose Device member is at the reserved
/// instance is empty.
#[test]
fn device_qualified_references_name_their_column() {
    let remote = BACnetDeviceObjectPropertyReference {
        property_array_index: Some(8),
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap()),
        ..reference(
            ObjectType::BINARY_OUTPUT,
            2,
            PropertyIdentifier::PRIORITY_ARRAY,
        )
    };
    let unset_device = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(
            ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE).unwrap(),
        ),
        ..reference(ObjectType::ANALOG_INPUT, 1, PropertyIdentifier::EVENT_STATE)
    };
    let columns = LogColumns::decode(&encoded(&[remote, unset_device]));
    let column = columns.column(0);
    assert_eq!(
        column,
        Some(LoggedProperty {
            object: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 2).unwrap(),
            property: PropertyIdentifier::PRIORITY_ARRAY,
            index: Some(8),
            device: Some(77),
        })
    );
    // A Priority_Array slot of a binary output names its state.
    assert_eq!(logged(column, &PropertyValue::Enumerated(1)), "active");
    assert_eq!(columns.column(1), None, "the Device member is unset");
}

/// Which logs have a Log_DeviceObjectProperty to read.
#[test]
fn only_trend_logs_name_their_columns() {
    assert!(LogColumns::applies(ObjectType::TREND_LOG));
    assert!(LogColumns::applies(ObjectType::TREND_LOG_MULTIPLE));
    assert!(!LogColumns::applies(ObjectType::EVENT_LOG));
    assert!(!LogColumns::applies(ObjectType::AUDIT_LOG));
}

fn state_column(object_type: ObjectType, instance: u32) -> Option<LoggedProperty> {
    Some(LoggedProperty {
        object: ObjectIdentifier::new(object_type, instance).unwrap(),
        property: PropertyIdentifier::PRESENT_VALUE,
        index: None,
        device: None,
    })
}

/// Column `index` of `columns`, as text, named with its state texts.
fn named(columns: &LogColumns, index: usize, value: &PropertyValue) -> String {
    ValueFormat::new(
        ObjectType::TREND_LOG,
        columns.state_texts(index),
        Floats::Exact,
    )
    .logged(columns.column(index), value)
}

/// A binary and a multi-state column are named with their objects' state
/// texts; a state outside a short State_Text, an empty text and a column
/// without texts keep today's names (#2122).
#[tokio::test]
async fn state_texts_name_binary_and_multi_state_columns() {
    let mut columns: LogColumns = [
        state_column(ObjectType::BINARY_VALUE, 1),
        state_column(ObjectType::MULTI_STATE_VALUE, 1),
        state_column(ObjectType::MULTI_STATE_INPUT, 2),
    ]
    .into_iter()
    .collect();
    columns
        .read_state_texts(Some(5), |object| async move {
            ControlFlow::Continue(match object.object_type() {
                ObjectType::BINARY_VALUE => StateTexts {
                    active: Some("Running".into()),
                    inactive: Some(String::new()),
                    states: Vec::new(),
                },
                ObjectType::MULTI_STATE_VALUE => StateTexts {
                    states: vec!["Off".into(), "Lo\nw".into(), "x".repeat(200)],
                    ..StateTexts::default()
                },
                // A refused read.
                _ => StateTexts::default(),
            })
        })
        .await;
    let enumerated = PropertyValue::Enumerated;
    assert_eq!(named(&columns, 0, &enumerated(1)), "active (Running)");
    assert_eq!(named(&columns, 0, &enumerated(0)), "inactive", "empty text");
    // Control characters in a device's text show as spaces.
    assert_eq!(named(&columns, 1, &PropertyValue::Unsigned(2)), "2 (Lo w)");
    assert_eq!(named(&columns, 1, &PropertyValue::Unsigned(1)), "1 (Off)");
    // Past the end of State_Text, and state 0, which no text names.
    assert!(named(&columns, 1, &PropertyValue::Unsigned(3)).starts_with("3 (xxx"));
    assert_eq!(named(&columns, 1, &PropertyValue::Unsigned(4)), "4");
    assert_eq!(named(&columns, 1, &PropertyValue::Unsigned(0)), "0");
    assert_eq!(named(&columns, 2, &PropertyValue::Unsigned(2)), "2");
    // The typed form keeps the formatter's own names: no device text.
    let typed = ValueFormat::new(ObjectType::TREND_LOG, columns.state_texts(0), Floats::Exact)
        .logged_data(columns.column(0), &enumerated(1));
    assert!(!typed.may_hold_device_text(), "{typed:?}");
    // Shown cleaned and cut; kept raw for a front end that cleans its own.
    let cut = "x".repeat(crate::format::MAX_NAME_CHARS);
    assert_eq!(columns.state_texts(1).states, ["Off", "Lo w", cut.as_str()]);
    assert_eq!(columns.raw_state_texts(1).states[1], "Lo\nw");
    assert_eq!(columns.raw_state_texts(1).states[2], "x".repeat(200));
    assert_eq!(columns.raw_state_texts(2), &StateTexts::default());
    assert_eq!(columns.state_texts(9), &StateTexts::default());
}

/// Texts are read only for state columns, once an object, on the log's own
/// device, for at most MAX_STATE_TEXT_OBJECTS objects, and a break ends
/// the reading (#2122).
#[tokio::test]
async fn state_texts_are_read_once_an_object_within_the_cap() {
    let on = |device| {
        Some(LoggedProperty {
            device,
            ..state_column(ObjectType::BINARY_INPUT, 3).unwrap()
        })
    };
    let mut wanted: Vec<Option<LoggedProperty>> = vec![
        state_column(ObjectType::MULTI_STATE_VALUE, 1),
        // The same object again, a non-state property, a non-state object,
        // an empty member, and a binary input on this device and another.
        state_column(ObjectType::MULTI_STATE_VALUE, 1),
        Some(LoggedProperty {
            property: PropertyIdentifier::STATUS_FLAGS,
            ..state_column(ObjectType::MULTI_STATE_VALUE, 2).unwrap()
        }),
        state_column(ObjectType::ANALOG_VALUE, 1),
        None,
        on(Some(5)),
        on(Some(6)),
    ];
    wanted.extend((10..20).map(|instance| state_column(ObjectType::BINARY_VALUE, instance)));
    let oid = |object_type, instance| ObjectIdentifier::new(object_type, instance).unwrap();

    let mut asked = Vec::new();
    let mut columns: LogColumns = wanted.iter().copied().collect();
    columns
        .read_state_texts(Some(5), |object| {
            asked.push(object);
            async { ControlFlow::Continue(StateTexts::default()) }
        })
        .await;
    assert_eq!(asked.len(), MAX_STATE_TEXT_OBJECTS, "{asked:?}");
    assert_eq!(asked[0], oid(ObjectType::MULTI_STATE_VALUE, 1));
    assert_eq!(asked[1], oid(ObjectType::BINARY_INPUT, 3));
    assert_eq!(asked[2], oid(ObjectType::BINARY_VALUE, 10));

    // A log device the client doesn't know reads only unqualified ones.
    let mut asked = Vec::new();
    let mut columns: LogColumns = wanted.iter().copied().collect();
    columns
        .read_state_texts(None, |object| {
            asked.push(object);
            async { ControlFlow::Continue(StateTexts::default()) }
        })
        .await;
    assert!(
        !asked.contains(&oid(ObjectType::BINARY_INPUT, 3)),
        "{asked:?}"
    );

    // A break ends the reading, and the columns read before it keep their
    // texts.
    let mut columns: LogColumns = wanted.iter().copied().collect();
    let mut calls = 0;
    columns
        .read_state_texts(Some(5), |_| {
            calls += 1;
            let first = calls == 1;
            async move {
                if first {
                    ControlFlow::Continue(StateTexts {
                        states: vec!["Off".into()],
                        ..StateTexts::default()
                    })
                } else {
                    ControlFlow::Break(())
                }
            }
        })
        .await;
    assert_eq!(calls, 2);
    assert_eq!(columns.state_texts(0).states, ["Off"]);
    assert_eq!(columns.state_texts(1).states, ["Off"], "the same object");
    assert_eq!(columns.state_texts(5), &StateTexts::default());

    // No state column: nothing is read.
    let mut plain = LogColumns::decode(&encoded(&[reference(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::PRESENT_VALUE,
    )]));
    plain
        .read_state_texts(Some(5), |_| async {
            panic!("an analog column has no state texts")
        })
        .await;
}

//! Event timestamp and property-metadata contracts for enrollment objects.

use super::super::*;
use crate::property_metadata::{PropertyConformance, PropertyMetadata, PropertyWriteCapability};
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

fn assert_default_timestamp_array(object: &dyn BACnetObject, label: &str) {
    let value = object
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
        .unwrap_or_else(|error| panic!("{label}: Event_Time_Stamps read failed: {error:?}"));
    let PropertyValue::List(elements) = value else {
        panic!("{label}: expected Event_Time_Stamps list");
    };
    assert_eq!(elements.len(), 3, "{label}: timestamp count");
    for (slot, value) in elements.into_iter().enumerate() {
        let PropertyValue::ApplicationData(bytes) = value else {
            panic!("{label}: slot {} was not an encoded CHOICE", slot + 1);
        };
        let (decoded, end) = decode_timestamp_choice(&bytes, 0).unwrap();
        assert_eq!(decoded, BACnetTimeStamp::SequenceNumber(0));
        assert_eq!(end, bytes.len(), "{label}: trailing CHOICE bytes");
    }
}

#[test]
fn enrollment_objects_default_event_time_stamps_are_three_zero_sequences() {
    let event = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let alert = AlertEnrollmentObject::new(1, "AE-1").unwrap();

    assert_default_timestamp_array(&event, "Event Enrollment");
    assert_default_timestamp_array(&alert, "Alert Enrollment");
}

fn seeded_timestamps() -> [BACnetTimeStamp; 3] {
    [
        BACnetTimeStamp::Time(Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        }),
        BACnetTimeStamp::SequenceNumber(22),
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 7,
                day: 30,
                day_of_week: 4,
            },
            time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
        },
    ]
}

fn assert_timestamp_value(actual: PropertyValue, expected: &BACnetTimeStamp, context: &str) {
    let PropertyValue::ApplicationData(bytes) = actual else {
        panic!("{context}: expected encoded timestamp CHOICE");
    };
    let (decoded, end) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(&decoded, expected, "{context}: timestamp CHOICE");
    assert_eq!(end, bytes.len(), "{context}: trailing timestamp bytes");
}

fn assert_seeded_timestamp_reads(
    object: &dyn BACnetObject,
    expected: &[BACnetTimeStamp; 3],
    label: &str,
) {
    let whole = object
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
        .unwrap();
    let PropertyValue::List(values) = whole else {
        panic!("{label}: expected timestamp list");
    };
    assert_eq!(values.len(), 3, "{label}: array length");
    for (slot, (actual, expected)) in values.into_iter().zip(expected).enumerate() {
        assert_timestamp_value(actual, expected, &format!("{label} slot {}", slot + 1));
    }

    assert_eq!(
        object
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(3),
        "{label}: array count"
    );
    for (slot, expected) in expected.iter().enumerate() {
        assert_timestamp_value(
            object
                .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(slot as u32 + 1))
                .unwrap(),
            expected,
            &format!("{label} indexed slot {}", slot + 1),
        );
    }
    for index in [4, u32::MAX] {
        assert_property_error(
            object.read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(index)),
            ErrorCode::INVALID_ARRAY_INDEX,
            &format!("{label} index {index}"),
        );
    }
}

fn assert_property_error<T: std::fmt::Debug>(
    result: Result<T, Error>,
    expected: ErrorCode,
    context: &str,
) {
    match result.expect_err(context) {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32, "{context}");
            assert_eq!(code, expected.to_raw() as u32, "{context}");
        }
        other => panic!("{context}: expected property error, got {other:?}"),
    }
}

fn assert_history_surface(object: &mut dyn BACnetObject, label: &str) {
    let properties = object.property_list();
    assert!(
        properties.contains(&PropertyIdentifier::EVENT_TIME_STAMPS),
        "{label}: Event_Time_Stamps missing from Property_List"
    );
    assert!(
        !properties.contains(&PropertyIdentifier::EVENT_MESSAGE_TEXTS),
        "{label}: optional Event_Message_Texts must remain absent"
    );
    assert_property_error(
        object.read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, None),
        ErrorCode::UNKNOWN_PROPERTY,
        &format!("{label}: Event_Message_Texts read"),
    );
    assert_property_error(
        object.write_property(
            PropertyIdentifier::EVENT_TIME_STAMPS,
            None,
            PropertyValue::List(Vec::new()),
            None,
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
        &format!("{label}: Event_Time_Stamps write"),
    );
    assert!(!object.is_writable_property(PropertyIdentifier::EVENT_TIME_STAMPS));
}

#[test]
fn enrollment_event_time_stamp_arrays_preserve_order_indexes_and_choices() {
    let expected = seeded_timestamps();
    let mut event = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    event.event_history.time_stamps = expected.clone();
    let mut alert = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    alert.event_history.time_stamps = expected.clone();

    assert_seeded_timestamp_reads(&event, &expected, "Event Enrollment");
    assert_seeded_timestamp_reads(&alert, &expected, "Alert Enrollment");
    assert_history_surface(&mut event, "Event Enrollment");
    assert_history_surface(&mut alert, "Alert Enrollment");
}

#[test]
fn event_enrollment_detection_disable_resets_history_and_rollback_restores_it() {
    let expected = seeded_timestamps();
    let mut object = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    object.event_history.time_stamps = expected.clone();
    let rollback = object
        .capture_write_property_rollback(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            &PropertyValue::Boolean(false),
        )
        .expect("Event Enrollment detection write needs an opaque rollback");

    object
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_default_timestamp_array(&object, "disabled Event Enrollment");

    object.restore_write_property_rollback(rollback).unwrap();
    assert_seeded_timestamp_reads(&object, &expected, "restored Event Enrollment");
}

#[test]
fn alert_enrollment_disable_projection_reset_and_rollback_cover_history() {
    let expected = seeded_timestamps();
    let mut object = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    object.event_history.time_stamps = expected.clone();
    let rollback = object
        .capture_write_property_rollback(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            &PropertyValue::Boolean(false),
        )
        .expect("Alert Enrollment detection write needs an opaque rollback");

    object.set_event_detection_enable(false);
    assert_default_timestamp_array(&object, "disabled Alert Enrollment");
    object.restore_write_property_rollback(rollback).unwrap();
    assert_seeded_timestamp_reads(&object, &expected, "restored Alert Enrollment");

    object.event_detection_enable = false;
    assert_default_timestamp_array(&object, "directly-disabled Alert Enrollment");
    assert_eq!(object.event_history.time_stamps, expected);

    object.set_event_detection_enable(true);
    assert_default_timestamp_array(&object, "re-enabled Alert Enrollment");
    assert_eq!(object.event_history.time_stamps, seeded_zero_timestamps());
}

fn seeded_zero_timestamps() -> [BACnetTimeStamp; 3] {
    [
        BACnetTimeStamp::SequenceNumber(0),
        BACnetTimeStamp::SequenceNumber(0),
        BACnetTimeStamp::SequenceNumber(0),
    ]
}

fn assert_complete_metadata(
    object: &dyn BACnetObject,
    expected: &[PropertyIdentifier],
    label: &str,
) {
    let metadata = object.property_metadata();
    let actual: Vec<_> = metadata.iter().map(|row| row.property_identifier).collect();
    assert_eq!(actual, expected, "{label}: metadata identifiers");

    let projection: Vec<_> = expected
        .iter()
        .copied()
        .filter(|property| *property != PropertyIdentifier::PROPERTY_LIST)
        .collect();
    assert_eq!(object.property_list().as_ref(), projection.as_slice());

    for row in metadata.iter() {
        object
            .read_property(row.property_identifier, None)
            .unwrap_or_else(|error| {
                panic!(
                    "{label}: metadata row {:?} is unreadable: {error:?}",
                    row.property_identifier
                )
            });
        assert_eq!(
            row.write_capability.is_writable(),
            object.is_writable_property(row.property_identifier),
            "{label}: {:?} writability",
            row.property_identifier
        );
    }
}

fn metadata_row(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyMetadata {
    *object
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == property)
        .unwrap_or_else(|| panic!("missing metadata row for {property:?}"))
}

#[test]
fn enrollment_property_metadata_is_complete_and_pins_timestamp_requirements() {
    let event = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let event_ids = [
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::DESCRIPTION,
        PropertyIdentifier::OBJECT_TYPE,
        PropertyIdentifier::EVENT_TYPE,
        PropertyIdentifier::NOTIFY_TYPE,
        PropertyIdentifier::EVENT_PARAMETERS,
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::FAULT_TYPE,
        PropertyIdentifier::FAULT_PARAMETERS,
        PropertyIdentifier::TIME_DELAY_NORMAL,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
        PropertyIdentifier::PROPERTY_LIST,
    ];
    assert_complete_metadata(&event, &event_ids, "Event Enrollment");

    let alert = AlertEnrollmentObject::new(1, "AE-1").unwrap();
    let alert_ids = [
        PropertyIdentifier::OBJECT_IDENTIFIER,
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::DESCRIPTION,
        PropertyIdentifier::OBJECT_TYPE,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyIdentifier::EVENT_ENABLE,
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyIdentifier::EVENT_TIME_STAMPS,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::RELIABILITY,
        PropertyIdentifier::PROPERTY_LIST,
    ];
    assert_complete_metadata(&alert, &alert_ids, "Alert Enrollment");

    for object in [&event as &dyn BACnetObject, &alert as &dyn BACnetObject] {
        let timestamps = metadata_row(object, PropertyIdentifier::EVENT_TIME_STAMPS);
        assert_eq!(timestamps.conformance, PropertyConformance::RequiredRead);
        assert_eq!(
            timestamps.write_capability,
            PropertyWriteCapability::ReadOnly
        );
    }

    assert!(alert
        .property_metadata()
        .iter()
        .all(|row| row.property_identifier != PropertyIdentifier::NOTIFY_TYPE));
    assert!(!alert
        .property_list()
        .contains(&PropertyIdentifier::NOTIFY_TYPE));
    assert_property_error(
        alert.read_property(PropertyIdentifier::NOTIFY_TYPE, None),
        ErrorCode::UNKNOWN_PROPERTY,
        "Alert Enrollment Notify_Type remains an explicit model limitation",
    );
}

#[test]
fn alert_to_normal_acknowledgment_cannot_be_cleared() {
    let mut alert = AlertEnrollmentObject::new(1, "AE-1").unwrap();

    alert.set_acked_transitions_internal(0x04, false).unwrap();
    assert_eq!(alert.acked_transitions, 0b111);

    alert.set_acked_transitions_internal(0x01, false).unwrap();
    assert_eq!(alert.acked_transitions, 0b110);
    alert.set_acked_transitions_internal(0x02, false).unwrap();
    assert_eq!(alert.acked_transitions, 0b100);
    alert.set_acked_transitions_internal(0x01, true).unwrap();
    alert.set_acked_transitions_internal(0x02, true).unwrap();
    assert_eq!(alert.acked_transitions, 0b111);

    alert.set_event_detection_enable(false);
    assert!(alert.set_acked_transitions_internal(0x04, false).is_err());
}

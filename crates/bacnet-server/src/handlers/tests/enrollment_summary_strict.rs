use bacnet_objects::event::EventTransition;
use bacnet_services::enrollment_summary::GetEnrollmentSummaryRequest;
use bacnet_types::enums::EventType;

use super::enrollment_summary_support::*;
use super::*;

fn candidate(
    state: EventState,
    acknowledged: u8,
    transition: Option<EventTransition>,
) -> SummaryFixture {
    SummaryFixture::candidate(
        1,
        EventType::OUT_OF_RANGE,
        state,
        acknowledged,
        7,
        transition,
    )
}

fn database(candidate: SummaryFixture) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate)).unwrap();
    db.add(Box::new(class(7, 19, [11, 22, 33], Vec::new())))
        .unwrap();
    db
}

#[test]
fn most_recent_coordinate_selects_offnormal_fault_and_normal_priority() {
    for (state, transition, priority) in [
        (EventState::HIGH_LIMIT, EventTransition::ToOffnormal, 11),
        (EventState::FAULT, EventTransition::ToFault, 22),
        (EventState::NORMAL, EventTransition::ToNormal, 33),
    ] {
        let db = database(candidate(state, 0b111, Some(transition)));
        let ack = response(&db, &request()).unwrap();
        assert_eq!(ack.entries.len(), 1);
        let entry = &ack.entries[0];
        assert_eq!(entry.object_identifier.instance_number(), 1);
        assert_eq!(entry.event_type, EventType::OUT_OF_RANGE);
        assert_eq!(entry.event_state, state);
        assert_eq!(entry.priority, priority);
        assert_eq!(entry.notification_class, Some(7));
    }
}

#[test]
fn no_history_uses_normal_priority_only_for_canonical_initial_state() {
    let db = database(candidate(EventState::NORMAL, 0b111, None));
    assert_eq!(response(&db, &request()).unwrap().entries[0].priority, 33);

    for (state, acknowledged) in [(EventState::OFFNORMAL, 0b111), (EventState::NORMAL, 0b110)] {
        let db = database(candidate(state, acknowledged, None));
        assert_operational_problem(response(&db, &request()).unwrap_err());
    }
}

#[test]
fn acknowledged_transitions_requires_one_canonical_three_bit_octet() {
    for malformed in [
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0xe0],
        },
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xe0, 0],
        },
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xe1],
        },
        PropertyValue::Boolean(true),
    ] {
        let mut object = candidate(
            EventState::OFFNORMAL,
            0b111,
            Some(EventTransition::ToOffnormal),
        );
        object.set(PropertyIdentifier::ACKED_TRANSITIONS, malformed);
        let db = database(object);
        assert_operational_problem(response(&db, &request()).unwrap_err());
    }
}

#[test]
fn detection_false_excludes_before_other_malformed_fields() {
    let mut object = candidate(
        EventState::OFFNORMAL,
        0b111,
        Some(EventTransition::ToOffnormal),
    );
    object.advertise(PropertyIdentifier::EVENT_DETECTION_ENABLE);
    object.set(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        PropertyValue::Boolean(false),
    );
    object.set(
        PropertyIdentifier::EVENT_STATE,
        PropertyValue::Boolean(false),
    );
    object.set(
        PropertyIdentifier::ACKED_TRANSITIONS,
        PropertyValue::Boolean(false),
    );
    object.set(
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyValue::Boolean(false),
    );
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();

    assert!(response(&db, &request()).unwrap().entries.is_empty());
}

#[test]
fn detection_field_is_strict_when_advertised() {
    for value in [None, Some(PropertyValue::Unsigned(1))] {
        let mut object = candidate(
            EventState::OFFNORMAL,
            0b111,
            Some(EventTransition::ToOffnormal),
        );
        object.advertise(PropertyIdentifier::EVENT_DETECTION_ENABLE);
        if let Some(value) = value {
            object.set(PropertyIdentifier::EVENT_DETECTION_ENABLE, value);
        }
        let db = database(object);
        assert_operational_problem(response(&db, &request()).unwrap_err());
    }
}

#[test]
fn objects_without_explicit_capability_are_excluded_without_property_reads() {
    let object = candidate(EventState::OFFNORMAL, 0b111, None).without_capability();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();

    assert!(response(&db, &request()).unwrap().entries.is_empty());
}

#[test]
fn class_resolution_uses_direct_instance_and_ignores_other_own_property_values() {
    let db = database(candidate(
        EventState::OFFNORMAL,
        0b111,
        Some(EventTransition::ToOffnormal),
    ));
    assert_eq!(response(&db, &request()).unwrap().entries[0].priority, 11);

    let mut with_unrelated_class = db;
    with_unrelated_class
        .add(Box::new(class(20, 7, [1, 2, 3], Vec::new())))
        .unwrap();
    assert_eq!(
        response(&with_unrelated_class, &request()).unwrap().entries[0].priority,
        11
    );
}

#[test]
fn missing_direct_class_instance_fails_even_when_another_class_uses_number() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(candidate(
        EventState::OFFNORMAL,
        0b111,
        Some(EventTransition::ToOffnormal),
    )))
    .unwrap();
    db.add(Box::new(class(19, 7, [1, 2, 3], Vec::new())))
        .unwrap();
    assert_operational_problem(response(&db, &request()).unwrap_err());
}

#[test]
fn direct_class_own_notification_class_is_not_required_or_validated() {
    let priority = Some(PropertyValue::List(vec![
        PropertyValue::Unsigned(1),
        PropertyValue::Unsigned(2),
        PropertyValue::Unsigned(3),
    ]));
    for class_value in [None, Some(PropertyValue::Boolean(true))] {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(candidate(
            EventState::OFFNORMAL,
            0b111,
            Some(EventTransition::ToOffnormal),
        )))
        .unwrap();
        db.add(Box::new(SummaryFixture::notification_class(
            7,
            class_value,
            priority.clone(),
            None,
        )))
        .unwrap();
        assert_eq!(response(&db, &request()).unwrap().entries[0].priority, 1);
    }
}

#[test]
fn unreadable_and_malformed_priority_fail_strictly() {
    for priority in [
        None,
        Some(PropertyValue::Unsigned(1)),
        Some(PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Unsigned(2),
        ])),
        Some(PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Boolean(false),
            PropertyValue::Unsigned(3),
        ])),
        Some(PropertyValue::List(vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Unsigned(256),
            PropertyValue::Unsigned(3),
        ])),
    ] {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(candidate(
            EventState::OFFNORMAL,
            0b111,
            Some(EventTransition::ToOffnormal),
        )))
        .unwrap();
        db.add(Box::new(SummaryFixture::notification_class(
            7, None, priority, None,
        )))
        .unwrap();
        assert_operational_problem(response(&db, &request()).unwrap_err());
    }
}

#[test]
fn malformed_required_candidate_fields_are_operational_problems() {
    for property in [
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::NOTIFICATION_CLASS,
    ] {
        let mut object = candidate(
            EventState::OFFNORMAL,
            0b111,
            Some(EventTransition::ToOffnormal),
        );
        object.set(property, PropertyValue::Boolean(false));
        assert_operational_problem(response(&database(object), &request()).unwrap_err());
    }

    let mut object = candidate(
        EventState::OFFNORMAL,
        0b111,
        Some(EventTransition::ToOffnormal),
    );
    object.set(
        PropertyIdentifier::NOTIFICATION_CLASS,
        PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
    );
    assert_operational_problem(response(&database(object), &request()).unwrap_err());
}

#[test]
fn empty_candidate_set_returns_positive_zero_length_ack() {
    let encoded_request = {
        let mut encoded = BytesMut::new();
        request().encode(&mut encoded);
        encoded
    };
    let mut encoded_ack = BytesMut::new();
    handle_get_enrollment_summary(&ObjectDatabase::new(), &encoded_request, &mut encoded_ack)
        .unwrap();
    assert!(encoded_ack.is_empty());
    assert!(
        bacnet_services::enrollment_summary::GetEnrollmentSummaryAck::decode(&encoded_ack)
            .unwrap()
            .entries
            .is_empty()
    );
}

#[test]
fn malformed_request_is_rejected_before_response_bytes_are_written() {
    let request = GetEnrollmentSummaryRequest {
        acknowledgment_filter: 0,
        ..request()
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);
    encoded.extend_from_slice(&[0]);
    let mut response_bytes = BytesMut::new();
    assert!(
        handle_get_enrollment_summary(&ObjectDatabase::new(), &encoded, &mut response_bytes)
            .is_err()
    );
    assert!(response_bytes.is_empty());
}

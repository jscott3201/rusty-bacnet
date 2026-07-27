//! `Event_Detection_Enable` filtering in the three event-summarization
//! services (ASHRAE 135-2020 Clause 13.2.4 Table 13-2, and the Service
//! Procedures of Clauses 13.10.1.4, 13.11.1.4 and 13.12.1.4).
//!
//! Each of those three clauses carries its own independent "shall be ignored"
//! for an object whose `Event_Detection_Enable` is FALSE, so each service is
//! tested separately rather than trusting one shared code path.

use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{EventState, EventType};

/// An enrollment sitting in HIGH_LIMIT, with `Event_Detection_Enable` as given.
///
/// When disabling, `set_event_state` runs *after* the write so the object is
/// left in the inconsistent state a non-conformant peer or a persistence
/// restore could produce. That keeps these tests honest: they must prove the
/// services filter on the property, not merely that the reset happened to move
/// `Event_State` to NORMAL.
fn db_with_enrollment(detection_enabled: bool) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();

    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    if !detection_enabled {
        ee.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    }
    ee.set_event_state(EventState::HIGH_LIMIT.to_raw());
    db.add(Box::new(ee)).unwrap();
    db
}

fn alarm_summary_entry_count(db: &ObjectDatabase) -> usize {
    use bacnet_services::alarm_summary::GetAlarmSummaryAck;
    let mut buf = BytesMut::new();
    handle_get_alarm_summary(db, &mut buf).unwrap();
    GetAlarmSummaryAck::decode(&buf).unwrap().entries.len()
}

/// Clause 13.10.1.4: "Any object that has an Event_Detection_Enable property
/// with a value of FALSE shall be ignored."
#[test]
fn get_alarm_summary_excludes_detection_disabled_object() {
    assert_eq!(
        alarm_summary_entry_count(&db_with_enrollment(true)),
        1,
        "control: an alarming enrollment is reported when detection is enabled"
    );
    assert_eq!(
        alarm_summary_entry_count(&db_with_enrollment(false)),
        0,
        "an object with Event_Detection_Enable FALSE shall be ignored"
    );
}

/// The exclusion tests the property, not the absence of the property.
///
/// Clause 13.12.1.4 phrases it as a double negative — objects that "do not have
/// an Event_Detection_Enable property with a value of FALSE" are searched — so
/// an object that does not model the property at all must still be reported.
/// Getting this backwards would silently empty these responses for every object
/// type that lacks the property, which is most of them.
#[test]
fn summarization_includes_objects_without_the_property() {
    use bacnet_objects::event::LimitEnable;

    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    for (p, v) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
        (PropertyIdentifier::LOW_LIMIT, 20.0),
        (PropertyIdentifier::DEADBAND, 2.0),
    ] {
        ai.write_property(p, None, PropertyValue::Real(v), None)
            .unwrap();
    }
    ai.write_property(
        PropertyIdentifier::LIMIT_ENABLE,
        None,
        PropertyValue::BitString {
            unused_bits: 6,
            data: vec![LimitEnable::BOTH.to_bits()],
        },
        None,
    )
    .unwrap();
    ai.set_present_value(85.0);
    ai.evaluate_intrinsic_reporting(); // -> HIGH_LIMIT

    // Precondition: this object type really does lack the property.
    assert!(
        ai.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
            .is_err(),
        "fixture assumes AnalogInput does not model Event_Detection_Enable"
    );
    db.add(Box::new(ai)).unwrap();

    assert_eq!(
        alarm_summary_entry_count(&db),
        1,
        "an object with no Event_Detection_Enable property must still be reported"
    );
}

/// Clause 13.11.1.4 carries the same exclusion. This service matters most for
/// the check: unlike the other two it applies no default `Event_State` filter,
/// so the exclusion cannot fall out of the forced-NORMAL invariant and has to
/// be implemented explicitly.
#[test]
fn get_enrollment_summary_excludes_detection_disabled_object() {
    use bacnet_services::enrollment_summary::{
        GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
    };

    let count = |db: &ObjectDatabase| {
        let request = GetEnrollmentSummaryRequest {
            acknowledgment_filter: 0, // all
            enrollment_filter: None,
            event_state_filter: None,
            event_type_filter: None,
            priority_filter: None,
            notification_class_filter: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let mut ack = BytesMut::new();
        handle_get_enrollment_summary(db, &buf, &mut ack).unwrap();
        GetEnrollmentSummaryAck::decode(&ack).unwrap().entries.len()
    };

    assert_eq!(
        count(&db_with_enrollment(true)),
        1,
        "control: the enrollment is summarized when detection is enabled"
    );
    assert_eq!(
        count(&db_with_enrollment(false)),
        0,
        "Event_Detection_Enable FALSE excludes the object from the summary"
    );
}

/// Clause 13.12.1.4 carries the exclusion for GetEventInformation, the one
/// summarization service Clause 13.2.4 requires notification-servers to
/// support (the other two are deprecated).
#[test]
fn get_event_information_excludes_detection_disabled_object() {
    use bacnet_services::alarm_event::GetEventInformationRequest;

    let responses = [true, false].map(|enabled| {
        let db = db_with_enrollment(enabled);
        let request = GetEventInformationRequest {
            last_received_object_identifier: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);
        let mut ack = BytesMut::new();
        handle_get_event_information(&db, &buf, &mut ack).unwrap();
        ack.to_vec()
    });

    assert_ne!(
        responses[0], responses[1],
        "disabling detection must change the GetEventInformation response"
    );
    // The enabled response carries the object identifier; the disabled one is
    // the shorter empty-list form.
    assert!(
        responses[1].len() < responses[0].len(),
        "the disabled object must be absent from the response, got {} vs {} bytes",
        responses[1].len(),
        responses[0].len()
    );
}

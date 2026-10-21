//! An Event Enrollment whose Event_Parameters are `none` evaluates
//! reliability only (Clause 12.12.7, #1868): when its monitored object
//! faults it goes TO_FAULT, and that transition is listed by
//! GetEnrollmentSummary (Table 13-2) and GetEventInformation and takes an
//! AcknowledgeAlarm (Table 13-10). So does one whose parameters the stack
//! doesn't model, such as access-event. One that can't fault stays out.

use super::enrollment_summary_support::{request, response};
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_services::alarm_event::{
    AcknowledgeAlarmRequest, GetEventInformationAck, GetEventInformationRequest,
};
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{EngineeringUnits, EventType, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;

/// AI-1 out of service, monitored by EE-5 with unset (`none`) parameters,
/// on Notification Class 5, which wants TO_FAULT acknowledged.
fn make_db(with_reference: bool) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    make_db_with(with_reference, None)
}

/// [`make_db`], with `parameters` set when given.
fn make_db_with(
    with_reference: bool,
    parameters: Option<BACnetEventParameter>,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    // The type argument only names the legacy octet layout's algorithm.
    let mut ee = EventEnrollmentObject::new(5, "EE-none", EventType::OUT_OF_RANGE).unwrap();
    if with_reference {
        ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        )))
        .unwrap();
    }
    if let Some(parameters) = parameters {
        ee.set_event_parameters(parameters);
    }
    ee.set_event_enable(EventTransitionBits::all());
    ee.set_notification_class(5);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let mut nc = NotificationClass::new(5, "NC-5").unwrap();
    nc.ack_required = EventTransitionBits::TO_FAULT;
    db.add(Box::new(nc)).unwrap();
    // AI-1 reports intrinsically on Notification Class 0.
    db.add(Box::new(NotificationClass::new(0, "NC-0").unwrap()))
        .unwrap();
    (db, ee_oid, ai_oid)
}

fn fault(db: &mut ObjectDatabase, ai_oid: ObjectIdentifier) {
    db.get_mut(&ai_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::OVER_RANGE.to_raw()),
            None,
        )
        .unwrap();
}

fn event_information(db: &ObjectDatabase) -> GetEventInformationAck {
    let mut buf = BytesMut::new();
    GetEventInformationRequest {
        last_received_object_identifier: None,
    }
    .encode(&mut buf);
    let mut ack = BytesMut::new();
    handle_get_event_information(db, &buf, &mut ack).unwrap();
    GetEventInformationAck::decode(&ack).unwrap()
}

fn to_fault_stamp(db: &ObjectDatabase, ee_oid: ObjectIdentifier) -> BACnetTimeStamp {
    let PropertyValue::ApplicationData(bytes) = db
        .get(&ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, Some(2))
        .unwrap()
    else {
        panic!("a timestamp CHOICE");
    };
    bacnet_encoding::primitives::decode_timestamp_choice(&bytes, 0)
        .unwrap()
        .0
}

#[test]
fn a_none_enrollment_that_faults_is_listed_and_acknowledged() {
    fault_listed_and_acknowledged(None, EventType::NONE);
}

/// access-event [13] is kept as opaque octets and runs no algorithm, but
/// the enrollment still faults on its monitored object.
#[test]
fn an_unmodeled_enrollment_that_faults_is_listed_and_acknowledged() {
    let access_event = BACnetEventParameter::Opaque {
        tag: 13,
        data: vec![0x09, 0x00],
    };
    fault_listed_and_acknowledged(Some(access_event), EventType::from_raw(13));
}

fn fault_listed_and_acknowledged(parameters: Option<BACnetEventParameter>, listed: EventType) {
    let (mut db, ee_oid, ai_oid) = make_db_with(true, parameters);
    fault(&mut db, ai_oid);
    let report = crate::event_enrollment::evaluate_event_enrollments_report(&mut db, 1);
    assert_eq!(
        report.reliability_results[0].new_reliability,
        Reliability::MONITORED_OBJECT_FAULT
    );
    let state = db
        .get(&ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(state, PropertyValue::Enumerated(EventState::FAULT.to_raw()));

    // Listed, with Event_Type NONE.
    let summary = response(&db, &request()).unwrap();
    let entry = summary
        .entries
        .iter()
        .find(|entry| entry.object_identifier == ee_oid)
        .expect("a fault-capable enrollment is listed");
    assert_eq!(entry.event_type, listed);
    assert_eq!(entry.event_state, EventState::FAULT);

    // GetEventInformation shows it with TO_FAULT owed.
    let owed = |db: &ObjectDatabase| {
        event_information(db)
            .list_of_event_summaries
            .into_iter()
            .find(|summary| summary.object_identifier == ee_oid)
            .expect("in GetEventInformation while FAULT")
            .acknowledged_transitions
            & EventTransitionBits::TO_FAULT
    };
    assert_eq!(owed(&db), EventTransitionBits::empty());

    let mut ack = BytesMut::new();
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ee_oid,
        event_state_acknowledged: EventState::FAULT,
        timestamp: to_fault_stamp(&db, ee_oid),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(9),
    }
    .encode(&mut ack)
    .unwrap();
    handle_acknowledge_alarm(&mut db, &ack).unwrap();
    assert_eq!(owed(&db), EventTransitionBits::TO_FAULT);
}

#[test]
fn a_none_enrollment_that_cant_fault_stays_out() {
    // No monitored reference: nothing to fault on.
    let (db, ee_oid, _) = make_db(false);
    let summary = response(&db, &request()).unwrap();
    assert!(summary
        .entries
        .iter()
        .all(|e| e.object_identifier != ee_oid));

    // Detection disabled: the handler leaves it out too.
    let (mut db, ee_oid, _) = make_db(true);
    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    let summary = response(&db, &request()).unwrap();
    assert!(summary
        .entries
        .iter()
        .all(|e| e.object_identifier != ee_oid));
}

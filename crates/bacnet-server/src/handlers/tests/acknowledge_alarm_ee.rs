//! AcknowledgeAlarm over the wire for EventEnrollment (PR-#290 review
//! blocker 3): EE transitions maintain `Acked_Transitions` per Clause 13.2.3
//! (the evaluator clears a bit whose Notification Class requires ack), so
//! AcknowledgeAlarm on an EE must actually succeed — previously the trait
//! default rejected every ack with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED and
//! a required acknowledgment could never arrive.

use super::*;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_services::alarm_event::{AcknowledgeAlarmRequest, GetEventInformationRequest};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
};
use bacnet_types::enums::EventType;
use bacnet_types::primitives::BACnetTimeStamp;

/// Database: BI-1 monitored by a COS enrollment (EE-7) referencing
/// NotificationClass 7, which requires acknowledgment of TO_OFFNORMAL.
fn make_db_with_ack_required_ee() -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_present_value(1); // in the alarm list below
    let bi_oid = bi.object_identifier();
    db.add(Box::new(bi)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(7, "EE-COS", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        bi_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: vec![BACnetPropertyStates::UnsignedValue(1)],
    });
    ee.set_event_enable(0x07);
    ee.set_notification_class(7);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let mut nc = NotificationClass::new(7, "NC-7").unwrap();
    nc.ack_required = [true, false, false]; // TO_OFFNORMAL requires ack
    db.add(Box::new(nc)).unwrap();

    (db, ee_oid)
}

fn ack_request(ee_oid: ObjectIdentifier, event_state: u32) -> bytes::BytesMut {
    let request = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ee_oid,
        event_state_acknowledged: event_state,
        timestamp: BACnetTimeStamp::SequenceNumber(1),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(2),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();
    buf
}

fn gei_summary_acked(db: &ObjectDatabase, ee_oid: ObjectIdentifier) -> u8 {
    let request = GetEventInformationRequest {
        last_received_object_identifier: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_get_event_information(db, &buf, &mut ack_buf).unwrap();
    let ack = bacnet_services::alarm_event::GetEventInformationAck::decode(&ack_buf).unwrap();
    let summary = ack
        .list_of_event_summaries
        .iter()
        .find(|s| s.object_identifier == ee_oid)
        .expect("EE must appear in GetEventInformation while non-NORMAL");
    summary.acknowledged_transitions
}

/// The full loop: ack-required transition fires -> GEI shows the
/// TO_OFFNORMAL bit cleared (ack owed) -> AcknowledgeAlarm succeeds -> the
/// bit is set -> GEI shows it set. A duplicate ack is idempotent per Clause
/// 13.2.3's unconditional "is set".
#[test]
fn ee_acknowledge_alarm_round_trip_over_services() {
    let (mut db, ee_oid) = make_db_with_ack_required_ee();

    // Fire the ack-required NORMAL -> OFFNORMAL transition through the
    // evaluated path (#166's actions: 13.2.3 clears the owed bit).
    let transitions = crate::event_enrollment::evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    assert_eq!(
        gei_summary_acked(&db, ee_oid) & 0x01,
        0,
        "TO_OFFNORMAL ack owed: GEI shows the bit cleared"
    );

    // Acknowledge TO_OFFNORMAL over the service (OFFNORMAL matches any
    // offnormal 'To State', Table 13-9).
    handle_acknowledge_alarm(
        &mut db,
        &ack_request(ee_oid, EventState::OFFNORMAL.to_raw()),
    )
    .unwrap();
    assert_eq!(
        gei_summary_acked(&db, ee_oid) & 0x01,
        0x01,
        "after the ack the bit is set"
    );

    // Duplicate ack: 13.2.3 sets the bit unconditionally — idempotent
    // success, and the other bits are untouched.
    handle_acknowledge_alarm(
        &mut db,
        &ack_request(ee_oid, EventState::OFFNORMAL.to_raw()),
    )
    .unwrap();
    assert_eq!(gei_summary_acked(&db, ee_oid), 0b111);
}

/// A TO_NORMAL ack on an EE is equally serviceable (13.9's state matching:
/// NORMAL matches only NORMAL).
#[test]
fn ee_acknowledge_to_normal_bit() {
    let (mut db, ee_oid) = make_db_with_ack_required_ee();
    db.get_mut(&ee_oid)
        .unwrap()
        .set_acked_transitions_internal(0x04, false)
        .unwrap();
    handle_acknowledge_alarm(&mut db, &ack_request(ee_oid, EventState::NORMAL.to_raw())).unwrap();
    match db
        .get(&ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    {
        PropertyValue::BitString { data, .. } => {
            assert_eq!(bacnet_types::bitstring::unpack_octet(&data, 3), 0b111)
        }
        other => panic!("expected BitString, got {other:?}"),
    }
}

/// Table 13-10: an EE with `Event_Detection_Enable` FALSE "does not support
/// or is not configured for event generation" — the ack fails
/// OBJECT / NO_ALARM_CONFIGURED, and the initial-condition
/// `Acked_Transitions` it must hold (Clause 12.12) is untouched.
#[test]
fn ee_acknowledge_alarm_detection_disabled_refused() {
    let (mut db, ee_oid) = make_db_with_ack_required_ee();
    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

    let err = handle_acknowledge_alarm(
        &mut db,
        &ack_request(ee_oid, EventState::OFFNORMAL.to_raw()),
    )
    .unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::OBJECT.to_raw() as u32);
            assert_eq!(code, ErrorCode::NO_ALARM_CONFIGURED.to_raw() as u32);
        }
        other => panic!("expected OBJECT/NO_ALARM_CONFIGURED, got {other:?}"),
    }
    match db
        .get(&ee_oid)
        .unwrap()
        .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
        .unwrap()
    {
        PropertyValue::BitString { data, .. } => assert_eq!(
            bacnet_types::bitstring::unpack_octet(&data, 3),
            0b111,
            "the refused ack must not disturb the initial condition"
        ),
        other => panic!("expected BitString, got {other:?}"),
    }
}

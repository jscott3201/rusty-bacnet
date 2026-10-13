//! The access-control objects' runtime route (#1132):
//! `report_access_input_internal` takes each object's own record whole,
//! follows each object's out-of-service rule (the point refuses, the reader
//! and the door set the input aside), and fails closed on any other object
//! or record.

use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat};
use bacnet_types::enums::{
    AuthenticationFactorType, ErrorClass, ErrorCode, PropertyIdentifier as P,
};

use super::*;
use crate::analog::AnalogValueObject;

fn assert_error(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class: c, code: e })
            if c == class.to_raw() as u32 && e == code.to_raw() as u32),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

fn read(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn set_out_of_service(object: &mut dyn BACnetObject, out_of_service: bool) {
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
}

fn card(format_type: AuthenticationFactorType) -> BACnetAuthenticationFactor {
    BACnetAuthenticationFactor {
        format_type,
        format_class: 0,
        value: vec![0x12, 0x34, 0x56],
    }
}

/// [`card`] for Wiegand 26, as served.
fn card_value() -> PropertyValue {
    PropertyValue::ApplicationData(vec![0x09, 0x08, 0x19, 0x00, 0x2B, 0x12, 0x34, 0x56])
}

fn event_rows(point: &AccessPointObject) -> Vec<PropertyValue> {
    [
        P::ACCESS_EVENT,
        P::ACCESS_EVENT_TAG,
        P::ACCESS_EVENT_TIME,
        P::ACCESS_EVENT_CREDENTIAL,
        P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
    ]
    .map(|property| read(point, property))
    .to_vec()
}

#[test]
fn access_point_takes_an_event_record_whole() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    let badge = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 5).unwrap();
    let report = AccessEventReport {
        time: Some(BACnetTimeStamp::SequenceNumber(9)),
        credential: Some(badge.into()),
        authentication_factor: Some(card(AuthenticationFactorType::WIEGAND26)),
        ..AccessEventReport::new(AccessEvent::GRANTED, 7)
    };
    point
        .report_access_input_internal(AccessControlInput::AccessEvent(report))
        .unwrap();
    assert_eq!(
        event_rows(&point),
        [
            PropertyValue::Enumerated(AccessEvent::GRANTED.to_raw()),
            PropertyValue::Unsigned(7),
            PropertyValue::ApplicationData(vec![0x19, 9]),
            PropertyValue::ApplicationData(vec![0x1C, 0x08, 0x00, 0x00, 0x05]),
            card_value(),
        ]
    );

    // With no time and no usable clock the time served counts on by one;
    // with no factor the UNDEFINED one is stored.
    point
        .report_access_input_internal(AccessControlInput::AccessEvent(AccessEventReport::new(
            AccessEvent::DENIED_OTHER,
            65_536,
        )))
        .unwrap();
    let rows = event_rows(&point);
    assert_eq!(rows[2], PropertyValue::ApplicationData(vec![0x19, 10]));
    assert_eq!(
        rows[4],
        PropertyValue::ApplicationData(vec![0x09, 0x00, 0x19, 0x00, 0x28])
    );

    // A factor outside the closed production is refused, and nothing moves.
    let before = event_rows(&point);
    let refused = AccessEventReport {
        authentication_factor: Some(card(AuthenticationFactorType::from_raw(25))),
        ..AccessEventReport::new(AccessEvent::GRANTED, 8)
    };
    assert_error(
        point.report_access_input_internal(AccessControlInput::AccessEvent(refused)),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(event_rows(&point), before);
}

#[test]
fn access_point_takes_only_events_in_the_production() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    // Named events, and the proprietary range 512..=65535 (Clause 21).
    for raw in [0, 16, 128, 164, 512, 65_535] {
        let event = AccessEvent::from_raw(raw);
        point
            .report_access_input_internal(AccessControlInput::AccessEvent(AccessEventReport::new(
                event,
                u64::from(raw),
            )))
            .unwrap();
        assert_eq!(
            read(&point, P::ACCESS_EVENT),
            PropertyValue::Enumerated(raw)
        );
    }
    // Reserved values and values past the Unsigned16 range change nothing,
    // through the route or the setter.
    let before = event_rows(&point);
    for raw in [17, 127, 165, 511, 65_536] {
        let report = AccessEventReport::new(AccessEvent::from_raw(raw), 1);
        assert_error(
            point.report_access_input_internal(AccessControlInput::AccessEvent(report.clone())),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_error(
            point.set_access_event(report),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(event_rows(&point), before);
    }
}

#[test]
fn access_point_refuses_events_while_unreliable() {
    use bacnet_types::constructed::BACnetAuthenticationPolicy;
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    // An empty policy is a configuration error (Clause 12.31.12), and an
    // unreliable point generates no access events (Clause 12.31.7).
    point
        .set_authentication_policies([("empty", BACnetAuthenticationPolicy::default())])
        .unwrap();
    assert_eq!(
        read(&point, P::RELIABILITY),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    let before = event_rows(&point);
    let granted =
        || AccessControlInput::AccessEvent(AccessEventReport::new(AccessEvent::GRANTED, 1));
    assert_error(
        point.report_access_input_internal(granted()),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(event_rows(&point), before);
    // Fixed, the point takes events again.
    let card_reader = ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, 1).unwrap();
    point
        .set_authentication_policies([(
            "card",
            BACnetAuthenticationPolicy {
                policy: vec![bacnet_types::constructed::BACnetAuthenticationPolicyEntry {
                    credential_data_input: card_reader.into(),
                    index: 1,
                }],
                order_enforced: false,
                timeout: 0,
            },
        )])
        .unwrap();
    point
        .write_property(
            P::ACTIVE_AUTHENTICATION_POLICY,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    point.report_access_input_internal(granted()).unwrap();
    assert_eq!(
        read(&point, P::ACCESS_EVENT),
        PropertyValue::Enumerated(AccessEvent::GRANTED.to_raw())
    );
}

#[test]
fn access_point_refuses_events_out_of_service_and_other_records() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    set_out_of_service(&mut point, true);
    // The entry edge recorded OUT_OF_SERVICE; an event now changes nothing
    // (Clause 12.31.8).
    let before = event_rows(&point);
    assert_error(
        point.report_access_input_internal(AccessControlInput::AccessEvent(
            AccessEventReport::new(AccessEvent::GRANTED, 9),
        )),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(event_rows(&point), before);
    // The setter still takes one, to set a point up before the server holds
    // it.
    point
        .set_access_event(AccessEventReport::new(AccessEvent::GRANTED, 9))
        .unwrap();
    assert_eq!(
        read(&point, P::ACCESS_EVENT),
        PropertyValue::Enumerated(AccessEvent::GRANTED.to_raw())
    );
    for input in [
        AccessControlInput::DoorState(DoorStateReport::default()),
        AccessControlInput::CredentialRead(CredentialReadReport::new(card(
            AuthenticationFactorType::WIEGAND26,
        ))),
    ] {
        assert_error(
            point.report_access_input_internal(input),
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        );
    }
}

#[test]
fn credential_data_input_takes_a_read_and_stamps_a_missing_time() {
    let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    reader
        .set_supported_formats([(
            BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::WIEGAND26),
            0,
        )])
        .unwrap();
    let read_card = |reader: &mut CredentialDataInputObject, format_type| {
        reader.report_access_input_internal(AccessControlInput::CredentialRead(
            CredentialReadReport::new(card(format_type)),
        ))
    };
    // With no usable clock each read takes the object's next sequence
    // number, so the same card read twice moves Update_Time twice.
    for sequence in [1, 2] {
        read_card(&mut reader, AuthenticationFactorType::WIEGAND26).unwrap();
        assert_eq!(read(&reader, P::PRESENT_VALUE), card_value());
        assert_eq!(
            read(&reader, P::UPDATE_TIME),
            PropertyValue::ApplicationData(vec![0x19, sequence])
        );
    }
    // An undeclared format is refused and uses up no sequence number.
    assert_error(
        read_card(&mut reader, AuthenticationFactorType::WIEGAND37),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    let given = CredentialReadReport {
        update_time: Some(BACnetTimeStamp::SequenceNumber(40)),
        ..CredentialReadReport::new(card(AuthenticationFactorType::ERROR))
    };
    reader
        .report_access_input_internal(AccessControlInput::CredentialRead(given))
        .unwrap();
    assert_eq!(
        read(&reader, P::UPDATE_TIME),
        PropertyValue::ApplicationData(vec![0x19, 40])
    );
    read_card(&mut reader, AuthenticationFactorType::WIEGAND26).unwrap();
    assert_eq!(
        read(&reader, P::UPDATE_TIME),
        PropertyValue::ApplicationData(vec![0x19, 3])
    );

    // Out of service a read replaces the reader's values put aside (#1168):
    // the served ones stay, and the return to service serves the latest
    // read. Each read still has to name a declared format.
    set_out_of_service(&mut reader, true);
    let served = [P::PRESENT_VALUE, P::UPDATE_TIME].map(|p| read(&reader, p));
    let error_read = CredentialReadReport {
        update_time: Some(BACnetTimeStamp::SequenceNumber(60)),
        ..CredentialReadReport::new(card(AuthenticationFactorType::ERROR))
    };
    for later in [
        error_read.clone(),
        CredentialReadReport::new(card(AuthenticationFactorType::WIEGAND26)),
    ] {
        reader
            .report_access_input_internal(AccessControlInput::CredentialRead(later))
            .unwrap();
        assert_eq!(
            [P::PRESENT_VALUE, P::UPDATE_TIME].map(|p| read(&reader, p)),
            served
        );
    }
    assert_error(
        read_card(&mut reader, AuthenticationFactorType::WIEGAND37),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    set_out_of_service(&mut reader, false);
    // The last read set aside: the card, stamped with the next sequence
    // number.
    assert_eq!(read(&reader, P::PRESENT_VALUE), card_value());
    assert_eq!(
        read(&reader, P::UPDATE_TIME),
        PropertyValue::ApplicationData(vec![0x19, 4])
    );
    assert_error(
        reader.report_access_input_internal(AccessControlInput::AccessEvent(
            AccessEventReport::new(AccessEvent::GRANTED, 1),
        )),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

#[test]
fn access_door_takes_the_values_a_report_gives() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_alarm_values([DoorAlarmState::FORCED_OPEN])
        .unwrap();
    let state = |door: &AccessDoorObject| {
        [P::DOOR_STATUS, P::LOCK_STATUS, P::DOOR_ALARM_STATE].map(|p| read(door, p))
    };
    let raw = |door_status: DoorStatus, lock: LockStatus, alarm: DoorAlarmState| {
        [
            PropertyValue::Enumerated(door_status.to_raw()),
            PropertyValue::Enumerated(lock.to_raw()),
            PropertyValue::Enumerated(alarm.to_raw()),
        ]
    };
    let report = |door: &mut AccessDoorObject, report: DoorStateReport| {
        door.report_access_input_internal(AccessControlInput::DoorState(report))
    };
    // A field left out keeps its value.
    report(
        &mut door,
        DoorStateReport {
            door_status: Some(DoorStatus::OPENED),
            ..DoorStateReport::default()
        },
    )
    .unwrap();
    assert_eq!(
        state(&door),
        raw(
            DoorStatus::OPENED,
            LockStatus::LOCKED,
            DoorAlarmState::NORMAL
        )
    );
    report(
        &mut door,
        DoorStateReport {
            door_status: Some(DoorStatus::from_raw(1024)),
            lock_status: Some(LockStatus::UNLOCKED),
            door_alarm_state: Some(DoorAlarmState::FORCED_OPEN),
        },
    )
    .unwrap();
    let held = raw(
        DoorStatus::from_raw(1024),
        LockStatus::UNLOCKED,
        DoorAlarmState::FORCED_OPEN,
    );
    assert_eq!(state(&door), held);

    // One value refused refuses them all: a reserved Door_Status, a
    // Lock_Status past the closed production, an alarm state the lists
    // don't admit.
    for refused in [
        DoorStateReport {
            door_status: Some(DoorStatus::from_raw(1000)),
            lock_status: Some(LockStatus::LOCKED),
            ..DoorStateReport::default()
        },
        DoorStateReport {
            door_status: Some(DoorStatus::CLOSED),
            lock_status: Some(LockStatus::from_raw(5)),
            ..DoorStateReport::default()
        },
        DoorStateReport {
            door_status: Some(DoorStatus::CLOSED),
            door_alarm_state: Some(DoorAlarmState::TAMPER),
            ..DoorStateReport::default()
        },
    ] {
        assert_error(
            report(&mut door, refused),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(state(&door), held);
    }

    // Out of service a report replaces the device values put aside
    // (#1131): a client's simulated Door_Status stays served, the lists
    // still judge the alarm state, and the return to service serves the
    // latest values reported.
    set_out_of_service(&mut door, true);
    door.write_property(
        P::DOOR_STATUS,
        None,
        PropertyValue::Enumerated(DoorStatus::DOOR_FAULT.to_raw()),
        None,
    )
    .unwrap();
    let simulated = raw(
        DoorStatus::DOOR_FAULT,
        LockStatus::UNLOCKED,
        DoorAlarmState::FORCED_OPEN,
    );
    assert_eq!(state(&door), simulated);
    for later in [
        DoorStateReport {
            door_status: Some(DoorStatus::OPENED),
            ..DoorStateReport::default()
        },
        DoorStateReport {
            door_status: Some(DoorStatus::CLOSED),
            lock_status: Some(LockStatus::LOCKED),
            door_alarm_state: Some(DoorAlarmState::NORMAL),
        },
    ] {
        report(&mut door, later).unwrap();
        assert_eq!(state(&door), simulated);
    }
    let tamper = DoorStateReport {
        door_alarm_state: Some(DoorAlarmState::TAMPER),
        ..DoorStateReport::default()
    };
    assert_error(
        report(&mut door, tamper),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    set_out_of_service(&mut door, false);
    assert_eq!(
        state(&door),
        raw(
            DoorStatus::CLOSED,
            LockStatus::LOCKED,
            DoorAlarmState::NORMAL
        )
    );
}

#[test]
fn other_objects_fail_closed() {
    let mut value = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    assert_error(
        value.report_access_input_internal(AccessControlInput::DoorState(
            DoorStateReport::default(),
        )),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    assert_error(
        door.report_access_input_internal(AccessControlInput::AccessEvent(AccessEventReport::new(
            AccessEvent::GRANTED,
            1,
        ))),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

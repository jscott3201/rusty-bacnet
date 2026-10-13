//! The application's routes to the access-control inputs (#1132):
//! `report_access_event_local`, `report_credential_read_local` and
//! `report_door_state_local`. Each takes its record as one local write, so
//! the Table 13-1 report and the event pass follow it, and stamps a missing
//! time from the Device clock. While Out_Of_Service is TRUE the point refuses
//! an event, and the reader and the door keep a report aside, serving and
//! reporting nothing new until the return to service.

use super::*;
use bacnet_types::enums::{DoorStatus, EventState, LockStatus};

fn ap(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ACCESS_POINT, instance).unwrap()
}

fn cdi() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, 1).unwrap()
}

fn door_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 1).unwrap()
}

fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class: c, code: e })
            if c == class.to_raw() as u32 && e == code.to_raw() as u32),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

/// What `oid` serves for `property`, encoded.
async fn served(h: &Harness, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
    let db = h.server.database().read().await;
    encode(db.get(&oid).unwrap().read_property(property, None).unwrap())
}

#[tokio::test(start_paused = true)]
async fn access_event_route_stamps_the_clock_and_refuses_out_of_service() {
    const EVENT: PropertyIdentifier = PropertyIdentifier::ACCESS_EVENT;
    let oid = ap(1);
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(point(AccessEvent::GRANTED, 1, 7)).unwrap();
    })
    .await;
    subscribed(&mut h, oid).await;

    // An event with no time takes the Device clock's, and with no credential
    // or factor the no-credential reference and the UNDEFINED factor.
    h.set_clock(20);
    let unknown = AccessEventReport::new(AccessEvent::DENIED_UNKNOWN_CREDENTIAL, 2);
    h.server
        .report_access_event_local(&oid, unknown)
        .await
        .unwrap();
    let report = |event: AccessEvent, flags: u8, tag: u64, second: u8| {
        vec![
            (EVENT, enumerated(event.to_raw())),
            (SF, vec![0x82, 0x04, flags]),
            (PropertyIdentifier::ACCESS_EVENT_TAG, unsigned(tag)),
            (PropertyIdentifier::ACCESS_EVENT_TIME, stamp_bytes(second)),
            (
                PropertyIdentifier::ACCESS_EVENT_CREDENTIAL,
                no_credential_bytes(),
            ),
            (
                PropertyIdentifier::ACCESS_EVENT_AUTHENTICATION_FACTOR,
                undefined_factor_bytes(),
            ),
        ]
    };
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(AccessEvent::DENIED_UNKNOWN_CREDENTIAL, 0x00, 2, 20)
    );

    // A credential reference to another object type is refused whole.
    h.set_clock(25);
    let not_a_credential = AccessEventReport {
        credential: Some(ap(2).into()),
        ..AccessEventReport::new(AccessEvent::GRANTED, 3)
    };
    assert_refused(
        h.server
            .report_access_event_local(&oid, not_a_credential)
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    h.no_notification().await;

    // Out of service the point performs no authentication, so an event is
    // refused and changes nothing (Clause 12.31.8); the edge's own event
    // stays served.
    write(
        &mut h,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(AccessEvent::OUT_OF_SERVICE, 0x10, 3, 25)
    );
    h.set_clock(30);
    assert_refused(
        h.server
            .report_access_event_local(&oid, access_event(AccessEvent::GRANTED, 4, 30))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    h.no_notification().await;
    assert_eq!(
        served(&h, oid, EVENT).await,
        enumerated(AccessEvent::OUT_OF_SERVICE.to_raw())
    );

    // Back in service the route takes events again.
    write(
        &mut h,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    h.cov_notification().await;
    h.set_clock(35);
    h.server
        .report_access_event_local(&oid, AccessEventReport::new(AccessEvent::GRANTED, 5))
        .await
        .unwrap();
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(AccessEvent::GRANTED, 0x00, 5, 35)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn credential_read_route_stamps_the_clock_and_sets_aside_out_of_service() {
    let oid = cdi();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(reader(7)).unwrap();
    })
    .await;
    let report = |value: Vec<u8>, flags: u8, second: u8| {
        vec![
            (PV, value),
            (SF, vec![0x82, 0x04, flags]),
            (PropertyIdentifier::UPDATE_TIME, stamp_bytes(second)),
        ]
    };
    assert_eq!(subscribed(&mut h, oid).await, report(card_bytes(), 0x00, 7));

    // A read with no time takes the Device clock's, so the same card read
    // again still moves Update_Time and reports.
    for second in [20, 25] {
        h.set_clock(second);
        h.server
            .report_credential_read_local(&oid, CredentialReadReport::new(card()))
            .await
            .unwrap();
        assert_eq!(
            values(&h.cov_notification().await, oid),
            report(card_bytes(), 0x00, second)
        );
    }

    // A format the reader doesn't declare is refused (Clause 12.36.4).
    let wiegand37 = BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::WIEGAND37,
        ..card()
    };
    assert_refused(
        h.server
            .report_credential_read_local(&oid, CredentialReadReport::new(wiegand37))
            .await,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    h.no_notification().await;

    // Out of service a client may be simulating the reader, so a read
    // replaces the reader's values put aside (#1168): the served ones stay
    // and nothing reports. The return to service serves the latest read.
    write(
        &mut h,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    h.cov_notification().await;
    let other_card = BACnetAuthenticationFactor {
        value: vec![0x65, 0x43, 0x21],
        ..card()
    };
    for second in [30, 35] {
        h.set_clock(second);
        h.server
            .report_credential_read_local(&oid, CredentialReadReport::new(other_card.clone()))
            .await
            .unwrap();
        h.no_notification().await;
        assert_eq!(served(&h, oid, PV).await, card_bytes());
    }
    write(
        &mut h,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    let other_card_bytes = vec![0x09, 0x08, 0x19, 0x00, 0x2B, 0x65, 0x43, 0x21];
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(other_card_bytes, 0x00, 35)
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn door_state_route_runs_the_event_pass_and_sets_aside_out_of_service() {
    const ALARM: PropertyIdentifier = PropertyIdentifier::DOOR_ALARM_STATE;
    let oid = door_oid();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        // Event detection on: FORCED_OPEN is an alarm value with no delay.
        let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
        door.set_alarm_values([DoorAlarmState::FORCED_OPEN])
            .unwrap();
        db.add(Box::new(door)).unwrap();
    })
    .await;
    let report = |alarm: DoorAlarmState, flags: u8| {
        vec![
            (PV, enumerated(DoorValue::LOCK.to_raw())),
            (SF, vec![0x82, 0x04, flags]),
            (ALARM, enumerated(alarm.to_raw())),
        ]
    };
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(DoorAlarmState::NORMAL, 0x00)
    );

    // The event pass after the report puts the door in alarm at once, with
    // no tick, and the one report carries IN_ALARM.
    let forced = DoorStateReport {
        door_status: Some(DoorStatus::OPENED),
        lock_status: Some(LockStatus::LOCKED),
        door_alarm_state: Some(DoorAlarmState::FORCED_OPEN),
    };
    h.server
        .report_door_state_local(&oid, forced)
        .await
        .unwrap();
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(DoorAlarmState::FORCED_OPEN, 0x80)
    );
    h.no_notification().await;
    assert_eq!(
        served(&h, oid, PropertyIdentifier::EVENT_STATE).await,
        enumerated(EventState::OFFNORMAL.to_raw())
    );
    assert_eq!(
        served(&h, oid, PropertyIdentifier::DOOR_STATUS).await,
        enumerated(DoorStatus::OPENED.to_raw())
    );

    // A report is taken whole or not at all: a reserved Door_Status, or an
    // alarm state the lists don't admit, refuses the other values too.
    for refused in [
        DoorStateReport {
            door_status: Some(DoorStatus::from_raw(1000)),
            door_alarm_state: Some(DoorAlarmState::NORMAL),
            ..DoorStateReport::default()
        },
        DoorStateReport {
            door_status: Some(DoorStatus::CLOSED),
            door_alarm_state: Some(DoorAlarmState::TAMPER),
            ..DoorStateReport::default()
        },
    ] {
        assert_refused(
            h.server.report_door_state_local(&oid, refused).await,
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    h.no_notification().await;
    assert_eq!(
        served(&h, oid, PropertyIdentifier::DOOR_STATUS).await,
        enumerated(DoorStatus::OPENED.to_raw())
    );

    // Out of service a client may be simulating the door, so a report
    // replaces the device values put aside (#1131): the served values stay,
    // with no report and no event. The return to service serves the latest
    // values reported.
    write(
        &mut h,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    h.cov_notification().await;
    for later in [
        DoorStateReport {
            lock_status: Some(LockStatus::UNLOCKED),
            ..DoorStateReport::default()
        },
        DoorStateReport {
            door_status: Some(DoorStatus::CLOSED),
            door_alarm_state: Some(DoorAlarmState::NORMAL),
            ..DoorStateReport::default()
        },
    ] {
        h.server.report_door_state_local(&oid, later).await.unwrap();
        h.no_notification().await;
        assert_eq!(
            served(&h, oid, ALARM).await,
            enumerated(DoorAlarmState::FORCED_OPEN.to_raw())
        );
        assert_eq!(
            served(&h, oid, PropertyIdentifier::DOOR_STATUS).await,
            enumerated(DoorStatus::OPENED.to_raw())
        );
        assert_eq!(
            served(&h, oid, PropertyIdentifier::EVENT_STATE).await,
            enumerated(EventState::OFFNORMAL.to_raw())
        );
    }
    write(
        &mut h,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    let back = values(&h.cov_notification().await, oid);
    assert_eq!(
        back.last(),
        Some(&(ALARM, enumerated(DoorAlarmState::NORMAL.to_raw())))
    );
    for (property, value) in [
        (PropertyIdentifier::DOOR_STATUS, DoorStatus::CLOSED.to_raw()),
        (
            PropertyIdentifier::LOCK_STATUS,
            LockStatus::UNLOCKED.to_raw(),
        ),
    ] {
        assert_eq!(served(&h, oid, property).await, enumerated(value));
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn events_of_one_transaction_each_send_a_report() {
    let oid = ap(1);
    // One transaction, two events (Clause 12.31.27.1): the factor read, then
    // the grant, each stamped by the point.
    let read_then_grant = [
        AccessEventReport::new(AccessEvent::AUTHENTICATION_FACTOR_READ, 2),
        AccessEventReport::new(AccessEvent::GRANTED, 2),
    ];
    let time_of = |report: &Values| {
        report
            .iter()
            .find(|(property, _)| *property == PropertyIdentifier::ACCESS_EVENT_TIME)
            .map(|(_, value)| value.clone())
            .unwrap()
    };
    for clockless in [true, false] {
        let mut h = Harness::start_with(ServerConfig::default(), |db| {
            db.add(point(AccessEvent::GRANTED, 1, 7)).unwrap();
        })
        .await;
        if clockless {
            h.server.database().write().await.set_clock_reader(None);
        } else {
            h.set_clock(20);
        }
        subscribed(&mut h, oid).await;
        let mut times = Vec::new();
        for report in read_then_grant.clone() {
            let event = report.event;
            h.server
                .report_access_event_local(&oid, report)
                .await
                .unwrap();
            let values = values(&h.cov_notification().await, oid);
            assert_eq!(
                values[0],
                (PropertyIdentifier::ACCESS_EVENT, enumerated(event.to_raw()))
            );
            times.push(time_of(&values));
        }
        h.no_notification().await;
        // Without a clock the sequence numbers count from 1; with one, the
        // second event in the same hundredth is stamped one hundredth on.
        let expected = if clockless {
            vec![vec![0x19, 1], vec![0x19, 2]]
        } else {
            let mut next = stamp_bytes(20);
            next[10] = 1;
            vec![stamp_bytes(20), next]
        };
        assert_eq!(times, expected, "clockless: {clockless}");
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn routes_refuse_other_objects() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(point(AccessEvent::GRANTED, 1, 7)).unwrap();
        db.add(reader(7)).unwrap();
        db.add(Box::new(AccessDoorObject::new(1, "DOOR-1").unwrap()))
            .unwrap();
    })
    .await;
    let analog_value = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let event = || AccessEventReport::new(AccessEvent::GRANTED, 2);
    let read = || CredentialReadReport::new(card());
    for result in [
        h.server
            .report_access_event_local(&door_oid(), event())
            .await,
        h.server.report_access_event_local(&cdi(), event()).await,
        h.server.report_credential_read_local(&ap(1), read()).await,
        h.server
            .report_credential_read_local(&analog_value, read())
            .await,
        h.server
            .report_door_state_local(&ap(1), DoorStateReport::default())
            .await,
    ] {
        assert_refused(
            result,
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        );
    }
    assert_refused(
        h.server.report_access_event_local(&ap(9), event()).await,
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

//! The Access Door row of Table 13-1 (#1061): Present_Value, Status_Flags
//! and Door_Alarm_State, with Door_Alarm_State the trigger, reported on a
//! device change (`report_door_state_local`, #1132) and on a client's
//! simulated value while the door is out of service (#1131).

use super::*;

/// A door in `alarm`, whose Alarm_Values admit every alarm state these tests
/// use. Event detection is off, so only the COV row moves: an event
/// transition would set IN_ALARM in Status_Flags (#1149).
fn door(alarm: DoorAlarmState) -> Box<dyn BACnetObject> {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_alarm_values([
        DoorAlarmState::DOOR_OPEN_TOO_LONG,
        DoorAlarmState::FORCED_OPEN,
        DoorAlarmState::TAMPER,
    ])
    .unwrap();
    door.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    door.set_door_alarm_state(alarm).unwrap();
    Box::new(door)
}

#[tokio::test(start_paused = true)]
async fn access_door_cov_reports_and_triggers_on_door_alarm_state() {
    let oid = ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(door(DoorAlarmState::NORMAL)).unwrap();
    })
    .await;
    let report = |alarm: DoorAlarmState| {
        vec![
            (PV, enumerated(DoorValue::LOCK.to_raw())),
            (SF, normal()),
            (
                PropertyIdentifier::DOOR_ALARM_STATE,
                enumerated(alarm.to_raw()),
            ),
        ]
    };
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(DoorAlarmState::NORMAL)
    );

    // A Door_Alarm_State change alone sends a report.
    let alarm = |state| DoorStateReport {
        door_alarm_state: Some(state),
        ..DoorStateReport::default()
    };
    h.server
        .report_door_state_local(&oid, alarm(DoorAlarmState::FORCED_OPEN))
        .await
        .unwrap();
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(DoorAlarmState::FORCED_OPEN)
    );
    h.no_notification().await;

    // Row values unchanged: neither the same state again, nor Door_Status
    // and Lock_Status, which aren't in the row, nor a pulse-time write
    // reports.
    h.server
        .report_door_state_local(&oid, alarm(DoorAlarmState::FORCED_OPEN))
        .await
        .unwrap();
    h.server
        .report_door_state_local(
            &oid,
            DoorStateReport {
                door_status: Some(DoorStatus::OPENED),
                lock_status: Some(LockStatus::UNLOCKED),
                door_alarm_state: None,
            },
        )
        .await
        .unwrap();
    write(
        &mut h,
        oid,
        PropertyIdentifier::DOOR_PULSE_TIME,
        unsigned(20),
    )
    .await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn access_door_simulated_door_alarm_state_reports_and_restores() {
    const ALARM: PropertyIdentifier = PropertyIdentifier::DOOR_ALARM_STATE;
    const OUT_OF_SERVICE: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;
    let oid = ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(door(DoorAlarmState::DOOR_OPEN_TOO_LONG)).unwrap();
    })
    .await;
    let report = |alarm: DoorAlarmState, flags: Vec<u8>| {
        vec![
            (PV, enumerated(DoorValue::LOCK.to_raw())),
            (SF, flags),
            (ALARM, enumerated(alarm.to_raw())),
        ]
    };
    // Status_Flags with only OUT_OF_SERVICE set.
    let out_of_service = || vec![0x82, 0x04, 0x10];
    assert_eq!(
        subscribed(&mut h, oid).await,
        report(DoorAlarmState::DOOR_OPEN_TOO_LONG, normal())
    );

    // In service the write is refused and nothing reports.
    assert_eq!(
        try_write(
            &mut h,
            oid,
            ALARM,
            enumerated(DoorAlarmState::FORCED_OPEN.to_raw())
        )
        .await,
        Err(ErrorCode::WRITE_ACCESS_DENIED)
    );
    h.no_notification().await;

    // Out of service the OUT_OF_SERVICE flag reports, then a simulated
    // Door_Alarm_State reports on its own.
    write(
        &mut h,
        oid,
        OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(true)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(DoorAlarmState::DOOR_OPEN_TOO_LONG, out_of_service())
    );
    write(
        &mut h,
        oid,
        ALARM,
        enumerated(DoorAlarmState::FORCED_OPEN.to_raw()),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(DoorAlarmState::FORCED_OPEN, out_of_service())
    );
    h.no_notification().await;

    // Door_Status and Lock_Status aren't in the row, so simulating them
    // alone reports nothing.
    write(
        &mut h,
        oid,
        PropertyIdentifier::DOOR_STATUS,
        enumerated(DoorStatus::OPENED.to_raw()),
    )
    .await;
    write(
        &mut h,
        oid,
        PropertyIdentifier::LOCK_STATUS,
        enumerated(LockStatus::UNLOCKED.to_raw()),
    )
    .await;
    h.no_notification().await;

    // One WritePropertyMultiple that simulates two rows reports once.
    write_multiple(
        &mut h,
        oid,
        vec![
            (
                PropertyIdentifier::DOOR_STATUS,
                enumerated(DoorStatus::CLOSED.to_raw()),
            ),
            (ALARM, enumerated(DoorAlarmState::TAMPER.to_raw())),
        ],
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(DoorAlarmState::TAMPER, out_of_service())
    );
    h.no_notification().await;

    // The return to service brings back the device's Door_Alarm_State and
    // clears the flag, in one report.
    write(
        &mut h,
        oid,
        OUT_OF_SERVICE,
        encode(PropertyValue::Boolean(false)),
    )
    .await;
    assert_eq!(
        values(&h.cov_notification().await, oid),
        report(DoorAlarmState::DOOR_OPEN_TOO_LONG, normal())
    );
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

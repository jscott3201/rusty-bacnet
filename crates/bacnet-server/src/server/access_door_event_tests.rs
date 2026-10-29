//! Access Door intrinsic reporting through the running server (#1149): a
//! CHANGE_OF_STATE on Door_Alarm_State reaches the recipients of the door's
//! Notification Class going into alarm and coming out of it, whether the
//! application reports the state or a client simulates it out of service;
//! masking the state returns the door to NORMAL; and a Fault_Values member
//! reports a fault listing Door_Alarm_State then Present_Value, the door's
//! Table 13-5 properties.

use super::event_notifications_tests::{
    decode_broadcast_notification, local_broadcast_destination, recording_transport,
};
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use PropertyIdentifier as P;
use bacnet_encoding::constructed::decode_bacnet_property_value;
use bacnet_objects::access_control::AccessDoorObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::NotificationParameters;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetPropertyStates;
use bacnet_types::enums::{DoorAlarmState, EventState, EventType, NotifyType, Reliability};
use bacnet_types::primitives::StatusFlags;
use bytes::Bytes;

/// The door's Notification Class; class 0 doesn't exist here, so a
/// notification can reach the wire only by following the door's own one.
const CLASS: u32 = 5;

fn alarm_states(states: &[DoorAlarmState]) -> PropertyValue {
    PropertyValue::List(
        states
            .iter()
            .map(|state| PropertyValue::Enumerated(state.to_raw()))
            .collect(),
    )
}

/// A door that alarms on DOOR_OPEN_TOO_LONG and FORCED_OPEN after
/// `time_delay` seconds, faults on DOOR_FAULT and distributes every
/// transition, configured as a client would configure it.
fn door(time_delay: u64) -> AccessDoorObject {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    for (property, value) in [
        (
            P::ALARM_VALUES,
            alarm_states(&[
                DoorAlarmState::DOOR_OPEN_TOO_LONG,
                DoorAlarmState::FORCED_OPEN,
            ]),
        ),
        (P::FAULT_VALUES, alarm_states(&[DoorAlarmState::DOOR_FAULT])),
        (P::TIME_DELAY, PropertyValue::Unsigned(time_delay)),
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(CLASS.into())),
        (
            P::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::all().to_bacnet()],
            },
        ),
    ] {
        door.write_property(property, None, value, None).unwrap();
    }
    door
}

async fn start(door: AccessDoorObject) -> (BACnetServer<TestTransport>, ObjectIdentifier, SendLog) {
    let (transport, sent) = recording_transport();
    let oid = door.object_identifier();
    let mut class = NotificationClass::new(CLASS, "NC-5").unwrap();
    class
        .add_destination(local_broadcast_destination())
        .unwrap();
    let mut db = clocked_test_database();
    db.add(Box::new(door)).unwrap();
    db.add(Box::new(class)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    (server, oid, sent)
}

/// A local write, run through the same post-write event path as a client's.
async fn write(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: P,
    value: PropertyValue,
) {
    server
        .write_local(
            &oid,
            property,
            None,
            value,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

fn take(sent: &SendLog) -> Vec<Bytes> {
    sent.take().into_iter().map(|frame| frame.npdu).collect()
}

async fn read(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: P,
) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

fn change_of_state(state: DoorAlarmState, flags: StatusFlags) -> NotificationParameters {
    NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::DoorAlarmState(state.to_raw()),
        status_flags: flags,
    }
}

#[tokio::test(start_paused = true)]
async fn access_door_open_too_long_reaches_recipients_until_masked() {
    // The application's door logic has found the door held open too long.
    let mut reported = door(2);
    reported
        .set_door_alarm_state(DoorAlarmState::DOOR_OPEN_TOO_LONG)
        .unwrap();
    let (server, oid, sent) = start(reported).await;
    assert!(sent.is_empty(), "Time_Delay holds the transition back");

    tokio::time::sleep(Duration::from_secs(5)).await;
    let offnormal = decode_broadcast_notification(&take(&sent));
    assert_eq!(offnormal.event_object_identifier, oid);
    assert_eq!(offnormal.notification_class, CLASS);
    assert_eq!(offnormal.event_type, EventType::CHANGE_OF_STATE);
    assert_eq!(offnormal.notify_type, NotifyType::ALARM);
    assert_eq!(
        (offnormal.from_state, offnormal.to_state),
        (EventState::NORMAL, EventState::OFFNORMAL)
    );
    assert_eq!(
        offnormal.event_values,
        Some(change_of_state(
            DoorAlarmState::DOOR_OPEN_TOO_LONG,
            StatusFlags::IN_ALARM,
        ))
    );

    // An operator masks the alarm: the door is NORMAL at once, and the
    // return to normal follows Time_Delay.
    write(
        &server,
        oid,
        P::MASKED_ALARM_VALUES,
        alarm_states(&[DoorAlarmState::DOOR_OPEN_TOO_LONG]),
    )
    .await;
    assert_eq!(
        read(&server, oid, P::DOOR_ALARM_STATE).await,
        PropertyValue::Enumerated(DoorAlarmState::NORMAL.to_raw())
    );
    assert!(sent.is_empty());
    tokio::time::sleep(Duration::from_secs(5)).await;
    let normal = decode_broadcast_notification(&take(&sent));
    assert_eq!(
        (normal.from_state, normal.to_state),
        (EventState::OFFNORMAL, EventState::NORMAL)
    );
    assert_eq!(
        normal.event_values,
        Some(change_of_state(
            DoorAlarmState::NORMAL,
            StatusFlags::empty()
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn access_door_simulated_alarm_goes_into_and_out_of_alarm() {
    let (server, oid, sent) = start(door(0)).await;
    write(
        &server,
        oid,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert!(sent.is_empty());

    write(
        &server,
        oid,
        P::DOOR_ALARM_STATE,
        PropertyValue::Enumerated(DoorAlarmState::FORCED_OPEN.to_raw()),
    )
    .await;
    let offnormal = decode_broadcast_notification(&take(&sent));
    assert_eq!(
        (offnormal.from_state, offnormal.to_state),
        (EventState::NORMAL, EventState::OFFNORMAL)
    );
    assert_eq!(
        offnormal.event_values,
        Some(change_of_state(
            DoorAlarmState::FORCED_OPEN,
            StatusFlags::IN_ALARM | StatusFlags::OUT_OF_SERVICE,
        ))
    );

    write(
        &server,
        oid,
        P::DOOR_ALARM_STATE,
        PropertyValue::Enumerated(DoorAlarmState::NORMAL.to_raw()),
    )
    .await;
    let normal = decode_broadcast_notification(&take(&sent));
    assert_eq!(
        (normal.from_state, normal.to_state),
        (EventState::OFFNORMAL, EventState::NORMAL)
    );
    assert_eq!(
        normal.event_values,
        Some(change_of_state(
            DoorAlarmState::NORMAL,
            StatusFlags::OUT_OF_SERVICE,
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn access_door_fault_value_reports_door_alarm_state_and_present_value() {
    let (server, oid, sent) = start(door(0)).await;
    write(
        &server,
        oid,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    write(
        &server,
        oid,
        P::DOOR_ALARM_STATE,
        PropertyValue::Enumerated(DoorAlarmState::DOOR_FAULT.to_raw()),
    )
    .await;
    let fault = decode_broadcast_notification(&take(&sent));
    assert_eq!(fault.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(
        (fault.from_state, fault.to_state),
        (EventState::NORMAL, EventState::FAULT)
    );
    let Some(NotificationParameters::ChangeOfReliability {
        reliability,
        status_flags,
        property_values,
    }) = fault.event_values
    else {
        panic!(
            "expected CHANGE_OF_RELIABILITY values, got {:?}",
            fault.event_values
        );
    };
    assert_eq!(reliability, Reliability::MULTI_STATE_FAULT);
    assert_eq!(
        status_flags,
        StatusFlags::IN_ALARM | StatusFlags::FAULT | StatusFlags::OUT_OF_SERVICE
    );
    // Door_Alarm_State DOOR_FAULT, then Present_Value LOCK.
    let (state, next) = decode_bacnet_property_value(&property_values, 0).unwrap();
    assert_eq!(state.property_identifier, P::DOOR_ALARM_STATE);
    assert_eq!(state.value, [0x91, 0x05]);
    let (command, end) = decode_bacnet_property_value(&property_values, next).unwrap();
    assert_eq!(end, property_values.len());
    assert_eq!(command.property_identifier, P::PRESENT_VALUE);
    assert_eq!(command.value, [0x91, 0x00]);

    // Out of the fault values, the fault clears.
    write(
        &server,
        oid,
        P::DOOR_ALARM_STATE,
        PropertyValue::Enumerated(DoorAlarmState::NORMAL.to_raw()),
    )
    .await;
    let recovered = decode_broadcast_notification(&take(&sent));
    assert_eq!(
        (recovered.from_state, recovered.to_state),
        (EventState::FAULT, EventState::NORMAL)
    );
    assert_eq!(
        read(&server, oid, P::RELIABILITY).await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}

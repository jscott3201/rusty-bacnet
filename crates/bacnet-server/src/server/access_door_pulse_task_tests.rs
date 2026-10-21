//! Access Door pulse relock through the running server (#1073): the
//! monotonic operation task relinquishes a PULSE_UNLOCK or
//! EXTENDED_PULSE_UNLOCK slot at its deadline and fans out one COV.

use super::cov_notifications_tests::recording_transport;
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_objects::access_control::AccessDoorObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

async fn start_server() -> (BACnetServer<TestTransport>, ObjectIdentifier, SendLog) {
    let (transport, sent) = recording_transport();
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    // 2 s and 4 s, in tenths of a second.
    door.set_door_pulse_time(20);
    door.set_door_extended_pulse_time(40);
    let oid = door.object_identifier();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(100, 0)
            .expect("valid Device identity")
            .with_name("Access-door-pulse-device"),
        ..DeviceConfig::default()
    })
    .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(door)).unwrap();
    let config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::new(100, 0).unwrap(),
        enable_event_enrollment: false,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    settle().await;
    (server, oid, sent)
}

async fn command(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    value: Option<u32>,
    priority: u8,
) {
    server
        .write_local(
            &oid,
            PV,
            None,
            value.map_or(PropertyValue::Null, PropertyValue::Enumerated),
            Some(priority),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

async fn read(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

async fn subscribe(server: &BACnetServer<TestTransport>, oid: ObjectIdentifier) {
    server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 0xC2]),
            subscriber_network: None,
            subscriber_process_identifier: 9,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn door_pulse_relocks_at_its_exact_deadline_with_one_cov() {
    let (mut server, oid, sent) = start_server().await;
    subscribe(&server, oid).await;
    tokio::time::advance(Duration::from_millis(500)).await;
    settle().await;
    command(&server, oid, Some(2), 8).await;
    assert_eq!(sent.len(), 1, "accepted-write COV");
    sent.clear();
    assert_eq!(
        read(
            &server,
            oid,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            None
        )
        .await,
        PropertyValue::Unsigned(8)
    );

    tokio::time::advance(Duration::from_millis(1_999)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PV, None).await,
        PropertyValue::Enumerated(2)
    );
    assert_eq!(sent.len(), 0, "no early relock");

    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PV, None).await,
        PropertyValue::Enumerated(0)
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        PropertyValue::Null
    );
    assert_eq!(
        read(
            &server,
            oid,
            PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            None
        )
        .await,
        PropertyValue::Null
    );
    assert_eq!(sent.len(), 1, "the relock fans out one COV");

    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(sent.len(), 1, "one COV per relock");
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn door_extended_pulse_relocks_after_its_own_time_and_a_relinquish_disarms() {
    let (mut server, oid, _) = start_server().await;
    command(&server, oid, Some(3), 8).await;
    tokio::time::advance(Duration::from_millis(3_999)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PV, None).await,
        PropertyValue::Enumerated(3)
    );
    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PV, None).await,
        PropertyValue::Enumerated(0)
    );

    // A pulse relinquished early leaves nothing armed: a later steady
    // command at the slot stays.
    command(&server, oid, Some(2), 8).await;
    command(&server, oid, None, 8).await;
    command(&server, oid, Some(1), 8).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(
        read(&server, oid, PV, None).await,
        PropertyValue::Enumerated(1)
    );
    server.stop().await.unwrap();
}

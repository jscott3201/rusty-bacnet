//! Subscribed_Recipients entries lapse in a running server (#1049): the
//! monotonic operation task, which also relocks Access Door pulses, drops each
//! entry at its deadline, so the object stops holding it, not only stops
//! serving it.

use super::cov_notifications_tests::recording_transport;
use super::*;
use crate::server::test_forwarder::TestForwarder;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::constructed::encode_event_notification_subscription_list;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_types::constructed::{BACnetEventNotificationSubscription, BACnetRecipient};

const MINUTE: Duration = Duration::from_secs(60);
const SUBSCRIBED_RECIPIENTS: PropertyIdentifier = PropertyIdentifier::SUBSCRIBED_RECIPIENTS;

fn device(instance: u32, minutes: u32) -> BACnetEventNotificationSubscription {
    BACnetEventNotificationSubscription {
        recipient: BACnetRecipient::Device(
            ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        ),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        time_remaining: minutes,
    }
}

fn framed(subscriptions: &[BACnetEventNotificationSubscription]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    buf.to_vec()
}

async fn start_server() -> (BACnetServer<TestTransport>, ObjectIdentifier) {
    let (transport, _sent) = recording_transport();
    let forwarder = TestForwarder::new(1);
    let oid = forwarder.object_identifier();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(100, 0)
            .expect("valid Device identity")
            .with_name("Forwarder-device"),
        ..DeviceConfig::default()
    })
    .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(forwarder)).unwrap();
    let config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::new(100, 0).unwrap(),
        enable_event_enrollment: false,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    settle().await;
    (server, oid)
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// The object's next deadline and the list it serves.
async fn state(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
) -> (Option<Duration>, PropertyValue) {
    let db = server.database().read().await;
    let object = db.get(&oid).unwrap();
    (
        object.next_monotonic_deadline_internal(),
        object.read_property(SUBSCRIBED_RECIPIENTS, None).unwrap(),
    )
}

#[tokio::test(start_paused = true)]
async fn the_operation_task_drops_each_subscription_at_its_deadline() {
    let (mut server, oid) = start_server().await;
    let mut request = BytesMut::new();
    ListElementRequest {
        object_identifier: oid,
        property_identifier: SUBSCRIBED_RECIPIENTS,
        property_array_index: None,
        list_of_elements: framed(&[device(1, 1), device(2, 2)]),
    }
    .encode(&mut request)
    .unwrap();
    let added_at = {
        let mut db = server.database().write().await;
        crate::handlers::handle_add_list_element(&mut db, &request).unwrap();
        db.get(&oid)
            .unwrap()
            .next_monotonic_deadline_internal()
            .unwrap()
            - MINUTE
    };

    tokio::time::advance(MINUTE - Duration::from_millis(1)).await;
    settle().await;
    let (deadline, served) = state(&server, oid).await;
    assert_eq!(deadline, Some(added_at + MINUTE), "nothing lapsed early");
    assert_eq!(
        served,
        PropertyValue::ApplicationData(framed(&[device(1, 1), device(2, 2)]))
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    let (deadline, served) = state(&server, oid).await;
    assert_eq!(deadline, Some(added_at + 2 * MINUTE), "the first is gone");
    assert_eq!(
        served,
        PropertyValue::ApplicationData(framed(&[device(2, 1)]))
    );

    tokio::time::advance(MINUTE).await;
    settle().await;
    assert_eq!(
        state(&server, oid).await,
        (None, PropertyValue::ApplicationData(Vec::new()))
    );
    server.stop().await.unwrap();
}

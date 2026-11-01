//! Event notifications name the identity's Device as their initiator, and a
//! forwarder hands a copy addressed to that Device back to this device, even
//! once the application adds a Device of a lower instance after start
//! (#2217). The COV families are covered in
//! `active_cov_subscriptions_tests::initiating_device`; target Audit refuses
//! a new Device while it runs (`audit_reporter_identity_tests`).
use super::cov_wire_test_support::Harness;
use super::event_forwarding::Reception;
use super::event_forwarding_tests::{
    Forwarding, LOCAL_DEVICE, PEER_A, To, database, destination, notification, unconfirmed,
};
use super::event_log_notification_tests::{alarm, sent_notification, to_peer};
use super::event_recipient_routing_tests::address_recipient;
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::EventState;

fn device(instance: u32) -> Box<DeviceObject> {
    Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(instance, 0).unwrap(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    )
}

fn device_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_local_event_names_the_identity_device_beside_a_lower_one() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| alarm(db, to_peer())).await;
    // The harness's identity names Device 856.
    let identity = device_oid(856);
    h.server.database().write().await.add(device(5)).unwrap();
    assert_eq!(
        h.server.database().read().await.selected_device(),
        Some(device_oid(5))
    );
    h.set_clock(40);
    h.write_local(90.0).await;
    let alarm = sent_notification(&h).await;
    assert_eq!(alarm.to_state, EventState::HIGH_LIMIT);
    assert_eq!(alarm.initiating_device_identifier, identity);
    h.server.stop().await.unwrap();
}

/// The chain of `forwarders_chain_within_the_device_and_each_takes_a_notification_once`
/// beside Device 0, below the identity's Device 1: forwarder A's copy to
/// Device 1 still comes back to this device for forwarder B to send on.
#[tokio::test]
async fn a_forwarded_copy_to_the_identity_device_stays_local_beside_a_lower_one() {
    let local = BACnetRecipient::Device(device_oid(LOCAL_DEVICE));
    let mut a = NotificationForwarderObject::new(1, "A").unwrap();
    a.set_process_identifier_filter(Some(1));
    a.add_destination(destination(local, 2, false)).unwrap();
    let mut b = NotificationForwarderObject::new(2, "B").unwrap();
    b.set_process_identifier_filter(Some(2));
    b.add_destination(destination(address_recipient(0, &PEER_A), 3, false))
        .unwrap();
    let mut db = database(vec![a, b]);
    db.add(device(0)).unwrap();
    assert_eq!(db.selected_device(), Some(device_oid(0)));
    let forwarding = Forwarding::new(db);
    assert_eq!(
        forwarding
            .receive(&notification(1), Reception::UNICAST)
            .await,
        [unconfirmed(To::Local(PEER_A.to_vec()), 3)]
    );
}

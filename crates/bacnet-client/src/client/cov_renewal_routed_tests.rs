//! Managed COV subscriptions to a routed device (#1790): requests carry the
//! device's network and address, and only notifications from that address
//! move the renewal.

use super::*;
use bacnet_encoding::apdu::{SimpleAck, UnconfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

const CLIENT: [u8; 1] = [0x01];
const ROUTER: [u8; 1] = [0x02];
const NETWORK: u16 = 7;
const STATION: [u8; 1] = [0x05];

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn notification(
    process: u32,
    object: ObjectIdentifier,
    time_remaining: u32,
) -> COVNotificationRequest {
    COVNotificationRequest {
        subscriber_process_identifier: process,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        monitored_object_identifier: object,
        time_remaining,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
            priority: None,
        }],
    }
}

/// `received` as the client's notification channel would carry it, from
/// `mac` with `routed` as its source network and address.
fn received(mac: &[u8], routed: Option<(u16, &[u8])>) -> ReceivedCOVNotification {
    let source = routed.map(|(network, address)| NpduAddress {
        network,
        mac_address: MacAddr::from_slice(address),
    });
    ReceivedCOVNotification::new(
        notification(1, ai(1), 60),
        mac,
        &source,
        COVNotificationDelivery::Unconfirmed,
    )
}

/// Send `apdu` to the client as the router relays it: from `source`.
async fn relay(router: &LoopbackTransport, source: Option<(u16, &[u8])>, apdu: Apdu) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();
    let npdu = Npdu {
        source: source.map(|(network, address)| NpduAddress {
            network,
            mac_address: MacAddr::from_slice(address),
        }),
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    router.send_unicast(&npdu_buf, &CLIENT).await.unwrap();
}

async fn relay_notification(
    router: &LoopbackTransport,
    source: Option<(u16, &[u8])>,
    time_remaining: u32,
) {
    let mut service_request = BytesMut::new();
    notification(9, ai(1), time_remaining).encode(&mut service_request);
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
        service_request: service_request.freeze(),
    });
    relay(router, source, apdu).await;
}

/// The next SubscribeCOV the router receives: it goes to the router with
/// the station's network and address as its destination.
async fn routed_subscribe(rx: &mut mpsc::Receiver<ReceivedNpdu>) -> (u8, SubscribeCOVRequest) {
    let received = rx.recv().await.expect("router channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    let destination = npdu
        .destination
        .expect("a routed request names its destination");
    assert_eq!(destination.network, NETWORK);
    assert_eq!(&destination.mac_address[..], &STATION);
    let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected a confirmed request");
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::SUBSCRIBE_COV
    );
    (
        request.invoke_id,
        SubscribeCOVRequest::decode(&request.service_request).unwrap(),
    )
}

async fn ack(router: &LoopbackTransport, invoke_id: u8) {
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
    });
    relay(router, Some((NETWORK, &STATION)), ack).await;
}

/// The subscription and its renewals go through the router to the station,
/// and a notification moves the renewal only when it comes from the
/// station's network and address: not from another station on that
/// network, nor from a local device whose MAC equals the station address.
#[tokio::test(start_paused = true)]
async fn a_routed_subscription_renews_through_the_router_and_follows_its_station() {
    let (client_transport, mut router) = LoopbackTransport::pair(CLIENT.to_vec(), ROUTER.to_vec());
    let mut rx = router.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );
    let start = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            client
                .manage_cov_subscription(
                    CovTarget::routed(&ROUTER, NETWORK, &STATION),
                    9,
                    ai(1),
                    false,
                    60,
                    ManagedCOVSubscriptionOptions::default()
                        .with_renewal_margin(Duration::from_secs(10)),
                )
                .await
        }
    });
    let (invoke_id, request) = routed_subscribe(&mut rx).await;
    assert_eq!(request.subscriber_process_identifier, 9);
    assert_eq!(request.lifetime, Some(60));
    ack(&router, invoke_id).await;
    let managed = start.await.unwrap().unwrap();
    let mut events = managed.events();

    // Neither of these is the station; had either matched, its time would
    // come first.
    relay_notification(&router, Some((NETWORK, &[0x06])), 40).await;
    relay_notification(&router, None, 41).await;
    relay_notification(&router, Some((NETWORK, &STATION)), 3).await;
    assert_eq!(
        events.recv().await.unwrap(),
        ManagedCOVSubscriptionEvent::NotificationObserved {
            time_remaining: 3,
            renew_after: Duration::ZERO,
        }
    );
    assert_eq!(
        events.recv().await.unwrap(),
        ManagedCOVSubscriptionEvent::ImpendingExpiry { time_remaining: 3 }
    );

    let (renewal, request) = routed_subscribe(&mut rx).await;
    assert_eq!(request.lifetime, Some(60));
    ack(&router, renewal).await;
    assert_eq!(
        events.recv().await.unwrap(),
        ManagedCOVSubscriptionEvent::Renewed {
            requested_lifetime: 60,
            renew_after: Duration::from_secs(50),
        }
    );

    managed.stop().await;
    let Ok(mut client) = Arc::try_unwrap(client) else {
        panic!("managed subscription kept a client reference after stop");
    };
    client.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[test]
fn a_local_target_takes_only_its_mac_with_no_source_network() {
    let target = CovTarget::local(&[0x05]);
    assert!(target.sent(&received(&[0x05], None), None));
    assert!(!target.sent(&received(&[0x06], None), None));
    assert!(!target.sent(&received(&[0x05], Some((NETWORK, &[0x05]))), None));
}

#[test]
fn a_routed_target_takes_its_network_and_station_through_any_router() {
    let target = CovTarget::routed(&ROUTER, NETWORK, &STATION);
    assert!(target.sent(&received(&ROUTER, Some((NETWORK, &STATION))), None));
    // Another router on the way back still names the same station.
    assert!(target.sent(&received(&[0x09], Some((NETWORK, &STATION))), None));
    assert!(!target.sent(&received(&ROUTER, Some((NETWORK, &[0x06]))), None));
    assert!(!target.sent(&received(&ROUTER, Some((8, &STATION))), None));
    assert!(!target.sent(&received(&STATION, None), None));
    // Once this network is known to be the target's, the station answers as
    // a local device (#1358).
    assert!(target.sent(&received(&STATION, None), Some(NETWORK)));
    assert!(!target.sent(&received(&STATION, None), Some(8)));
}

#[test]
fn a_discovered_device_is_routed_when_its_i_am_came_through_a_router() {
    let device = |routed: Option<(u16, &[u8])>| DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        mac_address: MacAddr::from_slice(&ROUTER),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 0,
        last_seen: std::time::Instant::now(),
        source_network: routed.map(|(network, _)| network),
        source_address: routed.map(|(_, address)| MacAddr::from_slice(address)),
    };
    assert_eq!(
        CovTarget::of_device(&device(Some((NETWORK, &STATION)))),
        CovTarget::routed(&ROUTER, NETWORK, &STATION)
    );
    assert_eq!(
        CovTarget::of_device(&device(None)),
        CovTarget::local(&ROUTER)
    );
}

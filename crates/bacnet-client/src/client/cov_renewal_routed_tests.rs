//! Managed COV subscriptions to a routed device (#1790, #2588): requests
//! carry the device's network and address, only notifications from that
//! address move the renewal, and the cancel goes to the same station.

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
fn received(mac: &[u8], routed: Option<(u16, &[u8])>) -> COVEvent {
    let source = routed.map(|(network, address)| NpduAddress {
        network,
        mac_address: MacAddr::from_slice(address),
    });
    COVEvent::Object(ReceivedCOVNotification::new(
        notification(1, ai(1), 60),
        mac,
        &source,
        COVNotificationDelivery::Unconfirmed,
    ))
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
    notification(9, ai(1), time_remaining)
        .encode(&mut service_request)
        .unwrap();
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

/// The device-table row of device 100, heard through `ROUTER` from
/// `routed`, or on this link when `routed` is `None`.
fn device(routed: Option<(u16, &[u8])>) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        mac_address: MacAddr::from_slice(&ROUTER),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
        vendor_id: 0,
        last_seen: std::time::Instant::now(),
        last_announced: None,
        source_network: routed.map(|(network, _)| network),
        source_address: routed.map(|(_, address)| MacAddr::from_slice(address)),
    }
}

/// The subscription to a device by instance, and its renewals, go through
/// the router to the station, and a notification moves the renewal only
/// when it comes from the station's network and address: not from another
/// station on that network, nor from a local device whose MAC equals the
/// station address. After stop, the cancel goes to the same station for the
/// same process and object.
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
    client
        .device_table
        .lock()
        .upsert(device(Some((NETWORK, &STATION))));
    let to = client.resolve_device(100).await.unwrap();
    assert_eq!(to, Station::routed(&ROUTER, NETWORK, &STATION));
    let start = tokio::spawn({
        let client = Arc::clone(&client);
        let to = to.clone();
        async move {
            client
                .manage_cov_subscription(
                    &to,
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
    let cancel = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.unsubscribe_cov(&to, 9, ai(1)).await }
    });
    let (invoke_id, request) = routed_subscribe(&mut rx).await;
    assert_eq!(
        (
            request.subscriber_process_identifier,
            request.monitored_object_identifier,
            request.issue_confirmed_notifications,
            request.lifetime,
        ),
        (9, ai(1), None, None),
        "a cancel: the same process and object, no mode or lifetime"
    );
    ack(&router, invoke_id).await;
    cancel.await.unwrap().unwrap();
    let Ok(mut client) = Arc::try_unwrap(client) else {
        panic!("managed subscription kept a client reference after stop");
    };
    client.stop().await.unwrap();
    router.stop().await.unwrap();
}

fn renew_early() -> ManagedCOVSubscriptionOptions {
    ManagedCOVSubscriptionOptions::default().with_renewal_margin(Duration::from_secs(10))
}

/// A subscription to a station named by its address takes the router its
/// row names when it starts and keeps it: a renewal still goes after the
/// purge has taken the row, as it does for a device that only notifies and
/// is never asked anything past its row's age limit. With no row at the
/// start, nothing is sent.
#[tokio::test(start_paused = true)]
async fn an_address_keeps_the_router_it_started_with_after_its_row_goes() {
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
    let to = Station::remote(NETWORK, &STATION);
    let refused = Arc::clone(&client)
        .manage_cov_subscription(&to, 9, ai(1), false, 60, renew_early())
        .await
        .err();
    assert!(
        matches!(refused, Some(Error::UnknownDevice { .. })),
        "{refused:?}"
    );
    assert!(rx.try_recv().is_err(), "nothing sent without a route");
    // An address on this link, by network 0 or the client's own number
    // once known, is fixed as the local station.
    let fixed = |to: Station| client.with_fixed_route(to).unwrap();
    assert_eq!(
        fixed(Station::remote(0, &STATION)),
        Station::local(&STATION)
    );
    client
        .network
        .local_network_number()
        .publish(bacnet_types::network_number::NetworkNumber::configured(8).unwrap());
    assert_eq!(
        fixed(Station::remote(8, &STATION)),
        Station::local(&STATION)
    );

    client
        .device_table
        .lock()
        .upsert(device(Some((NETWORK, &STATION))));
    let start = tokio::spawn({
        let client = Arc::clone(&client);
        let to = to.clone();
        async move {
            client
                .manage_cov_subscription(&to, 9, ai(1), false, 60, renew_early())
                .await
        }
    });
    let (invoke_id, _) = routed_subscribe(&mut rx).await;
    ack(&router, invoke_id).await;
    let managed = start.await.unwrap().unwrap();
    let mut events = managed.events();

    client.device_table.lock().clear();
    let (renewal, request) =
        tokio::time::timeout(Duration::from_secs(120), routed_subscribe(&mut rx))
            .await
            .expect("the renewal goes through the router the subscription started with");
    assert_eq!(request.lifetime, Some(60));
    ack(&router, renewal).await;
    assert_eq!(
        events.recv().await.unwrap(),
        ManagedCOVSubscriptionEvent::Renewed {
            requested_lifetime: 60,
            renew_after: Duration::from_secs(50),
        }
    );

    // The cancel goes to the station the subscription went to, through the
    // same router, with the table still empty; to the bare address it has
    // no route.
    let station = managed.station().clone();
    assert_eq!(station, Station::routed(&ROUTER, NETWORK, &STATION));
    managed.stop().await;
    let refused = client.unsubscribe_cov(&to, 9, ai(1)).await.err();
    assert!(
        matches!(refused, Some(Error::UnknownDevice { .. })),
        "{refused:?}"
    );
    let cancel = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.unsubscribe_cov(&station, 9, ai(1)).await }
    });
    let (invoke_id, request) = routed_subscribe(&mut rx).await;
    assert_eq!(
        (request.issue_confirmed_notifications, request.lifetime),
        (None, None),
        "a cancel"
    );
    ack(&router, invoke_id).await;
    cancel.await.unwrap().unwrap();

    let Ok(mut client) = Arc::try_unwrap(client) else {
        panic!("managed subscription kept a client reference after stop");
    };
    client.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[test]
fn a_local_target_takes_only_its_mac_with_no_source_network() {
    let target = Station::local(&[0x05]);
    assert!(target.is_source_of(&received(&[0x05], None), None));
    assert!(!target.is_source_of(&received(&[0x06], None), None));
    assert!(!target.is_source_of(&received(&[0x05], Some((NETWORK, &[0x05]))), None));
}

#[test]
fn a_routed_target_takes_its_network_and_station_through_any_router() {
    let target = Station::routed(&ROUTER, NETWORK, &STATION);
    assert!(target.is_source_of(&received(&ROUTER, Some((NETWORK, &STATION))), None));
    // Another router on the way back still names the same station.
    assert!(target.is_source_of(&received(&[0x09], Some((NETWORK, &STATION))), None));
    assert!(!target.is_source_of(&received(&ROUTER, Some((NETWORK, &[0x06]))), None));
    assert!(!target.is_source_of(&received(&ROUTER, Some((8, &STATION))), None));
    assert!(!target.is_source_of(&received(&STATION, None), None));
    // Once this network is known to be the target's, the station answers as
    // a local device (#1358).
    assert!(target.is_source_of(&received(&STATION, None), Some(NETWORK)));
    assert!(!target.is_source_of(&received(&STATION, None), Some(8)));
    // An address on network 0 is on this link: its notifications come with
    // no source network.
    let here = Station::remote(0, &STATION);
    assert!(here.is_source_of(&received(&STATION, None), None));
    assert!(!here.is_source_of(&received(&[0x06], None), None));
    assert!(!here.is_source_of(&received(&ROUTER, Some((NETWORK, &STATION))), None));
    // An address, with the router left to the table, matches the same way.
    let remote = Station::remote(NETWORK, &STATION);
    assert!(remote.is_source_of(&received(&[0x09], Some((NETWORK, &STATION))), None));
    assert!(!remote.is_source_of(&received(&ROUTER, Some((NETWORK, &[0x06]))), None));
    assert!(!remote.is_source_of(&received(&STATION, None), None));
    assert!(remote.is_source_of(&received(&STATION, None), Some(NETWORK)));
}

#[test]
fn a_discovered_device_is_routed_when_its_i_am_came_through_a_router() {
    assert_eq!(
        Station::from(&device(Some((NETWORK, &STATION)))),
        Station::routed(&ROUTER, NETWORK, &STATION)
    );
    assert_eq!(Station::from(&device(None)), Station::local(&ROUTER));
}

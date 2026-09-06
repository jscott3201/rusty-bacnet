use super::*;
use bacnet_encoding::apdu::{encode_apdu, UnconfirmedRequest};
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

fn i_am_npdu(instance: u32, vendor_id: u16) -> BytesMut {
    i_am_npdu_with_capabilities(instance, vendor_id, 1476, Segmentation::NONE)
}

fn i_am_npdu_with_capabilities(
    instance: u32,
    vendor_id: u16,
    max_apdu_length: u32,
    segmentation_supported: Segmentation,
) -> BytesMut {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length,
        segmentation_supported,
        vendor_id,
    }
    .encode(&mut service_request);

    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();

    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    npdu_buf
}

#[tokio::test]
async fn device_events_emit_discovered_and_updated_for_i_am() {
    let client_mac = vec![0x01];
    let sender_mac = vec![0x02];
    let (client_transport, sender_transport) =
        LoopbackTransport::pair(client_mac, sender_mac.clone());
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut events = client.device_events();
    let mut collisions = client.device_collision_events();

    sender_transport
        .send_unicast(&i_am_npdu(1001, 42), client.local_mac())
        .await
        .unwrap();
    let event = timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("timed out waiting for discovered event")
        .expect("device event channel closed");
    assert_eq!(event.kind, DeviceEventKind::Discovered);
    assert_eq!(event.device.object_identifier.instance_number(), 1001);
    assert_eq!(event.device.mac_address.as_slice(), sender_mac.as_slice());
    assert_eq!(event.device.vendor_id, 42);
    assert!(matches!(
        collisions.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    sender_transport
        .send_unicast(
            &i_am_npdu_with_capabilities(1001, 84, 480, Segmentation::BOTH),
            client.local_mac(),
        )
        .await
        .unwrap();
    let event = timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("timed out waiting for updated event")
        .expect("device event channel closed");
    assert_eq!(event.kind, DeviceEventKind::Updated);
    assert_eq!(event.device.object_identifier.instance_number(), 1001);
    assert_eq!(event.device.vendor_id, 84);
    assert_eq!(event.device.max_apdu_length, 480);
    assert_eq!(event.device.segmentation_supported, Segmentation::BOTH);
    let row = client.get_device(1001).await.unwrap();
    assert_eq!(row.vendor_id, 84);
    assert_eq!(row.max_apdu_length, 480);
    assert_eq!(row.segmentation_supported, Segmentation::BOTH);
    assert!(matches!(
        collisions.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    client.stop().await.unwrap();
}

#[tokio::test]
async fn collision_event_retains_authoritative_route_without_device_update() {
    let client_mac = vec![0x01];
    let incoming_mac = vec![0x02];
    let retained_mac = vec![0x03];
    let (client_transport, sender_transport) =
        LoopbackTransport::pair(client_mac, incoming_mac.clone());
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut device_events = client.device_events();
    let mut collision_events = client.device_collision_events();
    let retained_seen = Instant::now();
    let retained = DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1001).unwrap(),
        mac_address: MacAddr::from_slice(&retained_mac),
        max_apdu_length: 1024,
        segmentation_supported: Segmentation::RECEIVE,
        max_segments_accepted: Some(8),
        vendor_id: 42,
        last_seen: retained_seen,
        source_network: None,
        source_address: None,
    };
    client.device_table.lock().await.upsert(retained);

    let before_send = Instant::now();
    sender_transport
        .send_unicast(
            &i_am_npdu_with_capabilities(1001, 84, 480, Segmentation::BOTH),
            client.local_mac(),
        )
        .await
        .unwrap();
    let event = timeout(Duration::from_secs(1), collision_events.recv())
        .await
        .expect("timed out waiting for collision event")
        .expect("device collision event channel closed");
    let after_receive = Instant::now();

    assert_eq!(event.retained.object_identifier.instance_number(), 1001);
    assert_eq!(
        event.retained.mac_address.as_slice(),
        retained_mac.as_slice()
    );
    assert_eq!(event.retained.max_apdu_length, 1024);
    assert_eq!(event.retained.segmentation_supported, Segmentation::RECEIVE);
    assert_eq!(event.retained.max_segments_accepted, Some(8));
    assert_eq!(event.retained.vendor_id, 42);
    assert_eq!(event.retained.last_seen, retained_seen);
    assert!(event.retained.is_local());
    assert_eq!(event.incoming.object_identifier.instance_number(), 1001);
    assert_eq!(
        event.incoming.mac_address.as_slice(),
        incoming_mac.as_slice()
    );
    assert_eq!(event.incoming.max_apdu_length, 480);
    assert_eq!(event.incoming.segmentation_supported, Segmentation::BOTH);
    assert_eq!(event.incoming.max_segments_accepted, None);
    assert_eq!(event.incoming.vendor_id, 84);
    assert!(event.incoming.last_seen >= before_send);
    assert!(event.incoming.last_seen <= after_receive);
    assert!(event.incoming.is_local());
    assert!(matches!(
        device_events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    let row = client.get_device(1001).await.unwrap();
    assert_eq!(row.mac_address.as_slice(), retained_mac.as_slice());
    assert_eq!(row.max_apdu_length, 1024);
    assert_eq!(row.segmentation_supported, Segmentation::RECEIVE);
    assert_eq!(row.max_segments_accepted, Some(8));
    assert_eq!(row.vendor_id, 42);
    assert_eq!(row.last_seen, retained_seen);
    let resolved = client.resolve_device(1001).await.unwrap();
    assert_eq!(resolved, (retained_mac, None));

    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn device_events_emit_lost_when_purge_removes_stale_device() {
    let (client_transport, _peer_transport) = LoopbackTransport::pair(vec![0x10], vec![0x11]);
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut events = client.device_events();

    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    client.device_table.lock().await.upsert(DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2001).unwrap(),
        mac_address: MacAddr::from_slice(&[192, 168, 1, 42, 0xBA, 0xC0]),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap_or_else(Instant::now),
        source_network: None,
        source_address: None,
    });

    tokio::time::advance(Duration::from_secs(300)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    let event = events.try_recv().expect("lost device event");
    assert_eq!(event.kind, DeviceEventKind::Lost);
    assert_eq!(event.device.object_identifier.instance_number(), 2001);
    assert!(client.discovered_devices().await.is_empty());

    client.stop().await.unwrap();
}

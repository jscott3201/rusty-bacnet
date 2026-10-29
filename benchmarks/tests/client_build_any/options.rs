//! Wire-driven COV and event channel checks shared by all three builders.
use super::*;
use bacnet_client::client::{COVNotificationDelivery, ConfirmedCOVNotificationResponse};
use bacnet_encoding::{
    apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu},
    npdu::{Npdu, decode_npdu, encode_npdu},
};
use bacnet_services::cov::COVNotificationRequest;
use bacnet_transport::{
    port::ReceivedNpdu,
    sc::{ScConnectionState, ScTransport},
    sc_hub::ScHub,
    sc_tls::TlsWebSocket,
};
use bacnet_types::{
    enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier, RejectReason},
    primitives::ObjectIdentifier,
};
use bytes::{Bytes, BytesMut};
use tokio::sync::{broadcast::error::RecvError, mpsc};

macro_rules! configured {
    ($builder:expr_2021) => {
        $builder
            .apdu_timeout_ms(2_000)
            .apdu_retries(0)
            .max_segments(Some(8))
            .segmented_response_accepted(false)
            .proposed_window_size(4)
            .min_request_interval_ms(5)
            .cov_channel_capacity(1)
            .event_channel_capacity(2)
            .confirmed_cov_notification_ack_policy(|received| {
                assert_eq!(received.delivery, COVNotificationDelivery::Confirmed);
                ConfirmedCOVNotificationResponse::Reject(RejectReason::OTHER)
            })
    };
}

async fn send<T: TransportPort>(
    peer: &T,
    client: &[u8],
    service: ConfirmedServiceChoice,
    payload: Bytes,
    invoke: u8,
) {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: invoke,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: payload,
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Default::default()
        },
    )
    .unwrap();
    bounded(peer.send_unicast(&npdu, client)).await.unwrap();
}

async fn reply(rx: &mut mpsc::Receiver<ReceivedNpdu>) -> Apdu {
    let packet = bounded(rx.recv()).await.unwrap();
    decode_apdu(decode_npdu(packet.npdu).unwrap().payload).unwrap()
}

async fn options<T: TransportPort + 'static, P: TransportPort>(
    mut client: BACnetClient<T>,
    peer: &P,
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
) {
    let mut cov = client.cov_notifications();
    let mut event = client.event_notifications();
    for invoke in 1..=3 {
        let mut payload = BytesMut::new();
        COVNotificationRequest {
            subscriber_process_identifier: u32::from(invoke),
            initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
            monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7)
                .unwrap(),
            time_remaining: 60,
            list_of_values: vec![bacnet_services::common::BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x44, 0x42, 0x48, 0, 0],
                priority: None,
            }],
        }
        .encode(&mut payload);
        send(
            peer,
            client.local_mac(),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            payload.freeze(),
            invoke,
        )
        .await;
        let Apdu::Reject(rejected) = reply(rx).await else {
            panic!("custom COV policy must reject");
        };
        assert_eq!(
            (rejected.invoke_id, rejected.reject_reason),
            (invoke, RejectReason::OTHER)
        );
    }
    assert_eq!(cov.recv().await.unwrap_err(), RecvError::Lagged(2));
    assert_eq!(
        cov.recv()
            .await
            .unwrap()
            .notification
            .subscriber_process_identifier,
        3
    );
    // Existing event-notification fixture: process 42, device 100, AI 7,
    // sequence 5, out-of-range alarm. Every response synchronizes dispatch.
    let event_payload = Bytes::from_static(&[
        0x09, 42, 0x1c, 2, 0, 0, 100, 0x2c, 0, 0, 0, 7, 0x3e, 0x19, 5, 0x3f, 0x49, 2, 0x59, 16,
        0x69, 5, 0x89, 0, 0x99, 1, 0xa9, 0, 0xb9, 3, 0xce, 0x5e, 0x0c, 0x42, 0x48, 0, 0, 0x1a, 4,
        0, 0x2c, 0x3f, 0x80, 0, 0, 0x3c, 0x42, 0x40, 0, 0, 0x5f, 0xcf,
    ]);
    for invoke in 4..=6 {
        send(
            peer,
            client.local_mac(),
            ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            event_payload.clone(),
            invoke,
        )
        .await;
        let Apdu::SimpleAck(ack) = reply(rx).await else {
            panic!("event must be acknowledged");
        };
        assert_eq!(ack.invoke_id, invoke);
    }
    assert_eq!(event.recv().await.unwrap_err(), RecvError::Lagged(1));
    assert_eq!(
        event.recv().await.unwrap().notification.process_identifier,
        42
    );
    assert!(cov.try_recv().is_err());
    {
        let request = Box::pin(client.read_property(
            peer.local_mac(),
            ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
            PropertyIdentifier::OBJECT_IDENTIFIER,
            None,
        ));
        tokio::pin!(request);
        let sent = tokio::select! {
            result = &mut request => panic!("peer has not replied: {result:?}"),
            sent = reply(rx) => sent,
        };
        let Apdu::ConfirmedRequest(sent) = sent else {
            panic!("expected ReadProperty request");
        };
        assert_eq!(sent.max_segments, Some(8));
        assert!(!sent.segmented_response_accepted);
        assert_eq!(sent.max_apdu_length, 1476);
        // Dropping the outstanding request also exercises the erased client's
        // transaction cleanup before stop.
    }
    client.stop().await.unwrap();
}

#[tokio::test]
async fn bip_builders_keep_custom_cov_policy_and_independent_channel_capacities() {
    for erased in [false, true] {
        let mut peer = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut rx = peer.start().await.unwrap();
        let builder = configured!(
            BACnetClient::bip_builder()
                .interface(Ipv4Addr::LOCALHOST)
                .port(0)
        );
        if erased {
            options(builder.build_any().await.unwrap(), &peer, &mut rx).await;
        } else {
            options(builder.build().await.unwrap(), &peer, &mut rx).await;
        }
        peer.stop().await.unwrap();
    }
}

#[tokio::test]
async fn bip6_builders_keep_custom_cov_policy_and_independent_channel_capacities() {
    for erased in [false, true] {
        let mut peer = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(234));
        let mut rx = peer.start().await.unwrap();
        let builder = configured!(
            BACnetClient::bip6_builder()
                .interface(Ipv6Addr::LOCALHOST)
                .port(0)
                .device_instance(123)
        );
        if erased {
            let client = builder.build_any().await.unwrap();
            let AnyTransport::Bip6(transport) = client.transport() else {
                panic!("expected B/IPv6");
            };
            peer.resolve_virtual_address(transport.local_address().unwrap())
                .await
                .unwrap();
            options(client, &peer, &mut rx).await;
        } else {
            let client = builder.build().await.unwrap();
            peer.resolve_virtual_address(client.transport().local_address().unwrap())
                .await
                .unwrap();
            options(client, &peer, &mut rx).await;
        }
        peer.stop().await.unwrap();
    }
}

#[tokio::test]
async fn sc_builders_keep_custom_cov_policy_and_independent_channel_capacities() {
    let certs = generate_test_certs();
    let mut hub = Box::pin(ScHub::start_with_uuid(
        "127.0.0.1:0",
        try_make_hub_tls_config(&certs).unwrap(),
        [9; 6],
        [9; 16],
    ))
    .await
    .unwrap();
    let url = format!("wss://localhost:{}", hub.local_addr().unwrap().port());
    for erased in [false, true] {
        let ws = TlsWebSocket::connect(&url, try_make_node_tls_config(&certs).unwrap())
            .await
            .unwrap();
        let mut peer = ScTransport::new(ws, [8; 6]).with_device_uuid([8; 16]);
        let mut rx = peer.start().await.unwrap();
        let builder = configured!(
            BACnetClient::sc_builder()
                .hub_url(&url)
                .tls_config(try_make_node_tls_config(&certs).unwrap())
                .vmac([2; 6])
                .device_uuid([3; 16])
        );
        if erased {
            let client = Box::pin(builder.build_any()).await.unwrap();
            let AnyTransport::Sc(transport) = client.transport() else {
                panic!("expected SC");
            };
            assert_eq!(
                *transport.connection_state_changes().borrow(),
                ScConnectionState::Connected
            );
            Box::pin(options(client, &peer, &mut rx)).await;
        } else {
            Box::pin(options(
                Box::pin(builder.build()).await.unwrap(),
                &peer,
                &mut rx,
            ))
            .await;
        }
        peer.stop().await.unwrap();
    }
    hub.stop().await;
}

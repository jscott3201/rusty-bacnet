//! Which answers restart a discovery-table row's age (#2560): the device's
//! acknowledgment, or its Error, plain or segmented; not a timeout, and not
//! a router's refusal of the path.

use std::sync::Arc;
use std::time::Instant;

use bacnet_encoding::apdu::{self, Apdu, ErrorPdu, SegmentAck, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ErrorClass, ErrorCode, ObjectType, Segmentation,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use tokio::task::JoinHandle;
use tokio::time::{Duration, timeout};

use super::routed_path_limit_tests::{
    DNET, confirmed_request, harness as capture, inject_reason_4, routed_request,
};
use super::{BACnetClient, ClientConfig};
use crate::discovery::DiscoveredDevice;

const CLIENT_MAC: &[u8] = &[0x01];
const DEVICE_MAC: &[u8] = &[0x02];

/// How the device answers the last frame of a request.
#[derive(Clone, Copy)]
enum Answer {
    SimpleAck,
    Error,
    Silent,
}

/// A device row at `mac` on `network` (0 for local), heard at `seen`.
fn row(network: u16, mac: &[u8], max_apdu: u32, seen: Instant) -> DiscoveredDevice {
    let routed = (network != 0).then(|| MacAddr::from_slice(mac));
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap(),
        mac_address: MacAddr::from_slice(if routed.is_some() { &[0x09] } else { mac }),
        max_apdu_length: max_apdu,
        segmentation_supported: Segmentation::SEGMENTED_BOTH,
        max_segments_accepted: None,
        vendor_id: 1,
        last_seen: seen,
        last_announced: Some(seen),
        source_network: routed.is_some().then_some(network),
        source_address: routed,
    }
}

/// A client holding the device's row, heard at the returned instant, and
/// the device answering every request with `answer` after acknowledging
/// each segment but the last.
async fn harness(
    answer: Answer,
    max_apdu: u32,
) -> (BACnetClient<LoopbackTransport>, JoinHandle<()>, Instant) {
    let (client_transport, mut device) =
        LoopbackTransport::pair(CLIENT_MAC.to_vec(), DEVICE_MAC.to_vec());
    let mut rx = device.start().await.unwrap();
    let client = BACnetClient::start(
        ClientConfig {
            apdu_timeout_ms: 100,
            apdu_retries: 0,
            proposed_window_size: 1,
            ..ClientConfig::default()
        },
        client_transport,
    )
    .await
    .unwrap();
    let task = tokio::spawn(async move {
        while let Some(received) = rx.recv().await {
            let npdu = decode_npdu(received.npdu).unwrap();
            let Ok(Apdu::ConfirmedRequest(request)) = apdu::decode_apdu(npdu.payload) else {
                continue;
            };
            let reply = if request.segmented && request.more_follows {
                Apdu::SegmentAck(SegmentAck {
                    negative_ack: false,
                    sent_by_server: true,
                    invoke_id: request.invoke_id,
                    sequence_number: request.sequence_number.unwrap(),
                    actual_window_size: 1,
                })
            } else {
                match answer {
                    Answer::SimpleAck => Apdu::SimpleAck(SimpleAck {
                        invoke_id: request.invoke_id,
                        service_choice: request.service_choice,
                    }),
                    Answer::Error => Apdu::Error(ErrorPdu {
                        invoke_id: request.invoke_id,
                        service_choice: request.service_choice,
                        error_class: ErrorClass::OBJECT,
                        error_code: ErrorCode::UNKNOWN_OBJECT,
                        error_data: Bytes::new(),
                    }),
                    Answer::Silent => continue,
                }
            };
            let mut apdu_buf = BytesMut::new();
            encode_apdu(&mut apdu_buf, &reply).unwrap();
            let mut npdu_buf = BytesMut::new();
            let npdu = Npdu {
                payload: apdu_buf.freeze(),
                ..Npdu::default()
            };
            encode_npdu(&mut npdu_buf, &npdu).unwrap();
            device.send_unicast(&npdu_buf, CLIENT_MAC).await.unwrap();
        }
    });
    let seen = crate::discovery::now();
    client
        .device_table
        .lock()
        .upsert(row(0, DEVICE_MAC, max_apdu, seen));
    (client, task, seen)
}

fn last_seen<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    network: u16,
    mac: &[u8],
) -> Instant {
    client
        .device_table
        .lock()
        .get(network, mac)
        .unwrap()
        .last_seen
}

/// A WriteProperty of `len` octets of service data to the device.
async fn write(client: &BACnetClient<LoopbackTransport>, len: usize) -> Result<Bytes, Error> {
    let data = vec![0x0C; len];
    timeout(
        Duration::from_secs(5),
        client.confirmed_request(DEVICE_MAC, ConfirmedServiceChoice::WRITE_PROPERTY, &data),
    )
    .await
    .expect("the request ends")
}

#[tokio::test]
async fn an_acknowledged_segmented_request_restarts_the_age() {
    let (mut client, task, seen) = harness(Answer::SimpleAck, 128).await;
    let result = write(&client, 400).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(last_seen(&client, 0, DEVICE_MAC) > seen, "no refresh");
    // Only an I-Am announces: whoever answers at the address, the row's
    // announcement keeps its age (#2560).
    let announced = client
        .device_table
        .lock()
        .get(0, DEVICE_MAC)
        .unwrap()
        .last_announced;
    assert_eq!(announced, Some(seen), "an answer moved the announcement");
    task.abort();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn an_error_answer_restarts_the_age() {
    for max_apdu in [1476, 128] {
        let (mut client, task, seen) = harness(Answer::Error, max_apdu).await;
        let result = write(&client, 400).await;
        assert!(matches!(result, Err(Error::Remote(_))), "{result:?}");
        assert!(
            last_seen(&client, 0, DEVICE_MAC) > seen,
            "an Error to a request of max APDU {max_apdu} did not refresh"
        );
        task.abort();
        client.stop().await.unwrap();
    }
}

#[tokio::test]
async fn a_timeout_leaves_the_age() {
    let (mut client, task, seen) = harness(Answer::Silent, 1476).await;
    let result = write(&client, 10).await;
    assert!(result.is_err());
    assert_eq!(
        last_seen(&client, 0, DEVICE_MAC),
        seen,
        "a timeout refreshed"
    );
    task.abort();
    client.stop().await.unwrap();
}

/// A router refusing a message too long for the path is no answer from the
/// device behind it: the device's row keeps its age.
#[tokio::test]
async fn a_routers_refusal_leaves_the_age() {
    let (router, dadr) = (vec![2], vec![3]);
    let (transport, inbound, mut outbound) = capture(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(
            ClientConfig {
                apdu_timeout_ms: 40,
                apdu_retries: 0,
                ..ClientConfig::default()
            },
            transport,
        )
        .await
        .unwrap(),
    );
    client
        .configure_routed_path_max_npdu(&router, DNET, 1497)
        .await
        .unwrap();
    let seen = crate::discovery::now();
    client
        .device_table
        .lock()
        .upsert(row(DNET, &dadr, 1476, seen));
    let task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x44; 300],
    );
    let _ = confirmed_request(outbound.recv().await.unwrap(), &router);
    inject_reason_4(&inbound, &router, DNET).await;
    let result = timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, Err(Error::RoutedPathTooLong { dnet: DNET })),
        "{result:?}"
    );
    assert_eq!(
        last_seen(&client, DNET, &dadr),
        seen,
        "a router's refusal refreshed"
    );
    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

//! A confirmed request whose link-layer source is a group address is
//! ignored and counted (#1504): its answer, and the confirmed COV
//! notifications of a subscription it makes, would go back to that address
//! and reach every node in the group. The built-in transports hand up no
//! such source, so the shared test transport plays a custom one.

use std::time::Duration;

use bacnet_encoding::apdu::{encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu};
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::server::test_transport::{TestTransport, BIP_LOCAL_MAC};
use crate::server::{BACnetServer, ServerConfig};

/// A B/IP multicast address the test transport reports as a group.
const GROUP: [u8; 6] = [224, 0, 0, 1, 0xBA, 0xC0];
/// An ordinary peer.
const PEER: [u8; 6] = [127, 0, 0, 2, 0xBA, 0xC0];

fn confirmed(invoke_id: u8, service_choice: ConfirmedServiceChoice, body: Bytes) -> Bytes {
    let request = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request: body,
    };
    let mut apdu = BytesMut::new();
    encode_apdu(&mut apdu, &Apdu::ConfirmedRequest(request)).unwrap();
    apdu.freeze()
}

/// `apdu` from link-layer `source`, sent to this device alone.
fn from(source: &[u8], apdu: Bytes) -> ReceivedNpdu {
    let mut npdu = BytesMut::new();
    let npdu_fields = Npdu {
        expecting_reply: true,
        payload: apdu,
        ..Default::default()
    };
    encode_npdu(&mut npdu, &npdu_fields).unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(source),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn read_present_value(invoke_id: u8) -> Bytes {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut body);
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
        body.freeze(),
    )
}

fn subscribe_cov(invoke_id: u8) -> Bytes {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: 7,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        issue_confirmed_notifications: Some(true),
        lifetime: Some(300),
    }
    .encode(&mut body)
    .unwrap();
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
        body.freeze(),
    )
}

#[tokio::test]
async fn a_confirmed_request_from_a_group_source_is_ignored_and_counted() {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Group source".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();
    let input = AnalogInputObject::new(1, "Input", 62).unwrap();
    db.add(Box::new(input)).unwrap();
    let (tx, rx) = mpsc::channel(8);
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .group_mac(&GROUP)
        .inbound(rx)
        .build();
    let sent = transport.sent();
    let mut server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .unwrap();

    // From the group: a read and a subscription for confirmed
    // notifications. From the peer last: a subscription and a read, whose
    // answers show the group's went nowhere.
    for frame in [
        from(&GROUP, read_present_value(1)),
        from(&GROUP, subscribe_cov(2)),
        from(&PEER, subscribe_cov(3)),
        from(&PEER, read_present_value(4)),
    ] {
        tx.send(frame).await.unwrap();
    }
    // The peer's SimpleACK and ComplexACK, and its first COV notification.
    tokio::time::timeout(Duration::from_secs(5), sent.wait_for_len(3))
        .await
        .expect("the peer's requests are answered");
    assert_eq!(server.group_source_request_drops(), 2);
    assert_eq!(server.cov_table.read().await.len(), 1, "the peer's alone");
    server.stop().await.unwrap();

    let frames = sent.take();
    assert!(
        frames.iter().all(|frame| frame.mac[..] == PEER),
        "nothing went to the group: {:?}",
        frames.iter().map(|frame| &frame.mac).collect::<Vec<_>>()
    );
    let invoke_ids: Vec<u8> = frames
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::SimpleAck(ack) => Some(ack.invoke_id),
            Apdu::ComplexAck(ack) => Some(ack.invoke_id),
            _ => None,
        })
        .collect();
    assert_eq!(invoke_ids, [3, 4]);
}

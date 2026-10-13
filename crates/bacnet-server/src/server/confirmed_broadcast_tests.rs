//! Confirmed requests that arrive by broadcast (#1257, Clause 5.4.5.1): the
//! dispatch loop drops them whatever the service, before any reassembly, so a
//! ConfirmedEventNotification sent that way is neither acknowledged nor
//! offered to the forwarders, and a segment draws no SegmentACK or Abort.

use super::event_forwarding_tests::{
    copies, database, destination, encoded, notification, unconfirmed, To, LOCAL_DEVICE, PEER_A,
    PEER_B,
};
use super::event_recipient_routing_tests::{address_recipient, LITERAL_BROADCAST_MAC};
use super::test_transport::{SendLog, TestTransport, BIP_LOCAL_MAC};
use super::*;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};

/// How one request reached this device.
#[derive(Clone, Copy)]
enum Sent {
    ToThisDevice,
    LocalBroadcast,
    GlobalBroadcast,
}

/// An unsegmented confirmed request.
fn confirmed_request(
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    service_request: Bytes,
) -> ConfirmedRequestPdu {
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request,
    }
}

/// `request` from `PEER_B`, addressed as `sent` says.
fn inbound(request: ConfirmedRequestPdu, sent: Sent) -> ReceivedNpdu {
    let mut apdu = BytesMut::new();
    encode_apdu(&mut apdu, &Apdu::ConfirmedRequest(request)).unwrap();
    let destination = matches!(sent, Sent::GlobalBroadcast).then(|| NpduAddress {
        network: 0xFFFF,
        mac_address: MacAddr::new(),
    });
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: true,
            destination,
            payload: apdu.freeze(),
            ..Default::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&PEER_B),
        link_layer_group: !matches!(sent, Sent::ToThisDevice),
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn read_device_name() -> Bytes {
    let mut buf = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, LOCAL_DEVICE).unwrap(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut buf);
    buf.freeze()
}

#[tokio::test]
async fn a_confirmed_request_sent_by_broadcast_is_neither_executed_nor_answered() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    let (tx, rx) = mpsc::channel(8);
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(LITERAL_BROADCAST_MAC)
        .inbound(rx)
        .build();
    let sent = transport.sent();
    let mut server = BACnetServer::start(ServerConfig::default(), database(vec![nf]), transport)
        .await
        .unwrap();
    let request = notification(5);
    let event = ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION;

    // The notification by local and by global broadcast, and a ReadProperty
    // by local broadcast, go first; the notification addressed to this
    // device alone goes last.
    for frame in [
        inbound(
            confirmed_request(1, event, encoded(&request)),
            Sent::LocalBroadcast,
        ),
        inbound(
            confirmed_request(2, event, encoded(&request)),
            Sent::GlobalBroadcast,
        ),
        inbound(
            confirmed_request(3, ConfirmedServiceChoice::READ_PROPERTY, read_device_name()),
            Sent::LocalBroadcast,
        ),
        inbound(
            confirmed_request(4, event, encoded(&request)),
            Sent::ToThisDevice,
        ),
    ] {
        tx.send(frame).await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), sent.wait_for_len(2))
        .await
        .expect("the request addressed to this device is answered and forwarded");
    server.stop().await.unwrap();

    let (answers, forwarded): (Vec<_>, Vec<_>) = sent
        .take()
        .into_iter()
        .partition(|frame| frame.mac[..] == PEER_B);
    let answers: Vec<Apdu> = answers.iter().map(|frame| frame.apdu()).collect();
    assert!(
        matches!(
            answers[..],
            [Apdu::SimpleAck(SimpleAck {
                invoke_id: 4,
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            })]
        ),
        "only the request addressed to this device is answered: {answers:?}"
    );
    let log = SendLog::default();
    for frame in forwarded {
        log.push(frame);
    }
    assert_eq!(
        copies(&log, &request),
        [unconfirmed(To::Local(PEER_A.to_vec()), 40)]
    );
    // A dropped notification is not one that no forwarder took.
    assert_eq!(
        server.event_notification_counters(),
        EventNotificationCounters::default()
    );
}

#[tokio::test]
async fn a_segment_sent_by_broadcast_draws_no_segment_ack_or_abort() {
    // Without segmented reception the first segment would draw an Abort, and
    // with it a SegmentACK; by broadcast it draws neither.
    for segmentation in [Segmentation::NONE, Segmentation::BOTH] {
        let (tx, rx) = mpsc::channel(8);
        let transport = TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .broadcast_mac(LITERAL_BROADCAST_MAC)
            .inbound(rx)
            .build();
        let sent = transport.sent();
        let config = ServerConfig {
            segmentation_supported: segmentation,
            ..ServerConfig::default()
        };
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
                segmentation_supported: segmentation,
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let mut server = BACnetServer::start(config, db, transport).await.unwrap();
        // The first of several segments of a ReadProperty, by local
        // broadcast, then a ReadProperty to this device alone as the fence.
        let mut first =
            confirmed_request(1, ConfirmedServiceChoice::READ_PROPERTY, read_device_name());
        first.segmented = true;
        first.more_follows = true;
        first.sequence_number = Some(0);
        first.proposed_window_size = Some(1);
        tx.send(inbound(first, Sent::LocalBroadcast)).await.unwrap();
        tx.send(inbound(
            confirmed_request(2, ConfirmedServiceChoice::READ_PROPERTY, read_device_name()),
            Sent::ToThisDevice,
        ))
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), sent.wait_for_len(1))
            .await
            .expect("the fence is answered");
        server.stop().await.unwrap();
        let answers: Vec<Apdu> = sent.take().iter().map(|frame| frame.apdu()).collect();
        assert!(
            matches!(&answers[..], [Apdu::ComplexAck(ack)] if ack.invoke_id == 2),
            "{segmentation:?}: only the fence is answered: {answers:?}"
        );
    }
}

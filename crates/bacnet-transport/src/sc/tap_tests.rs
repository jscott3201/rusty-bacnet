//! The hub connection's frames reach the tap: the handshake, an NPDU each
//! way, a heartbeat and the Disconnect, in paused time against a scripted
//! hub over a loopback socket.

use std::time::Duration;

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::{Bytes, BytesMut};

use super::*;
use crate::port::TransportPort;
use crate::sc::LoopbackWebSocket;
use crate::sc_frame::{ScFunction, ScMessage, Vmac, decode_sc_message, encode_sc_message};
use crate::tap::{FrameTapReceiver, TappedFrame};

const NODE: Vmac = [0x01; 6];
const HUB: Vmac = [0x10; 6];
const PEER: Vmac = [0x02, 3, 4, 5, 6, 7];
/// A confirmed ReadProperty of Device 1's Object_Name, invoke ID 5.
const REQUEST: [u8; 13] = [
    0x01, 0x04, 0x00, 0x05, 0x05, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D,
];
/// Its ComplexAck: Object_Name "A".
const ANSWER: [u8; 17] = [
    0x01, 0x00, 0x30, 0x05, 0x0C, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x19, 0x4D, 0x3E, 0x72, 0x00, 0x41,
    0x3F,
];

fn message(function: ScFunction, message_id: u16, origin: Option<Vmac>, payload: &[u8]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    let message = ScMessage {
        function,
        message_id,
        originating_vmac: origin,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::copy_from_slice(payload),
    };
    encode_sc_message(&mut buf, &message);
    buf.to_vec()
}

/// Play the hub's side of the handshake.
async fn accept(hub: &LoopbackWebSocket) {
    let request = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    let mut payload = HUB.to_vec();
    payload.extend_from_slice(&[0x33; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let reply = message(
        ScFunction::ConnectAccept,
        request.message_id,
        None,
        &payload,
    );
    hub.send(&reply).await.unwrap();
}

fn next(frames: &mut FrameTapReceiver) -> (TapDirection, ScFunction, TappedFrame) {
    let frame = frames.try_recv().expect("a tapped frame");
    assert_eq!(frame.link, DataLink::Sc);
    assert_eq!(frame.peer, TapPeer::ScHub(ScActiveHub::Primary));
    let function = decode_sc_message(&frame.bytes).unwrap().function;
    (frame.direction, function, frame)
}

#[tokio::test(start_paused = true)]
async fn the_hub_connection_is_tapped_from_connect_to_disconnect() {
    let (socket, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(socket, NODE).with_device_uuid([1; 16]);
    let mut frames = transport.frame_tap().subscribe();
    let hub_task = tokio::spawn(async move {
        accept(&hub).await;
        hub
    });
    let mut npdus = transport.start().await.unwrap();
    let hub = hub_task.await.unwrap();

    // A ReadProperty out through the hub, and its answer back.
    transport.send_unicast(&REQUEST, &PEER).await.unwrap();
    let sent = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    let answer = message(ScFunction::EncapsulatedNpdu, 9, Some(PEER), &ANSWER);
    hub.send(&answer).await.unwrap();
    assert_eq!(&npdus.recv().await.unwrap().npdu[..], ANSWER);
    assert_eq!(&sent.payload[..], REQUEST);

    // Idle for a heartbeat interval: the node sends a Heartbeat-Request.
    let interval = crate::sc::heartbeat::DEFAULT_HEARTBEAT_INTERVAL_MS;
    tokio::time::sleep(Duration::from_millis(interval + 1)).await;
    let heartbeat = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    assert_eq!(heartbeat.function, ScFunction::HeartbeatRequest);
    let ack = message(ScFunction::HeartbeatAck, heartbeat.message_id, None, &[]);
    hub.send(&ack).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;

    let stopping = tokio::spawn(async move {
        transport.stop().await.unwrap();
        transport
    });
    let disconnect = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
    assert_eq!(disconnect.function, ScFunction::DisconnectRequest);
    drop(hub);
    let transport = stopping.await.unwrap();

    let (direction, function, _) = next(&mut frames);
    assert_eq!(
        (direction, function),
        (TapDirection::Outbound, ScFunction::ConnectRequest)
    );
    let (direction, function, _) = next(&mut frames);
    assert_eq!(
        (direction, function),
        (TapDirection::Inbound, ScFunction::ConnectAccept)
    );
    for (expected, wire) in [
        (TapDirection::Outbound, &REQUEST[..]),
        (TapDirection::Inbound, &ANSWER[..]),
    ] {
        let (direction, function, frame) = next(&mut frames);
        assert_eq!(
            (direction, function),
            (expected, ScFunction::EncapsulatedNpdu)
        );
        let npdu = decode_sc_message(&frame.bytes).unwrap().payload;
        assert_eq!(&npdu[..], wire);
        let apdu = decode_apdu(decode_npdu(npdu).unwrap().payload).unwrap();
        let (invoke_id, service) = match apdu {
            Apdu::ConfirmedRequest(request) => (request.invoke_id, request.service_choice),
            Apdu::ComplexAck(ack) => (ack.invoke_id, ack.service_choice),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            (invoke_id, service),
            (5, ConfirmedServiceChoice::READ_PROPERTY)
        );
    }
    let (direction, function, _) = next(&mut frames);
    assert_eq!(
        (direction, function),
        (TapDirection::Outbound, ScFunction::HeartbeatRequest)
    );
    let (direction, function, _) = next(&mut frames);
    assert_eq!(
        (direction, function),
        (TapDirection::Inbound, ScFunction::HeartbeatAck)
    );
    let (direction, function, _) = next(&mut frames);
    assert_eq!(
        (direction, function),
        (TapDirection::Outbound, ScFunction::DisconnectRequest)
    );
    assert!(frames.try_recv().is_none());
    let counters = transport.frame_tap().counters();
    assert_eq!((counters.delivered, counters.dropped), (7, 0));
}

/// Sockets a reconnect or failover connector dials are tapped too, and
/// labelled with their hub.
#[tokio::test(start_paused = true)]
async fn dialled_sockets_are_tapped_for_their_hub() {
    let tap = FrameTap::new();
    let mut frames = tap.subscribe();
    let (socket, hub) = LoopbackWebSocket::pair();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(socket)));
    let connector: WebSocketConnector<LoopbackWebSocket> = std::sync::Arc::new(move || {
        let socket = slot.lock().unwrap().take();
        Box::pin(async move { socket.ok_or_else(|| Error::Encoding("dialled once".into())) })
    });
    let dial = tapped_connector(&connector, &tap, ScActiveHub::Failover);
    let dialled = dial().await.unwrap();
    dialled.send(&[0x0A, 0x00]).await.unwrap();
    assert_eq!(hub.recv().await.unwrap(), [0x0A, 0x00]);
    hub.send(&[0x0B]).await.unwrap();
    assert_eq!(dialled.recv().await.unwrap(), [0x0B]);
    for (direction, bytes) in [
        (TapDirection::Outbound, &[0x0A, 0x00][..]),
        (TapDirection::Inbound, &[0x0B][..]),
    ] {
        let frame = frames.try_recv().unwrap();
        assert_eq!(frame.direction, direction);
        assert_eq!(frame.peer, TapPeer::ScHub(ScActiveHub::Failover));
        assert_eq!(&frame.bytes[..], bytes);
    }
    assert!(dial().await.is_err());
}

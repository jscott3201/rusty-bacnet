//! The receiver's fail-closed paths (RB-07, #2646): a segment from another
//! trust context, a segment through another router endpoint, and a
//! requester's Abort across contexts. The contexts are real: a relayed
//! origin from an SC hub over a loopback WebSocket, and router endpoints
//! from a running router's local delivery.
use super::*;
use bacnet_encoding::apdu::{Apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_network::layer::ReceivedApdu;
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{ScFunction, ScMessage, decode_sc_message, encode_sc_message};
use bacnet_transport::test_transport::{SendMode, TestTransport};
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::BytesMut;
use tokio::sync::mpsc;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(3);

fn segment(sequence: u8, more: bool) -> ConfirmedRequest {
    ConfirmedRequest {
        segmented: true,
        more_follows: more,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 9,
        sequence_number: Some(sequence),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        service_request: Bytes::from_static(&[0x09, 0x01]),
    }
}

fn receive(
    table: &mut SegmentedRequests,
    mac: &[u8],
    network: Option<&NpduAddress>,
    route: &ResponseRoute,
    request: &ConfirmedRequest,
) -> Receipt {
    table.receive(
        Segment {
            source_mac: mac,
            source_network: network,
            route,
            request,
        },
        Instant::now(),
    )
}

/// The provenance an SC hub's relayed NPDU carries: verified, not direct.
async fn relayed_origin() -> TransportProvenance {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    let hub = tokio::spawn(async move {
        let request = decode_sc_message(&ws_hub.recv().await.unwrap()).unwrap();
        let mut payload = Vec::with_capacity(26);
        payload.extend_from_slice(&[0x10; 6]);
        payload.extend_from_slice(&[0x33; 16]);
        payload.extend_from_slice(&1476u16.to_be_bytes());
        payload.extend_from_slice(&1476u16.to_be_bytes());
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: request.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &accept);
        ws_hub.send(&buf).await.unwrap();
        ws_hub
    });
    let mut rx = transport.start().await.unwrap();
    let ws_hub = hub.await.unwrap();
    let relayed = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2233,
        originating_vmac: Some([0x22; 6]),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &relayed);
    ws_hub.send(&buf).await.unwrap();
    let received = timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    transport.stop().await.unwrap();
    assert!(received.provenance.is_relayed_origin());
    assert!(!received.provenance.is_direct_peer());
    received.provenance
}

#[tokio::test]
async fn a_segment_from_another_trust_context_ends_the_transfer() {
    let relayed = ResponseRoute::new(relayed_origin().await, None);
    let unverified = ResponseRoute::unverified();
    let mut table = SegmentedRequests::new(Duration::from_secs(1));
    assert!(matches!(
        receive(&mut table, &[1], None, &unverified, &segment(0, true)),
        Receipt::Ack(_)
    ));
    // The same peer and invoke ID in a verified context: neither is merged
    // with the other.
    let Receipt::Abort(reason) = receive(&mut table, &[1], None, &relayed, &segment(1, true))
    else {
        panic!("expected an Abort");
    };
    assert_eq!(reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));
}

#[tokio::test]
async fn a_requesters_abort_ends_its_transfer_in_every_non_direct_context() {
    let relayed = ResponseRoute::new(relayed_origin().await, None);
    let mut table = SegmentedRequests::new(Duration::from_secs(1));
    assert!(matches!(
        receive(&mut table, &[1], None, &relayed, &segment(0, true)),
        Receipt::Ack(_)
    ));
    table.abort(&[1], None, 9, TransportProvenance::unverified());
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));
}

/// A router with three networks, whose own application takes what is
/// addressed to its port on network 2000 or 3000.
struct Routed {
    _router: BACnetRouter,
    frames: mpsc::Sender<ReceivedNpdu>,
    local: mpsc::Receiver<ReceivedApdu>,
}

impl Routed {
    async fn new() -> Self {
        let mut frames = None;
        let mut ports = Vec::new();
        for (number, mac) in [(1000, 1), (2000, 2), (3000, 3)] {
            let (tx, rx) = mpsc::channel(8);
            frames.get_or_insert(tx);
            let transport = TestTransport::builder()
                .local_mac(&[mac])
                .inbound(rx)
                .broadcast(SendMode::Ignore)
                .build();
            ports.push(RouterPort::new(transport, number));
        }
        let started = BACnetRouter::start(ports, RouterOptions::new())
            .await
            .unwrap();
        Self {
            _router: started.router,
            frames: frames.unwrap(),
            local: started.apdus,
        }
    }

    /// `request` from network 9000's station 7, through station 9 on
    /// network 1000, to the router's port on `network`.
    async fn deliver(&mut self, network: u16, request: &ConfirmedRequest) -> ReceivedApdu {
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, &Apdu::ConfirmedRequest(request.clone())).unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                source: Some(NpduAddress {
                    network: 9000,
                    mac_address: MacAddr::from_slice(&[7]),
                }),
                destination: Some(NpduAddress {
                    network,
                    mac_address: MacAddr::from_slice(&[(network / 1000) as u8]),
                }),
                expecting_reply: true,
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.frames
            .send(ReceivedNpdu::unverified(
                npdu.freeze(),
                MacAddr::from_slice(&[9]),
                false,
                vec![],
                None,
            ))
            .await
            .unwrap();
        timeout(WAIT, self.local.recv()).await.unwrap().unwrap()
    }
}

#[tokio::test]
async fn a_segment_through_another_router_endpoint_retires_the_transfer() {
    let mut routed = Routed::new().await;
    let mut table = SegmentedRequests::new(Duration::from_secs(1));
    let first = routed.deliver(2000, &segment(0, true)).await;
    let first_route = first.response_route();
    assert!(matches!(
        receive(
            &mut table,
            &first.source_mac,
            first.source_network.as_ref(),
            &first_route,
            &segment(0, true),
        ),
        Receipt::Ack(_)
    ));
    let moved = routed.deliver(3000, &segment(1, true)).await;
    let moved_route = moved.response_route();
    assert!(!first_route.same_origin(&moved_route));
    let Receipt::Retired(opening) = receive(
        &mut table,
        &moved.source_mac,
        moved.source_network.as_ref(),
        &moved_route,
        &segment(1, true),
    ) else {
        panic!("expected the transfer retired");
    };
    // Its Abort goes back the way the first segment came.
    assert!(opening.route.same_origin(&first_route));
    assert_eq!(opening.invoke_id, 9);
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));
    // Nothing is left for a later segment to join.
    let again = routed.deliver(2000, &segment(1, true)).await;
    assert!(matches!(
        receive(
            &mut table,
            &again.source_mac,
            again.source_network.as_ref(),
            &again.response_route(),
            &segment(1, true),
        ),
        Receipt::Abort(AbortReason::INVALID_APDU_IN_THIS_STATE)
    ));
}

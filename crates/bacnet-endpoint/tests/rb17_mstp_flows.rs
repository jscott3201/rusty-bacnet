//! RB-17 MS/TP flows: release, bounds, cancellation, stop, non-routing.
//!
//! Split from `rb17_mstp_proof.rs` (file-size gate). Same harness
//! (`support/rb17_harness.rs`: session-under-test + raw-frame peer over
//! [`LoopbackSerial::pair`](bacnet_transport::mstp::LoopbackSerial), both
//! execution modes, standard and extended frames, deterministic, no
//! sleeps), proving: dropped reply sender releases without late bytes;
//! token/PFM traffic during application work preserves postponed
//! ownership; queue/admission bounds release (MAC `WouldBlock`,
//! egress-full, layer full-drop); cancellation and stop with an outstanding
//! request; responder denial releases the one-use sender; non-routing
//! discard.
//!
//! Evidence level: simulator (LoopbackSerial), NOT physical bench or
//! on-wire conformance; timing qualification is RB-26, routing is RB-25.

#[path = "support/mstp_wire.rs"]
mod mstp_wire;
#[path = "support/rb17_harness.rs"]
mod rb17_harness;

use std::sync::Arc;

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_endpoint::session::SessionRole;
use bacnet_endpoint_core::endpoint_ingress::{EndpointApduDestination, EndpointIngress};
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::__endpoint_EndpointResponder as EndpointResponder;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::mstp::{
    LoopbackSerial, MstpConfig, MstpExecutionMode, MstpTransport, SerialPort,
};
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_types::enums::{NetworkPriority, ObjectType, PropertyIdentifier};
use bacnet_types::{error::Error, MacAddr};
use bytes::{Bytes, BytesMut};
use mstp_wire::{pfm_frame, token_frame, FrameType, MstpFrame};
use rb17_harness::*;
use tokio::sync::{mpsc, oneshot, RwLock};

async fn dropped_sender_impl(mode: MstpExecutionMode, frames: Frames) {
    // Client-only session has no server role: the lone reply sender is
    // released without bytes, yet the MAC still answers ReplyPostponed.
    let mut h = Harness::new(mode, SessionRole::ClientOnly, frames).await;
    assert!(h.session.server().is_none());
    h.peer_send(&data_expecting(1, 44)).await;
    let postponed = h
        .wait_for("ReplyPostponed", |f| {
            f.frame_type == FrameType::REPLY_POSTPONED && f.destination == PEER_MAC
        })
        .await;
    assert_eq!(postponed.destination, PEER_MAC);
    let counters = h.session.policy_counters().await;
    assert!(
        counters.no_server_role >= 1,
        "lost inbound must be policy-owned: {counters:?}"
    );
    // A token opportunity must NOT produce a phantom late response; the
    // session answers the token itself (unknown successor → PFM).
    h.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
    let mut saw_token_use = false;
    for _ in 0..4 {
        let frame = h.next_frame().await;
        assert_ne!(
            frame.frame_type,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            "released reply must never produce late bytes"
        );
        if matches!(
            frame.frame_type,
            FrameType::POLL_FOR_MASTER | FrameType::TOKEN
        ) {
            saw_token_use = true;
            break;
        }
    }
    assert!(saw_token_use, "token must still be answered after release");
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_dropped_sender_releases_tokio() {
    dropped_sender_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_dropped_sender_releases_dedicated() {
    dropped_sender_impl(MstpExecutionMode::DedicatedThread, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_dropped_sender_releases_extended_tokio() {
    dropped_sender_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_dropped_sender_releases_extended_dedicated() {
    dropped_sender_impl(MstpExecutionMode::DedicatedThread, Frames::Extended).await;
}

async fn token_during_work_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = Harness::new(mode, SessionRole::Both, frames).await;
    h.session
        .server()
        .expect("server role")
        .suspend_next_reply()
        .expect("arm suspension");
    h.peer_send(&data_expecting(1, 55)).await;
    // Token + PFM arrive while the application works (order vs dispatch is
    // deliberately uncontrolled). Neither may break the pending reply
    // ownership: ReplyPostponed still goes to the requester. The core keeps
    // the Token for after the reply and drops the poll, which it could only
    // answer late (Clause 9.5.6.9 leaves both pending while answering).
    h.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
    h.peer_send(&pfm_frame(SESSION_MAC, PEER_MAC)).await;
    let mut postponed = false;
    let mut early_data: Option<MstpFrame> = None;
    for _ in 0..16 {
        let frame = h.next_frame().await;
        match frame.frame_type {
            FrameType::REPLY_POSTPONED if frame.destination == PEER_MAC => postponed = true,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY if frame.destination == PEER_MAC => {
                early_data = Some(frame);
            }
            _ => {}
        }
        if postponed {
            break;
        }
    }
    assert!(
        postponed,
        "MAC must release ReplyPostponed during application work"
    );
    // Ownership preserved: the token opportunity delivers the postponed
    // response to the original requester (possibly already observed above).
    match early_data {
        Some(frame) => expect_present_value(&frame, 55, 11.0),
        None => {
            let late = h
                .drive_token_until_data("deferred ComplexAck", |f| {
                    f.frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                        && f.destination == PEER_MAC
                })
                .await;
            expect_present_value(&late, 55, 11.0);
        }
    }
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_token_pfm_during_work_tokio() {
    token_during_work_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_token_pfm_during_work_dedicated() {
    token_during_work_impl(MstpExecutionMode::DedicatedThread, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_token_pfm_during_work_extended_tokio() {
    token_during_work_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_token_pfm_during_work_extended_dedicated() {
    token_during_work_impl(MstpExecutionMode::DedicatedThread, Frames::Extended).await;
}

/// Serializes direction-blind sends: every send blocks forever, so the
/// endpoint session task stalls inside its first network send and the
/// bounded egress command channel must reject instead of growing.
/// `send_started` fires on send entry so the test can prove the session
/// task consumed the first command (and is stalled) before filling the
/// single free slot — no scheduling assumption.
struct StallTransport {
    tx: Option<mpsc::Sender<ReceivedNpdu>>,
    send_started: Arc<std::sync::atomic::AtomicBool>,
}

impl StallTransport {
    fn new() -> (Self, Arc<std::sync::atomic::AtomicBool>) {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        (
            Self {
                tx: None,
                send_started: Arc::clone(&flag),
            },
            flag,
        )
    }
}

impl TransportPort for StallTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let (tx, rx) = mpsc::channel(8);
        self.tx = Some(tx);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        self.send_started
            .store(true, std::sync::atomic::Ordering::Release);
        std::future::pending::<()>().await;
        #[allow(unreachable_code)]
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        std::future::pending::<()>().await;
        #[allow(unreachable_code)]
        Ok(())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[0x09]
    }
}

#[tokio::test]
async fn mstp_queue_and_admission_bounds_release() {
    // (a) MAC TX queue: 256 unicast NPDUs deep; the 257th send is
    // WouldBlock, and an oversized NPDU is rejected at the standard-frame
    // seam (both release without sending). Nobody gives the station the
    // token, so nothing leaves the queue.
    let (serial, _peer) = LoopbackSerial::pair();
    let mut transport = MstpTransport::new(
        serial,
        MstpConfig {
            this_station: SESSION_MAC,
            max_master: 7,
            max_info_frames: 1,
            baud_rate: 9600,
            ..MstpConfig::default()
        },
    );
    let _npdus = transport.start().await.expect("transport must start");
    for _ in 0..256 {
        transport
            .send_unicast(&[0x01, 0x00, 0x10], &[PEER_MAC])
            .await
            .unwrap();
    }
    let full = transport
        .send_unicast(&[0x01], &[PEER_MAC])
        .await
        .unwrap_err();
    assert!(
        full.to_string().contains("TX queue full"),
        "queue-full must be WouldBlock: {full}"
    );
    let big = transport
        .send_unicast(&[0u8; 502], &[PEER_MAC])
        .await
        .unwrap_err();
    assert!(
        big.to_string().contains("exceeds standard-frame maximum"),
        "oversized NPDU must be rejected: {big}"
    );
    transport.stop().await.expect("transport must stop");

    // (b) Endpoint egress (capacity 1, session task stalled in StallTransport):
    // the first send occupies the task (proven via `send_started`, not
    // scheduling); the second fills the single slot; the third is rejected
    // as full instead of hanging, and stop releases the stalled waiters.
    let (stall, send_started) = StallTransport::new();
    let mut ingress = EndpointIngress::new(stall, 1);
    let receivers = ingress.start().await.expect("ingress must start");
    let egress = receivers.egress;
    let send_one = |egress: bacnet_endpoint_core::endpoint_ingress::EndpointEgress| async move {
        egress
            .send_apdu(
                vec![0x10, 0x08],
                EndpointApduDestination::Direct {
                    destination_mac: MacAddr::from_slice(&[0x09]),
                },
                false,
                NetworkPriority::NORMAL,
                Vec::<DataAttribute>::new(),
            )
            .await
    };
    let s1 = tokio::spawn(send_one(egress.clone()));
    // Prove the session task consumed the first command and stalled inside
    // the network send (event-driven, no sleep): only then is the race below
    // independent of task scheduling.
    let deadline = tokio::time::Instant::now() + WAIT;
    while !send_started.load(std::sync::atomic::Ordering::Acquire) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "session task never consumed the first egress command"
        );
        tokio::task::yield_now().await;
    }
    // Two more sends race for the single free slot while the task is proven
    // stalled: exactly one buffers, the other is Full immediately (no
    // consumer progress is possible until stop). The Full one resolves
    // without any stop; the buffered one cannot finish before it — so the
    // first finished handle is the Full proof, deterministically.
    let s2 = tokio::spawn(send_one(egress.clone()));
    let s3 = tokio::spawn(send_one(egress));
    let race_deadline = tokio::time::Instant::now() + WAIT;
    while !(s2.is_finished() || s3.is_finished()) {
        assert!(
            tokio::time::Instant::now() < race_deadline,
            "egress race never settled"
        );
        tokio::task::yield_now().await;
    }
    ingress.stop().await.expect("ingress must stop");
    let mut full = 0;
    let mut shutdown = 0;
    for stalled in [s1, s2, s3] {
        let outcome = tokio::time::timeout(WAIT, stalled)
            .await
            .expect("egress send must resolve after stop")
            .expect("join failed");
        match outcome {
            Err(error) if error.to_string().contains("egress queue is full") => full += 1,
            Err(_) => shutdown += 1,
            Ok(()) => panic!("stalled egress send must not succeed"),
        }
    }
    assert_eq!(
        (full, shutdown),
        (1, 2),
        "one Full rejection + two fail-closed releases"
    );

    // (c) Network-layer receive bound (256, raw receiver): 300 arrivals from
    // one peer all send without blocking; exactly the bound is queued and the
    // excess drops without a hang.
    let (endpoint_side, mut peer_side) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut layer = NetworkLayer::new(endpoint_side);
    let mut apdus = layer.start().await.expect("layer must start");
    let _peer_rx = peer_side.start().await.unwrap();
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: None,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        },
    )
    .unwrap();
    let bytes = buf.freeze();
    for _ in 0..300 {
        peer_side
            .send_unicast(&bytes, &[0x01])
            .await
            .expect("peer send must not block on a full layer queue");
    }
    for _ in 0..256 {
        tokio::time::timeout(WAIT, apdus.recv())
            .await
            .expect("bound item missing")
            .expect("layer closed");
    }
    layer.stop().await.unwrap();
    peer_side.stop().await.unwrap();
}

async fn cancel_impl(mode: MstpExecutionMode, frames: Frames) {
    let h = Harness::new(mode, SessionRole::Both, frames).await;
    let client = h.session.cloned_client_handle().expect("client role");
    let pending = tokio::spawn(async move {
        client
            .read_property(
                &[PEER_MAC],
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    // The lease is held from reserve (before any token): poll to 1, cancel,
    // then prove the exact lease is released.
    let deadline = tokio::time::Instant::now() + WAIT;
    while h.session.active_leases() != 1 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "outbound lease never held"
        );
        tokio::task::yield_now().await;
    }
    pending.abort();
    let _ = pending.await;
    while h.session.active_leases() != 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "cancelled request stranded its lease"
        );
        tokio::task::yield_now().await;
    }
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_cancel_releases_lease_tokio() {
    cancel_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_cancel_releases_lease_dedicated() {
    cancel_impl(MstpExecutionMode::DedicatedThread, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_cancel_releases_lease_extended_tokio() {
    cancel_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_cancel_releases_lease_extended_dedicated() {
    cancel_impl(MstpExecutionMode::DedicatedThread, Frames::Extended).await;
}

async fn stop_outstanding_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = Harness::new(mode, SessionRole::Both, frames).await;
    let client = h.session.cloned_client_handle().expect("client role");
    let pending = tokio::spawn(async move {
        client
            .read_property(
                &[PEER_MAC],
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let deadline = tokio::time::Instant::now() + WAIT;
    while h.session.active_leases() != 1 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "outbound lease never held"
        );
        tokio::task::yield_now().await;
    }
    // Stop seals admission and releases the exact lease; the waiter observes
    // shutdown, never a hang or a success.
    h.session
        .stop()
        .await
        .expect("stop with outstanding request");
    assert_eq!(h.session.active_leases(), 0);
    let outcome = tokio::time::timeout(WAIT, pending)
        .await
        .expect("stop hung");
    match outcome {
        Ok(Err(_)) => {}
        Ok(Ok(_)) => panic!("pending request must not succeed after stop"),
        Err(_) => panic!("pending join must resolve after stop"),
    }
    h.drain.abort();
}

#[tokio::test]
async fn mstp_stop_with_outstanding_tokio() {
    stop_outstanding_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_stop_with_outstanding_dedicated() {
    stop_outstanding_impl(MstpExecutionMode::DedicatedThread, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_stop_with_outstanding_extended_tokio() {
    stop_outstanding_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_stop_with_outstanding_extended_dedicated() {
    stop_outstanding_impl(MstpExecutionMode::DedicatedThread, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_responder_denial_releases_reply() {
    // Denial (closed responder) releases the one-use reply sender without
    // bytes: the receiver observes closure and the peer sees no traffic.
    let (endpoint_side, mut peer_side) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut peer_rx = peer_side.start().await.unwrap();
    let mut ingress = EndpointIngress::new(endpoint_side, 2);
    let receivers = ingress.start().await.unwrap();
    let responder = EndpointResponder::new(
        Arc::new(RwLock::new(ObjectDatabase::new())),
        receivers.egress,
    );
    responder.close();
    let (reply_tx, reply_rx) = oneshot::channel();
    let err = responder
        .handle(ReceivedApdu {
            direct_response: None,
            apdu: read_request_apdu(7, 1),
            source_mac: MacAddr::from_slice(&[0x02]),
            ingress_network: None,
            source_network: None,
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: Some(reply_tx),
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("endpoint shutdown"));
    assert!(reply_rx.await.is_err(), "denial must release the sender");
    assert!(peer_rx.try_recv().is_err(), "denial must send no bytes");
    ingress.stop().await.unwrap();
    peer_side.stop().await.unwrap();
}

async fn nonrouting_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = Harness::new(mode, SessionRole::Both, frames).await;
    // Non-decodable bytes (no preamble): assembly discards them and the link
    // resynchronizes on the next preamble.
    h.peer.write(&[0xAA, 0xAA, 0xAA, 0x55]).await.unwrap();
    // Routed DNET request: the non-routing network layer discards it (no
    // application reply), while the MAC still owns the reply channel and
    // releases ReplyPostponed.
    let mut routed = BytesMut::new();
    encode_npdu(
        &mut routed,
        &Npdu {
            is_network_message: false,
            expecting_reply: true,
            priority: NetworkPriority::NORMAL,
            destination: Some(NpduAddress {
                network: 77,
                mac_address: MacAddr::from_slice(&[0x44]),
            }),
            source: None,
            hop_count: 255,
            payload: read_request_apdu(77, 1),
            ..Npdu::default()
        },
    )
    .unwrap();
    h.peer_send(&MstpFrame {
        frame_type: FrameType::BACNET_DATA_EXPECTING_REPLY,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: routed.freeze(),
    })
    .await;
    h.wait_for("postponed for discarded routed", |f| {
        f.frame_type == FrameType::REPLY_POSTPONED && f.destination == PEER_MAC
    })
    .await;
    // A valid local request on the same link is still served; the routed
    // invoke ID must never draw an application reply.
    h.peer_send(&data_expecting(1, 66)).await;
    let mut routed_answered = false;
    let reply = h
        .wait_for("local ComplexAck", |f| {
            if f.frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
                if let Ok(npdu) = decode_npdu(f.data.clone()) {
                    if let Ok(Apdu::ComplexAck(ack)) = decode_apdu(npdu.payload) {
                        if ack.invoke_id == 77 {
                            routed_answered = true;
                        }
                        if ack.invoke_id == 66 {
                            return true;
                        }
                    }
                }
            }
            false
        })
        .await;
    assert!(!routed_answered, "routed DNET traffic must draw no reply");
    expect_present_value(&reply, 66, 11.0);
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_nonrouting_standard_only_tokio() {
    nonrouting_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_nonrouting_standard_only_dedicated() {
    nonrouting_impl(MstpExecutionMode::DedicatedThread, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_nonrouting_extended_tokio() {
    nonrouting_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_nonrouting_extended_dedicated() {
    nonrouting_impl(MstpExecutionMode::DedicatedThread, Frames::Extended).await;
}

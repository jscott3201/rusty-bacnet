//! RB-17 MS/TP flows: release, bounds, cancellation, stop, non-routing.
//!
//! Split from `rb17_mstp_proof.rs` (file-size gate). Same harness shape
//! (session-under-test + raw-frame peer over [`LoopbackSerial::pair`](bacnet_transport::mstp::LoopbackSerial),
//! both execution modes, deterministic, no sleeps), proving: dropped reply
//! sender releases without late bytes; token/PFM traffic during application
//! work preserves postponed ownership; queue/admission bounds release
//! (MAC `WouldBlock`, egress-full, layer full-drop); cancellation and stop
//! with an outstanding request; responder denial releases the one-use
//! sender; non-routing discard + standard-frames-only behavior; builder
//! addressing/APDU-bound validation.
//!
//! Evidence level: simulator (LoopbackSerial), NOT physical bench or
//! on-wire conformance; timing qualification is RB-26, routing is RB-25.

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest as ConfirmedPdu};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_endpoint_core::endpoint_ingress::{EndpointApduDestination, EndpointIngress};
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_server::server::__endpoint_EndpointResponder as EndpointResponder;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::mstp::{
    LoopbackSerial, MasterNode, MstpConfig, MstpExecutionMode, MstpTransport, SerialPort,
};
use bacnet_transport::mstp_frame::{decode_frame, encode_frame, FrameType, MstpFrame};
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier, Segmentation,
    ServiceSupported,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::{error::Error, MacAddr};
use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot, RwLock};

const WAIT: Duration = Duration::from_secs(5);
const SESSION_MAC: u8 = 3;
const PEER_MAC: u8 = 7;

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(2001, 42)
        .unwrap()
        .with_max_apdu(480)
        .unwrap()
        .with_segmentation(Segmentation::NONE)
        .with_services(&[ServiceSupported::READ_PROPERTY])
}

fn db_with_analog(id: &DeviceIdentity, instance: u32, value: f32) -> ObjectDatabase {
    let mut o = AnalogInputObject::new(instance, format!("mstp-ai-{instance}"), 0).unwrap();
    o.set_present_value(value);
    build_database_with_extra(id, vec![Box::new(o)]).unwrap()
}

struct ObservedSerial {
    inner: LoopbackSerial,
    writes: mpsc::UnboundedSender<MstpFrame>,
}

impl SerialPort for ObservedSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        let (frame, consumed) = decode_frame(data)?;
        assert_eq!(consumed, data.len(), "one frame per MS/TP write");
        self.writes
            .send(frame)
            .expect("write observer must stay attached");
        self.inner.write(data).await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.inner.read(buf).await
    }
}

struct Harness {
    session: EndpointSession<MstpTransport<ObservedSerial>>,
    peer: Arc<LoopbackSerial>,
    writes: mpsc::UnboundedReceiver<MstpFrame>,
    drain: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(mode: MstpExecutionMode, role: SessionRole) -> Self {
        let (session_end, peer_end) = LoopbackSerial::pair();
        let (write_tx, writes) = mpsc::unbounded_channel();
        let observed = ObservedSerial {
            inner: session_end,
            writes: write_tx,
        };
        let id = identity();
        let db = db_with_analog(&id, 1, 11.0);
        let mut session = MstpEndpointBuilder::new(observed, SESSION_MAC)
            .max_master(7)
            .execution_mode(mode)
            .role(role)
            .queue_capacity(32)
            .client_timers(2_000, 0)
            .database(db)
            .identity(id)
            .build_session()
            .expect("mstp session must build");
        session.start().await.expect("mstp session must start");
        let peer = Arc::new(peer_end);
        let drain_peer = Arc::clone(&peer);
        let drain = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while drain_peer.read(&mut buf).await.is_ok() {}
        });
        Self {
            session,
            peer,
            writes,
            drain,
        }
    }

    async fn next_frame(&mut self) -> MstpFrame {
        let frame = tokio::time::timeout(WAIT, self.writes.recv())
            .await
            .expect("session frame timed out")
            .expect("writes closed");
        assert_eq!(
            frame.source, SESSION_MAC,
            "one link owner: every session frame carries the session station"
        );
        frame
    }

    async fn wait_for(
        &mut self,
        what: &str,
        mut accept: impl FnMut(&MstpFrame) -> bool,
    ) -> MstpFrame {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for session {what}");
            let frame = tokio::time::timeout(remaining, self.writes.recv())
                .await
                .expect("session frame timed out")
                .expect("writes closed");
            assert_eq!(
                frame.source, SESSION_MAC,
                "one link owner: every session frame carries the session station"
            );
            if accept(&frame) {
                return frame;
            }
        }
    }

    async fn peer_send(&self, frame: &MstpFrame) {
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, frame).expect("valid test frame");
        self.peer
            .write(&buf)
            .await
            .expect("peer write must succeed");
    }

    async fn drive_token_until_data(
        &mut self,
        what: &str,
        mut is_data: impl FnMut(&MstpFrame) -> bool,
    ) -> MstpFrame {
        for _ in 0..8 {
            self.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
            let frame = self
                .wait_for(what, |f| {
                    is_data(f)
                        || matches!(
                            f.frame_type,
                            FrameType::PollForMaster | FrameType::Token | FrameType::ReplyPostponed
                        )
                })
                .await;
            if is_data(&frame) {
                return frame;
            }
        }
        panic!("token drive did not deliver {what}");
    }

    async fn shutdown(mut self) {
        self.session.stop().await.expect("session must stop");
        self.drain.abort();
    }
}

fn token_frame(dest: u8, src: u8) -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::Token,
        destination: dest,
        source: src,
        data: Bytes::new(),
    }
}

fn pfm_frame(dest: u8, src: u8) -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::PollForMaster,
        destination: dest,
        source: src,
        data: Bytes::new(),
    }
}

fn read_request_apdu(invoke_id: u8, instance: u32) -> Bytes {
    let mut svc = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, instance),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut svc);
    let apdu = Apdu::ConfirmedRequest(ConfirmedPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: svc.freeze(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &apdu).unwrap();
    encoded.freeze()
}

fn read_request_npdu(invoke_id: u8, instance: u32) -> Bytes {
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            is_network_message: false,
            expecting_reply: true,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: None,
            payload: read_request_apdu(invoke_id, instance),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.freeze()
}

fn data_expecting(instance: u32, invoke_id: u8) -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: read_request_npdu(invoke_id, instance),
    }
}

fn expect_present_value(frame: &MstpFrame, invoke_id: u8, value: f32) {
    let npdu = decode_npdu(frame.data.clone()).expect("response frame carries NPDU");
    match decode_apdu(npdu.payload).expect("response carries APDU") {
        Apdu::ComplexAck(ack) => {
            assert_eq!(
                ack.invoke_id, invoke_id,
                "postponed identity: invoke ID preserved"
            );
            let decoded = ReadPropertyACK::decode(&ack.service_ack).unwrap();
            let mut expect = BytesMut::new();
            encode_property_value(&mut expect, &PropertyValue::Real(value)).unwrap();
            assert_eq!(decoded.property_value, expect.to_vec());
        }
        other => panic!("expected ComplexAck, got {other:?}"),
    }
}

async fn dropped_sender_impl(mode: MstpExecutionMode) {
    // Client-only session has no server role: the lone reply sender is
    // released without bytes, yet the MAC still answers ReplyPostponed.
    let mut h = Harness::new(mode, SessionRole::ClientOnly).await;
    assert!(h.session.server().is_none());
    h.peer_send(&data_expecting(1, 44)).await;
    let postponed = h
        .wait_for("ReplyPostponed", |f| {
            f.frame_type == FrameType::ReplyPostponed && f.destination == PEER_MAC
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
            FrameType::BACnetDataNotExpectingReply,
            "released reply must never produce late bytes"
        );
        if matches!(
            frame.frame_type,
            FrameType::PollForMaster | FrameType::Token
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
    dropped_sender_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_dropped_sender_releases_dedicated() {
    dropped_sender_impl(MstpExecutionMode::DedicatedThread).await;
}

async fn token_during_work_impl(mode: MstpExecutionMode) {
    let mut h = Harness::new(mode, SessionRole::Both).await;
    h.session
        .server()
        .expect("server role")
        .suspend_next_reply()
        .expect("arm suspension");
    h.peer_send(&data_expecting(1, 55)).await;
    // Token + PFM arrive while the application works (order vs dispatch is
    // deliberately uncontrolled); both must be honored without breaking the
    // pending reply ownership.
    h.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
    h.peer_send(&pfm_frame(SESSION_MAC, PEER_MAC)).await;
    let mut postponed = false;
    let mut pfm_reply = false;
    let mut early_data: Option<MstpFrame> = None;
    for _ in 0..16 {
        let frame = h.next_frame().await;
        match frame.frame_type {
            FrameType::ReplyPostponed if frame.destination == PEER_MAC => postponed = true,
            FrameType::ReplyToPollForMaster if frame.destination == PEER_MAC => pfm_reply = true,
            FrameType::BACnetDataNotExpectingReply if frame.destination == PEER_MAC => {
                early_data = Some(frame);
            }
            _ => {}
        }
        if postponed && pfm_reply {
            break;
        }
    }
    assert!(
        postponed,
        "MAC must release ReplyPostponed during application work"
    );
    assert!(pfm_reply, "PFM must be answered during application work");
    // Ownership preserved: the token opportunity delivers the postponed
    // response to the original requester (possibly already observed above).
    match early_data {
        Some(frame) => expect_present_value(&frame, 55, 11.0),
        None => {
            let late = h
                .drive_token_until_data("deferred ComplexAck", |f| {
                    f.frame_type == FrameType::BACnetDataNotExpectingReply
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
    token_during_work_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_token_pfm_during_work_dedicated() {
    token_during_work_impl(MstpExecutionMode::DedicatedThread).await;
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

    fn local_mac(&self) -> &[u8] {
        &[0x09]
    }
}

#[tokio::test]
async fn mstp_queue_and_admission_bounds_release() {
    // (a) MAC TX queue: 256-deep; the 257th send is WouldBlock, and an
    // oversized NPDU is rejected at the standard-frame seam (both release
    // without sending).
    let mut node = MasterNode::new(MstpConfig {
        this_station: SESSION_MAC,
        max_master: 7,
        max_info_frames: 1,
        baud_rate: 9600,
    })
    .unwrap();
    for _ in 0..256 {
        node.queue_npdu(PEER_MAC, Bytes::from_static(&[0x01, 0x00, 0x10]))
            .unwrap();
    }
    let full = node
        .queue_npdu(PEER_MAC, Bytes::from_static(&[0x01]))
        .unwrap_err();
    assert!(
        full.to_string().contains("TX queue full"),
        "queue-full must be WouldBlock: {full}"
    );
    let big = node
        .queue_npdu(PEER_MAC, Bytes::from(vec![0u8; 502]))
        .unwrap_err();
    assert!(
        big.to_string().contains("exceeds standard-frame maximum"),
        "oversized NPDU must be rejected: {big}"
    );

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

async fn cancel_impl(mode: MstpExecutionMode) {
    let h = Harness::new(mode, SessionRole::Both).await;
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
    cancel_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_cancel_releases_lease_dedicated() {
    cancel_impl(MstpExecutionMode::DedicatedThread).await;
}

async fn stop_outstanding_impl(mode: MstpExecutionMode) {
    let mut h = Harness::new(mode, SessionRole::Both).await;
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
    stop_outstanding_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_stop_with_outstanding_dedicated() {
    stop_outstanding_impl(MstpExecutionMode::DedicatedThread).await;
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
            apdu: read_request_apdu(7, 1),
            source_mac: MacAddr::from_slice(&[0x02]),
            ingress_network: None,
            source_network: None,
            link_layer_group: false,
            is_group: false,
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

async fn nonrouting_impl(mode: MstpExecutionMode) {
    let mut h = Harness::new(mode, SessionRole::Both).await;
    // Non-decodable bytes (no preamble): assembly discards them and the link
    // resynchronizes on the next preamble (standard frames only).
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
        frame_type: FrameType::BACnetDataExpectingReply,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: routed.freeze(),
    })
    .await;
    h.wait_for("postponed for discarded routed", |f| {
        f.frame_type == FrameType::ReplyPostponed && f.destination == PEER_MAC
    })
    .await;
    // A valid local request on the same link is still served; the routed
    // invoke ID must never draw an application reply.
    h.peer_send(&data_expecting(1, 66)).await;
    let mut routed_answered = false;
    let reply = h
        .wait_for("local ComplexAck", |f| {
            if f.frame_type == FrameType::BACnetDataNotExpectingReply {
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
    nonrouting_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_nonrouting_standard_only_dedicated() {
    nonrouting_impl(MstpExecutionMode::DedicatedThread).await;
}

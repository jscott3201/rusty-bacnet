//! RB-17 MS/TP proof: one-link endpoint over simulated serial, one owner.
//!
//! Session-under-test (via [`MstpEndpointBuilder`](bacnet_endpoint::mstp::MstpEndpointBuilder))
//! + raw-frame peer harness over [`LoopbackSerial::pair`](bacnet_transport::mstp::LoopbackSerial).
//! Every logical scenario runs in BOTH `Tokio` and `DedicatedThread`
//! execution modes with the same assertions. Deterministic and event-driven
//! (timeout-guarded channel receives, no sleeps).
//!
//! Evidence level (simulator, NOT physical bench or on-wire conformance):
//! this file proves single serial ownership + start-once, prompt replies via
//! the responder `reply_tx` path with no preceding `ReplyPostponed`,
//! bidirectional confirmed traffic, and postponed → later token-owned
//! responses with the postponed identity (destination + invoke ID + value)
//! preserved. Timing qualification is RB-26; extended-frame/router work is
//! RB-25. Companion `rb17_mstp_flows.rs` holds release/queue/cancellation/
//! stop/denial/non-routing proofs (file-size gate).
//!
//! Reply-path shape under test: prompt replies use the one-use `reply_tx`
//! (MAC `AnswerDataRequest` window, no token needed); token-owned sends
//! (client initiations, deferred-after-postponed) go through the single
//! `EndpointEgress` → `queue_npdu` path as `DataNotExpectingReply`. The peer
//! drives token opportunities by injecting `Token` frames addressed to the
//! session station (simulator control, not conformance traffic); the MAC's
//! own self-organization is the backstop, never the primary driver.

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, Apdu, ComplexAck, ConfirmedRequest as ConfirmedPdu,
};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, PolicyCountersSnapshot, SessionRole};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::mstp::{LoopbackSerial, MstpExecutionMode, MstpTransport, SerialPort};
use bacnet_transport::mstp_frame::{decode_frame, encode_frame, FrameType, MstpFrame};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier, Segmentation,
    ServiceSupported,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(5);
const SESSION_MAC: u8 = 3;
const PEER_MAC: u8 = 7;

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

fn identity() -> DeviceIdentity {
    // MS/TP composition keeps the 480 APDU bound (fits the 501 frame-data
    // cap); the builder rejects identities advertising more.
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

/// Serial wrapper recording every frame the session transmits (decoded).
///
/// Mirrors the `ObservedSerial` double in `mstp::port_timing_tests`: each
/// transport write must contain exactly one complete standard frame. The
/// peer harness drains the byte channel so session writes never stall on
/// loopback backpressure; assertions read the decoded stream here
/// (frame-category + destination ownership inspection).
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
            // Bounded queues + client timers mirror the RB-16 proof shape.
            .queue_capacity(32)
            .client_timers(2_000, 0)
            .database(db)
            .identity(id)
            .build_session()
            .expect("mstp session must build");
        session.start().await.expect("mstp session must start");
        let peer = Arc::new(peer_end);
        let drain_peer = Arc::clone(&peer);
        // Drain only prevents loopback backpressure; assertions read the
        // decoded `writes` stream, never these bytes.
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

    /// Collects session frames until `accept` matches (bounded, ownership-checked).
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

    /// Drives token opportunities until a data frame matching `is_data`
    /// arrives (bounded rounds, event-driven). Token answers without data
    /// (PFM/Token) and `ReplyPostponed` only trigger a retry: whichever of
    /// the egress-queue fill vs MAC reply wins the race, the outcome is the
    /// same postponed identity delivered token-owned.
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

fn complex_ack_npdu(invoke_id: u8, value: f32) -> Bytes {
    let mut val = BytesMut::new();
    encode_property_value(&mut val, &PropertyValue::Real(value)).unwrap();
    let ack = ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: val.to_vec(),
    };
    let mut svc = BytesMut::new();
    ack.encode(&mut svc);
    let apdu = Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: svc.freeze(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: None,
            payload: encoded.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.freeze()
}

fn decode_complex_ack(frame: &MstpFrame) -> (u8, ReadPropertyACK) {
    let npdu = decode_npdu(frame.data.clone()).expect("response frame carries NPDU");
    match decode_apdu(npdu.payload).expect("response carries APDU") {
        Apdu::ComplexAck(ack) => {
            let decoded = ReadPropertyACK::decode(&ack.service_ack)
                .expect("ComplexAck carries ReadPropertyACK");
            (ack.invoke_id, decoded)
        }
        other => panic!("expected ComplexAck, got {other:?}"),
    }
}

fn expect_present_value(frame: &MstpFrame, invoke_id: u8, value: f32) {
    let (iid, ack) = decode_complex_ack(frame);
    assert_eq!(iid, invoke_id, "postponed identity: invoke ID preserved");
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    let mut expect = BytesMut::new();
    encode_property_value(&mut expect, &PropertyValue::Real(value)).unwrap();
    assert_eq!(ack.property_value, expect.to_vec());
}

async fn one_owner_impl(mode: MstpExecutionMode) {
    // Transport-level identity: one MAC, 480 APDU bound, start-once.
    let (serial, _peer) = LoopbackSerial::pair();
    let mut transport = MstpEndpointBuilder::new(serial, SESSION_MAC)
        .max_master(7)
        .execution_mode(mode)
        .build_transport()
        .expect("transport must build");
    assert_eq!(transport.local_mac(), &[SESSION_MAC]);
    assert_eq!(transport.max_apdu_length(), 480);
    let _rx = transport.start().await.expect("transport must start");
    assert!(
        transport.start().await.is_err(),
        "start-once: no second MAC loop"
    );
    transport.stop().await.expect("transport must stop");

    // Session-level: the builder consumes the serial owner; start-once holds
    // and dropping the session releases the serial (peer observes closure).
    let (session_end, peer_end) = LoopbackSerial::pair();
    let (write_tx, writes) = mpsc::unbounded_channel();
    let observed = ObservedSerial {
        inner: session_end,
        writes: write_tx,
    };
    let id = identity();
    let mut session = MstpEndpointBuilder::new(observed, SESSION_MAC)
        .max_master(7)
        .execution_mode(mode)
        .database(db_with_analog(&id, 1, 1.0))
        .identity(id)
        .build_session()
        .expect("session must build");
    drop(writes);
    session.start().await.expect("session must start");
    assert!(session.is_running());
    assert!(session.start().await.is_err(), "session start-once");
    drop(session);
    let mut buf = [0u8; 64];
    let closed = tokio::time::timeout(WAIT, peer_end.read(&mut buf)).await;
    assert!(
        matches!(closed, Ok(Err(_))),
        "dropped session must release the serial owner"
    );
}

#[tokio::test]
async fn mstp_one_serial_owner_tokio() {
    one_owner_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_one_serial_owner_dedicated() {
    one_owner_impl(MstpExecutionMode::DedicatedThread).await;
}

async fn prompt_reply_impl(mode: MstpExecutionMode) {
    let mut h = Harness::new(mode, SessionRole::Both).await;
    h.peer_send(&data_expecting(1, 9)).await;
    // Prompt path: DataNotExpectingReply with NO ReplyPostponed before it.
    let mut saw_postponed = false;
    let reply = h
        .wait_for("prompt ComplexAck", |f| {
            if f.frame_type == FrameType::ReplyPostponed {
                saw_postponed = true;
            }
            f.frame_type == FrameType::BACnetDataNotExpectingReply && f.destination == PEER_MAC
        })
        .await;
    assert!(
        !saw_postponed,
        "prompt reply must not be preceded by ReplyPostponed"
    );
    expect_present_value(&reply, 9, 11.0);
    assert_eq!(h.session.active_leases(), 0);
    assert_eq!(
        h.session.policy_counters().await,
        PolicyCountersSnapshot::default(),
        "clean prompt path owns no policy outcomes"
    );
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_prompt_reply_tokio() {
    prompt_reply_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_prompt_reply_dedicated() {
    prompt_reply_impl(MstpExecutionMode::DedicatedThread).await;
}

async fn bidirectional_impl(mode: MstpExecutionMode) {
    let mut h = Harness::new(mode, SessionRole::Both).await;
    // Inbound: peer request-in answered promptly by the server role.
    h.peer_send(&data_expecting(1, 21)).await;
    let inbound_reply = h
        .wait_for("inbound ComplexAck", |f| {
            f.frame_type == FrameType::BACnetDataNotExpectingReply && f.destination == PEER_MAC
        })
        .await;
    expect_present_value(&inbound_reply, 21, 11.0);

    // Outbound: the session client initiates a confirmed request; the token
    // drive carries it (MAC self-organization is the backstop).
    let client = h.session.cloned_client_handle().expect("client role");
    let outbound = tokio::spawn(async move {
        client
            .read_property(
                &[PEER_MAC],
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let request = h
        .drive_token_until_data("outbound DataExpectingReply", |f| {
            f.frame_type == FrameType::BACnetDataExpectingReply && f.destination == PEER_MAC
        })
        .await;
    // Wire asserts: NPDU expects a reply; the client advertises the composed
    // 480 bound (identity proof on the MS/TP link).
    let npdu = decode_npdu(request.data.clone()).expect("request carries NPDU");
    assert!(npdu.expecting_reply);
    let outbound_iid = match decode_apdu(npdu.payload).expect("request carries APDU") {
        Apdu::ConfirmedRequest(req) => {
            assert_eq!(req.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
            assert_eq!(
                req.max_apdu_length, 480,
                "client must advertise the composed MS/TP bound"
            );
            req.invoke_id
        }
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    // Peer answers with the echoed invoke ID; the client future completes.
    h.peer_send(&MstpFrame {
        frame_type: FrameType::BACnetDataNotExpectingReply,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: complex_ack_npdu(outbound_iid, 22.0),
    })
    .await;
    let ack = tokio::time::timeout(WAIT, outbound)
        .await
        .expect("outbound hung")
        .expect("join failed")
        .expect("outbound failed");
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_bidirectional_tokio() {
    bidirectional_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_bidirectional_dedicated() {
    bidirectional_impl(MstpExecutionMode::DedicatedThread).await;
}

async fn deferred_impl(mode: MstpExecutionMode) {
    let mut h = Harness::new(mode, SessionRole::Both).await;
    // Arm suspension: the next prompt-capable request answers token-owned.
    h.session
        .server()
        .expect("server role")
        .suspend_next_reply()
        .expect("arm suspension");
    h.peer_send(&data_expecting(1, 33)).await;
    // The MAC releases ReplyPostponed once dispatch drops the prompt sender.
    let postponed = h
        .wait_for("ReplyPostponed", |f| {
            f.frame_type == FrameType::ReplyPostponed && f.destination == PEER_MAC
        })
        .await;
    assert_eq!(postponed.destination, PEER_MAC);
    // The deferred response follows at a token opportunity with the
    // postponed identity (same destination + invoke ID + value).
    let late = h
        .drive_token_until_data("deferred ComplexAck", |f| {
            f.frame_type == FrameType::BACnetDataNotExpectingReply && f.destination == PEER_MAC
        })
        .await;
    expect_present_value(&late, 33, 11.0);
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_deferred_after_postponed_tokio() {
    deferred_impl(MstpExecutionMode::Tokio).await;
}

#[tokio::test]
async fn mstp_deferred_after_postponed_dedicated() {
    deferred_impl(MstpExecutionMode::DedicatedThread).await;
}

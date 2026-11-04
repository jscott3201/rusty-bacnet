//! The RB-17 MS/TP harness the proof files share: a session under test,
//! built by [`MstpEndpointBuilder`], and a raw-frame peer, over
//! [`LoopbackSerial::pair`]. Each file declares `mstp_wire` beside it.
//!
//! The session's link runs standard frames, or extended ones with
//! [`MstpEndpointBuilder::extended_frames`] (#1663), when its identity then
//! advertises a Max_APDU_Length_Accepted of 1476.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use bacnet_encoding::apdu::{
    Apdu, ComplexAck, ConfirmedRequest as ConfirmedPdu, decode_apdu, encode_apdu,
};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint::identity::{DeviceIdentity, build_database_with_extra};
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_npdu::NetworkPriority;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::mstp::{
    LoopbackSerial, MstpExecutionMode, MstpThreadConfig, MstpTransport, SerialPort,
};
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier, Segmentation,
    ServiceSupported,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::mstp_wire::{
    FrameType, MstpFrame, decode_frame, encode_frame, reply_to_poll, token_frame,
};

/// A hang backstop, not a timing budget: no wait here is meant to come
/// near it. The tests run on paused time, the dedicated variants included
/// ([`dedicated`]), and a request the session dropped as out of turn never
/// comes back whatever the wait (#2269), which [`Harness::peer_deliver`]
/// handles.
///
/// One residual is left to a dedicated variant on the wall clock
/// ([`MstpExecutionMode::dedicated_thread`]), under extreme host load
/// only: the node's own thread is starved past the reply window (about
/// 250 ms) while it answers. It then rightly sends neither the answer nor
/// Reply Postponed, and the wait for either times out. The failure's
/// [`Harness::link_report`] shows it: `frames_received: 1,
/// npdus_delivered: 1, expired_requests: 1, reply_postponed_tx: 0`, and a
/// deadline lateness of 0.3 s or more. No harness change can recover that;
/// `.config/nextest.toml` gives such tests two threads each to make it
/// rarer.
pub const WAIT: Duration = Duration::from_secs(5);
/// How often [`Harness::peer_deliver`] sends a frame before giving up.
/// Each drop takes a stall of T_no_token or more on the test's thread, so
/// one resend is nearly always enough.
const DELIVERY_ATTEMPTS: usize = 8;
pub const SESSION_MAC: u8 = 3;
pub const PEER_MAC: u8 = 7;

/// A dedicated-thread node on the test's paused time (#2284): its thread
/// and runtime are its own, its deadlines the test runtime's, which
/// advances only when both are idle. Call it inside the test's runtime.
pub fn dedicated() -> MstpExecutionMode {
    MstpExecutionMode::DedicatedThread(
        MstpThreadConfig::new().with_test_time(tokio::runtime::Handle::current()),
    )
}

/// Whether `mode` runs the node on the test's paused time: a stalled test
/// thread can then move no MAC deadline.
fn on_test_time(mode: &MstpExecutionMode) -> bool {
    match mode {
        MstpExecutionMode::Tokio => true,
        MstpExecutionMode::DedicatedThread(thread) => thread.has_test_time(),
    }
}

/// The frames the session's link runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frames {
    /// Standard frames only, the builder's default: APDUs of up to 480.
    Standard,
    /// Extended frames on (`MstpEndpointBuilder::extended_frames`): APDUs of
    /// up to 1476.
    Extended,
}

impl Frames {
    /// Whether extended frames are on.
    pub fn extended(self) -> bool {
        self == Self::Extended
    }

    /// The APDU bound the link carries, which the identity advertises.
    pub fn max_apdu(self) -> u16 {
        match self {
            Self::Standard => 480,
            Self::Extended => 1476,
        }
    }
}

pub fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

/// The session's Device identity, advertising the APDU bound its link
/// carries; the builder rejects one advertising more.
pub fn identity(frames: Frames) -> DeviceIdentity {
    DeviceIdentity::new(2001, 42)
        .unwrap()
        .with_max_apdu(frames.max_apdu())
        .unwrap()
        .with_segmentation(Segmentation::NONE)
        .with_services(&[ServiceSupported::READ_PROPERTY])
}

pub fn db_with_analog(id: &DeviceIdentity, instance: u32, value: f32) -> ObjectDatabase {
    db_with_named_analog(id, instance, &format!("mstp-ai-{instance}"), value)
}

pub fn db_with_named_analog(
    id: &DeviceIdentity,
    instance: u32,
    name: &str,
    value: f32,
) -> ObjectDatabase {
    let mut o = AnalogInputObject::new(instance, name, EngineeringUnits::SQUARE_METERS).unwrap();
    o.set_present_value(value);
    build_database_with_extra(id, vec![Box::new(o)]).unwrap()
}

/// Serial wrapper recording every frame the session transmits (decoded).
///
/// Each transport write must contain exactly one complete frame. The peer
/// harness drains the byte channel so session writes never stall on
/// loopback backpressure; assertions read the decoded stream here
/// (frame-category + destination ownership inspection).
pub struct ObservedSerial {
    pub inner: LoopbackSerial,
    pub writes: mpsc::UnboundedSender<MstpFrame>,
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

pub struct Harness {
    pub session: EndpointSession<MstpTransport<ObservedSerial>>,
    pub peer: Arc<LoopbackSerial>,
    pub writes: mpsc::UnboundedReceiver<MstpFrame>,
    pub drain: tokio::task::JoinHandle<()>,
    /// Where the session's node runs; [`Self::peer_deliver`] resends only
    /// for a dedicated thread on the wall clock.
    pub mode: MstpExecutionMode,
}

impl Harness {
    pub async fn new(mode: MstpExecutionMode, role: SessionRole, frames: Frames) -> Self {
        let id = identity(frames);
        let db = db_with_analog(&id, 1, 11.0);
        Self::with_database(mode, role, frames, id, db).await
    }

    pub async fn with_database(
        mode: MstpExecutionMode,
        role: SessionRole,
        frames: Frames,
        id: DeviceIdentity,
        db: ObjectDatabase,
    ) -> Self {
        let (session_end, peer_end) = LoopbackSerial::pair();
        let (write_tx, writes) = mpsc::unbounded_channel();
        let observed = ObservedSerial {
            inner: session_end,
            writes: write_tx,
        };
        let mut session = MstpEndpointBuilder::new(observed, SESSION_MAC)
            .max_master(7)
            .extended_frames(frames.extended())
            .execution_mode(mode.clone())
            .role(role)
            // Bounded queues + client timers mirror the RB-16 proof shape.
            .queue_capacity(32)
            .client_timers(2_000, 0)
            .database(db)
            .device(id)
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
            mode,
        }
    }

    pub async fn next_frame(&mut self) -> MstpFrame {
        let frame = match tokio::time::timeout(WAIT, self.writes.recv()).await {
            Ok(frame) => frame.expect("writes closed"),
            Err(_) => panic!("session frame timed out; {}", self.link_report()),
        };
        assert_eq!(
            frame.source, SESSION_MAC,
            "one link owner: every session frame carries the session station"
        );
        frame
    }

    /// Collects session frames until `accept` matches (bounded, ownership-checked).
    pub async fn wait_for(
        &mut self,
        what: &str,
        mut accept: impl FnMut(&MstpFrame) -> bool,
    ) -> MstpFrame {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for session {what}; {}",
                self.link_report()
            );
            let frame = match tokio::time::timeout(remaining, self.writes.recv()).await {
                Ok(frame) => frame.expect("writes closed"),
                Err(_) => panic!(
                    "session frame timed out waiting for {what}; {}",
                    self.link_report()
                ),
            };
            assert_eq!(
                frame.source, SESSION_MAC,
                "one link owner: every session frame carries the session station"
            );
            if accept(&frame) {
                return frame;
            }
        }
    }

    /// The session's MAC state and counters, for a failure message: they
    /// tell a dropped request from a late or missing answer.
    pub fn link_report(&self) -> String {
        match self.session.mstp_diagnostics() {
            Some(diagnostics) => format!(
                "node {:?}, {:?}",
                diagnostics.node_state(),
                diagnostics.snapshot()
            ),
            None => "no MS/TP diagnostics".to_owned(),
        }
    }

    pub async fn peer_send(&self, frame: &MstpFrame) {
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, frame).expect("valid test frame");
        self.peer
            .write(&buf)
            .await
            .expect("peer write must succeed");
    }

    /// Sends `frame`, a data frame the session must hand up, until the
    /// session's MAC delivers it (#2269).
    ///
    /// The peer sends out of turn: it never holds the token, so the session
    /// takes the frame only while it sits in IDLE. A dedicated-thread MAC
    /// on the wall clock ([`MstpExecutionMode::dedicated_thread`]) runs its
    /// own runtime's time and keeps running while a loaded host starves the
    /// test's thread. After T_no_token (500 ms) of
    /// silence it polls for a successor, and a data frame that arrives
    /// during a poll is an unexpected frame: the session drops it and goes
    /// back to IDLE (Clause 9.5.6.8). No wait, however long, brings that
    /// frame back, so the peer resends it, as a requester retries. The MAC's
    /// counters tell the outcomes apart: they are published after each
    /// event, so the snapshot that first counts the frame received also
    /// counts it delivered, or never will.
    ///
    /// The Tokio variants and [`dedicated`] run on paused time, where a
    /// stall of the test's thread moves no MAC deadline, so there the first
    /// send must be delivered: a drop is a bug, and the call panics instead
    /// of resending.
    ///
    /// Call it with no earlier peer frame still on its way to the session,
    /// so the next frame the MAC counts is this one.
    pub async fn peer_deliver(&self, frame: &MstpFrame) {
        let diagnostics = self
            .session
            .mstp_diagnostics()
            .expect("an MS/TP session has diagnostics");
        for _ in 0..DELIVERY_ATTEMPTS {
            let before = diagnostics.snapshot().mac;
            self.peer_send(frame).await;
            let judged = tokio::time::timeout(WAIT, async {
                loop {
                    let now = diagnostics.snapshot().mac;
                    if now.frames_received > before.frames_received {
                        break now;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "the session never counted the peer's frame; {}",
                    self.link_report()
                )
            });
            if judged.npdus_delivered > before.npdus_delivered {
                return;
            }
            assert!(
                !on_test_time(&self.mode),
                "on paused time the session must take the peer's frame first time; {}",
                self.link_report()
            );
        }
        panic!(
            "the session dropped the peer's frame {DELIVERY_ATTEMPTS} times; {}",
            self.link_report()
        );
    }

    /// Drives token opportunities until a data frame matching `is_data`
    /// arrives (bounded rounds, event-driven). Frames without that data
    /// (PFM/Token/`ReplyPostponed`) only keep the ring turning: whichever of
    /// the egress-queue fill vs MAC reply wins the race, the outcome is the
    /// same postponed identity delivered token-owned.
    pub async fn drive_token_until_data(
        &mut self,
        what: &str,
        mut is_data: impl FnMut(&MstpFrame) -> bool,
    ) -> MstpFrame {
        // The peer plays the session's only other master: it hands over the
        // token, answers a Poll For Master addressed to it, and passes back
        // every token it is given. A poll for another address is left to time
        // out; a token sent while the session polls would be an unexpected
        // frame there (Clause 9.5.6.8) and cost the token.
        self.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
        for _ in 0..256 {
            let frame = self.wait_for(what, |_| true).await;
            if is_data(&frame) {
                return frame;
            }
            match frame.frame_type {
                FrameType::POLL_FOR_MASTER if frame.destination == PEER_MAC => {
                    self.peer_send(&reply_to_poll(SESSION_MAC, PEER_MAC)).await;
                }
                FrameType::TOKEN if frame.destination == PEER_MAC => {
                    self.peer_send(&token_frame(SESSION_MAC, PEER_MAC)).await;
                }
                _ => {}
            }
        }
        panic!("token drive did not deliver {what}; {}", self.link_report());
    }

    pub async fn shutdown(mut self) {
        self.session.stop().await.expect("session must stop");
        self.drain.abort();
    }
}

/// A ReadProperty of AI `instance`'s Present_Value from a requester that
/// accepts APDUs of up to 480 octets.
pub fn read_request_apdu(invoke_id: u8, instance: u32) -> Bytes {
    read_apdu(invoke_id, instance, PropertyIdentifier::PRESENT_VALUE, 480)
}

/// A ReadProperty of AI `instance`'s `property` from a requester that
/// accepts APDUs of up to `max_apdu` octets.
pub fn read_apdu(
    invoke_id: u8,
    instance: u32,
    property: PropertyIdentifier,
    max_apdu: u16,
) -> Bytes {
    let mut svc = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, instance),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut svc);
    let apdu = Apdu::ConfirmedRequest(ConfirmedPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: max_apdu,
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

/// `apdu` in a local NPDU.
pub fn local_npdu(apdu: Bytes, expecting_reply: bool) -> Bytes {
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply,
            priority: NetworkPriority::Normal,
            destination: None,
            source: None,
            payload: apdu,
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.freeze()
}

pub fn read_request_npdu(invoke_id: u8, instance: u32) -> Bytes {
    local_npdu(read_request_apdu(invoke_id, instance), true)
}

pub fn data_expecting(instance: u32, invoke_id: u8) -> MstpFrame {
    MstpFrame {
        frame_type: FrameType::BACNET_DATA_EXPECTING_REPLY,
        destination: SESSION_MAC,
        source: PEER_MAC,
        data: read_request_npdu(invoke_id, instance),
    }
}

pub fn complex_ack_npdu(invoke_id: u8, value: f32) -> Bytes {
    let mut val = BytesMut::new();
    encode_property_value(&mut val, &PropertyValue::Real(value)).unwrap();
    let ack = ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: val.to_vec(),
    };
    let mut svc = BytesMut::new();
    ack.encode(&mut svc).unwrap();
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
    local_npdu(encoded.freeze(), false)
}

pub fn decode_complex_ack(frame: &MstpFrame) -> (u8, ReadPropertyACK) {
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

pub fn expect_present_value(frame: &MstpFrame, invoke_id: u8, value: f32) {
    let (iid, ack) = decode_complex_ack(frame);
    assert_eq!(iid, invoke_id, "postponed identity: invoke ID preserved");
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    let mut expect = BytesMut::new();
    encode_property_value(&mut expect, &PropertyValue::Real(value)).unwrap();
    assert_eq!(ack.property_value, expect.to_vec());
}

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
    decode_apdu, encode_apdu, Apdu, ComplexAck, ConfirmedRequest as ConfirmedPdu,
};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::mstp::{LoopbackSerial, MstpExecutionMode, MstpTransport, SerialPort};
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier, Segmentation,
    ServiceSupported,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use crate::mstp_wire::{
    decode_frame, encode_frame, reply_to_poll, token_frame, FrameType, MstpFrame,
};

pub const WAIT: Duration = Duration::from_secs(5);
pub const SESSION_MAC: u8 = 3;
pub const PEER_MAC: u8 = 7;

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
    let mut o = AnalogInputObject::new(instance, name, 0).unwrap();
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

    pub async fn next_frame(&mut self) -> MstpFrame {
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

    /// Collects session frames until `accept` matches (bounded, ownership-checked).
    pub async fn wait_for(
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

    pub async fn peer_send(&self, frame: &MstpFrame) {
        let mut buf = BytesMut::new();
        encode_frame(&mut buf, frame).expect("valid test frame");
        self.peer
            .write(&buf)
            .await
            .expect("peer write must succeed");
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
        panic!("token drive did not deliver {what}");
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
            is_network_message: false,
            expecting_reply,
            priority: NetworkPriority::NORMAL,
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

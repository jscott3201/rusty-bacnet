//! RB-17 MS/TP proof: one-link endpoint over simulated serial, one owner.
//!
//! Session-under-test (via [`MstpEndpointBuilder`](bacnet_endpoint::mstp::MstpEndpointBuilder))
//! and raw-frame peer harness over [`LoopbackSerial::pair`](bacnet_transport::mstp::LoopbackSerial),
//! shared with `rb17_mstp_flows.rs` in `support/rb17_harness.rs`.
//! Every logical scenario runs in BOTH `Tokio` and `DedicatedThread`
//! execution modes with the same assertions, with standard frames and with
//! extended frames on (`MstpEndpointBuilder::extended_frames`, #1663).
//! Deterministic and event-driven (timeout-guarded channel receives, no
//! sleeps).
//!
//! Evidence level (simulator, NOT physical bench or on-wire conformance):
//! this file proves single serial ownership + start-once, prompt replies via
//! the responder `reply_tx` path with no preceding `ReplyPostponed`,
//! bidirectional confirmed traffic, and postponed → later token-owned
//! responses with the postponed identity (destination + invoke ID + value)
//! preserved. Timing qualification is RB-26; router work is RB-25.
//! Companion `rb17_mstp_flows.rs` holds release/queue/cancellation/
//! stop/denial/non-routing proofs, and `rb17_mstp_extended.rs` the
//! extended-frame bounds (file-size gate).
//!
//! Reply-path shape under test: prompt replies use the one-use `reply_tx`
//! (MAC `AnswerDataRequest` window, no token needed); token-owned sends
//! (client initiations, deferred-after-postponed) go through the single
//! `EndpointEgress` → `queue_npdu` path as `DataNotExpectingReply`. The peer
//! drives token opportunities by injecting `Token` frames addressed to the
//! session station (simulator control, not conformance traffic); the MAC's
//! own self-organization is the backstop, never the primary driver.

#[path = "support/mstp_wire.rs"]
mod mstp_wire;
#[path = "support/rb17_harness.rs"]
mod rb17_harness;

use std::time::Duration;

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::mstp::MstpEndpointBuilder;
use bacnet_endpoint::session::{PolicyCountersSnapshot, SessionRole};
use bacnet_transport::mstp::{LoopbackSerial, MstpExecutionMode, SerialPort};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use mstp_wire::{FrameType, MstpFrame};
use rb17_harness::*;
use tokio::sync::mpsc;

async fn one_owner_impl(mode: MstpExecutionMode, frames: Frames) {
    // Transport-level identity: one MAC, the link's APDU bound, start-once.
    let (serial, _peer) = LoopbackSerial::pair();
    let mut transport = MstpEndpointBuilder::new(serial, SESSION_MAC)
        .max_master(7)
        .extended_frames(frames.extended())
        .execution_mode(mode.clone())
        .build_transport()
        .expect("transport must build");
    assert_eq!(transport.local_mac(), &[SESSION_MAC]);
    assert_eq!(transport.egress_apdu_limit(), frames.max_apdu());
    assert_eq!(transport.local_receive_apdu_capacity(), frames.max_apdu());
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
    let id = identity(frames);
    let mut session = MstpEndpointBuilder::new(observed, SESSION_MAC)
        .max_master(7)
        .extended_frames(frames.extended())
        .execution_mode(mode)
        .database(db_with_analog(&id, 1, 1.0))
        .device(id)
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
    one_owner_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_one_serial_owner_dedicated() {
    one_owner_impl(MstpExecutionMode::dedicated_thread(), Frames::Standard).await;
}

#[tokio::test]
async fn mstp_one_serial_owner_extended_tokio() {
    one_owner_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_one_serial_owner_extended_dedicated() {
    one_owner_impl(MstpExecutionMode::dedicated_thread(), Frames::Extended).await;
}

async fn prompt_reply_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = Harness::new(mode, SessionRole::Both, frames).await;
    h.peer_send(&data_expecting(1, 9)).await;
    // Prompt path: DataNotExpectingReply with NO ReplyPostponed before it.
    let mut saw_postponed = false;
    let reply = h
        .wait_for("prompt ComplexAck", |f| {
            if f.frame_type == FrameType::REPLY_POSTPONED {
                saw_postponed = true;
            }
            f.frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY && f.destination == PEER_MAC
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
    prompt_reply_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_prompt_reply_dedicated() {
    prompt_reply_impl(MstpExecutionMode::dedicated_thread(), Frames::Standard).await;
}

#[tokio::test]
async fn mstp_prompt_reply_extended_tokio() {
    prompt_reply_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_prompt_reply_extended_dedicated() {
    prompt_reply_impl(MstpExecutionMode::dedicated_thread(), Frames::Extended).await;
}

async fn bidirectional_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = Harness::new(mode, SessionRole::Both, frames).await;
    // Inbound: peer request-in answered promptly by the server role.
    h.peer_send(&data_expecting(1, 21)).await;
    let inbound_reply = h
        .wait_for("inbound ComplexAck", |f| {
            f.frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY && f.destination == PEER_MAC
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
            f.frame_type == FrameType::BACNET_DATA_EXPECTING_REPLY && f.destination == PEER_MAC
        })
        .await;
    // Wire asserts: NPDU expects a reply; the client advertises the composed
    // bound, 480 or 1476 (identity proof on the MS/TP link).
    let npdu = decode_npdu(request.data.clone()).expect("request carries NPDU");
    assert!(npdu.expecting_reply);
    let outbound_iid = match decode_apdu(npdu.payload).expect("request carries APDU") {
        Apdu::ConfirmedRequest(req) => {
            assert_eq!(req.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
            assert_eq!(
                req.max_apdu_length,
                frames.max_apdu(),
                "client must advertise the composed MS/TP bound"
            );
            req.invoke_id
        }
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    // Peer answers with the echoed invoke ID; the client future completes.
    h.peer_send(&MstpFrame {
        frame_type: FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
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
    bidirectional_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_bidirectional_dedicated() {
    bidirectional_impl(MstpExecutionMode::dedicated_thread(), Frames::Standard).await;
}

#[tokio::test]
async fn mstp_bidirectional_extended_tokio() {
    bidirectional_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_bidirectional_extended_dedicated() {
    bidirectional_impl(MstpExecutionMode::dedicated_thread(), Frames::Extended).await;
}

async fn deferred_impl(mode: MstpExecutionMode, frames: Frames) {
    let mut h = Harness::new(mode, SessionRole::Both, frames).await;
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
            f.frame_type == FrameType::REPLY_POSTPONED && f.destination == PEER_MAC
        })
        .await;
    assert_eq!(postponed.destination, PEER_MAC);
    // The deferred response follows at a token opportunity with the
    // postponed identity (same destination + invoke ID + value).
    let late = h
        .drive_token_until_data("deferred ComplexAck", |f| {
            f.frame_type == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY && f.destination == PEER_MAC
        })
        .await;
    expect_present_value(&late, 33, 11.0);
    h.shutdown().await;
}

#[tokio::test]
async fn mstp_deferred_after_postponed_tokio() {
    deferred_impl(MstpExecutionMode::Tokio, Frames::Standard).await;
}

#[tokio::test]
async fn mstp_deferred_after_postponed_dedicated() {
    deferred_impl(MstpExecutionMode::dedicated_thread(), Frames::Standard).await;
}

#[tokio::test]
async fn mstp_deferred_after_postponed_extended_tokio() {
    deferred_impl(MstpExecutionMode::Tokio, Frames::Extended).await;
}

#[tokio::test]
async fn mstp_deferred_after_postponed_extended_dedicated() {
    deferred_impl(MstpExecutionMode::dedicated_thread(), Frames::Extended).await;
}

/// The builder's timing setters reach the transport's `MstpConfig`; left
/// alone they keep `MstpConfig::default()`'s, and a value outside Clause
/// 9.5.3's ranges, or a pad over 15 ms, fails validation naming the field.
#[test]
fn mstp_builder_timing_setters_reach_the_transport() {
    let ms = Duration::from_millis;
    let (serial, _peer) = LoopbackSerial::pair();
    let defaults = MstpEndpointBuilder::new(serial, SESSION_MAC)
        .build_transport()
        .expect("transport must build");
    let expected = bacnet_transport::mstp::MstpConfig::default();
    assert_eq!(
        (
            defaults.config().usage_timeout,
            defaults.config().reply_timeout,
            defaults.config().transmit_latency_pad
        ),
        (
            expected.usage_timeout,
            expected.reply_timeout,
            expected.transmit_latency_pad
        )
    );
    let (serial, _peer) = LoopbackSerial::pair();
    let set = MstpEndpointBuilder::new(serial, SESSION_MAC)
        .usage_timeout(ms(25))
        .reply_timeout(ms(280))
        .transmit_latency_pad(ms(10))
        .build_transport()
        .expect("transport must build");
    assert_eq!(set.config().usage_timeout, ms(25));
    assert_eq!(set.config().reply_timeout, ms(280));
    assert_eq!(set.config().transmit_latency_pad, ms(10));
    for (usage, reply, pad, field) in [
        (ms(19), ms(300), ms(0), "usage_timeout"),
        (ms(36), ms(300), ms(0), "usage_timeout"),
        (ms(35), ms(254), ms(0), "reply_timeout"),
        (ms(35), ms(301), ms(0), "reply_timeout"),
        (ms(35), ms(300), ms(16), "transmit_latency_pad"),
    ] {
        let (serial, _peer) = LoopbackSerial::pair();
        let error = MstpEndpointBuilder::new(serial, SESSION_MAC)
            .usage_timeout(usage)
            .reply_timeout(reply)
            .transmit_latency_pad(pad)
            .validate_only()
            .unwrap_err()
            .to_string();
        assert!(error.contains(field), "{error}");
    }
}

// Relocated verbatim from rb17_mstp_flows.rs (file-size gate): builder
// addressing/APDU-bound validation is mode-independent and self-contained.
#[tokio::test]
async fn mstp_builder_bounds_reject() {
    // Addressing validation fails fast at composition (MstpConfig::validate).
    let (serial, _peer) = LoopbackSerial::pair();
    assert!(MstpEndpointBuilder::new(serial, 9)
        .max_master(7)
        .validate_only()
        .is_err());
    let (serial, _peer) = LoopbackSerial::pair();
    assert!(MstpEndpointBuilder::new(serial, SESSION_MAC)
        .max_master(200)
        .validate_only()
        .is_err());
    // A port declaring an echo with no grace is rejected.
    let (serial, _peer) = LoopbackSerial::pair();
    let zero = bacnet_transport::mstp::Echo::Full {
        grace: Duration::ZERO,
    };
    assert!(
        MstpEndpointBuilder::new(serial.with_echo(zero), SESSION_MAC)
            .validate_only()
            .is_err()
    );
    // The requested ceiling is accepted here and resolved before startup.
    let (serial, _peer) = LoopbackSerial::pair();
    let big = DeviceIdentity::new(2002, 42).unwrap();
    assert_eq!(big.max_apdu_length(), 1476);
    assert!(MstpEndpointBuilder::new(serial, SESSION_MAC)
        .device(big)
        .build_session()
        .is_ok());
}

//! Extended frames through the host transport (#1611): the flag in
//! `MstpConfig`, the port's NPDU and APDU limits, the router's hook, and a
//! 1497-octet NPDU between two transports (Clauses 9.3, 9.3.9, 9.3.10).

use bacnet_mstp_codec::{FrameType, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_MPDU_DATA};
use bytes::Bytes;
use tokio::time::Duration;

use super::echo_tests::ECHOING;
use super::test_support::{Bus, config, encode};
use super::*;
use crate::any::AnyTransport;
use crate::port::TransportPort;

/// An NPDU of `length` octets; `expecting_reply` sets the control octet's
/// data-expecting-reply bit.
fn npdu(length: usize, expecting_reply: bool, tag: u8) -> Vec<u8> {
    let mut npdu: Vec<u8> = (0..length)
        .map(|index| (index as u8).wrapping_mul(13) ^ tag)
        .collect();
    npdu[0] = 0x01;
    npdu[1] = if expecting_reply { 0x04 } else { 0x00 };
    npdu
}

fn extended(station: u8, max_master: u8) -> MstpConfig {
    MstpConfig {
        baud_rate: 38_400,
        extended_frames: true,
        ..config(station, max_master)
    }
}

/// Clause 9.3: the port's NPDU limit is an extended frame's 1497 octets
/// with the flag and a standard one's 501 without, and the APDU limits the
/// network layer reads follow: 1476 and 480.
#[test]
fn the_ports_limits_follow_the_flag() {
    let (serial, _peer) = LoopbackSerial::pair();
    let standard = MstpTransport::new(serial, MstpConfig::default());
    assert!(!standard.config().extended_frames);
    assert_eq!(standard.max_npdu_length(), MAX_STANDARD_MPDU_DATA);
    assert_eq!(standard.local_receive_apdu_capacity(), 480);
    assert_eq!(standard.egress_apdu_limit(), 480);

    let (serial, _peer) = LoopbackSerial::pair();
    let extended = MstpTransport::new(serial, extended(1, 7));
    assert_eq!(extended.max_npdu_length(), MAX_EXTENDED_MPDU_DATA);
    assert_eq!(extended.local_receive_apdu_capacity(), 1476);
    assert_eq!(extended.egress_apdu_limit(), 1476);
    assert!(extended.config().validate().is_ok());
    let core = extended.config().core_config().unwrap();
    assert!(core.extended_frames);
    assert!(core.transmit_watchdog >= core.longest_frame_time());
}

/// Clause 9.3: a router turns extended frames on for every MS/TP port it
/// routes, before it starts it, through `prepare_for_routing`, which
/// `AnyTransport` passes on. A port already started keeps what it runs.
#[tokio::test]
async fn preparing_for_routing_turns_extended_frames_on() {
    let (serial, _peer) = LoopbackSerial::pair();
    let mut any: AnyTransport = MstpTransport::new(serial, MstpConfig::default()).into();
    assert_eq!(any.local_receive_apdu_capacity(), 480);
    any.prepare_for_routing();
    assert_eq!(any.local_receive_apdu_capacity(), 1476);
    assert_eq!(any.egress_apdu_limit(), 1476);

    let (serial, _peer) = LoopbackSerial::pair();
    let mut started = MstpTransport::new(serial, MstpConfig::default());
    let _npdus = started.start().await.unwrap();
    started.prepare_for_routing();
    assert!(!started.config().extended_frames);
    assert_eq!(started.local_receive_apdu_capacity(), 480);
    started.stop().await.unwrap();
}

/// Clauses 9.3.9, 9.3.10 and 9.5.6.9: between two transports with extended
/// frames, a 1497-octet request goes out as a Type 32, is delivered with a
/// reply channel, and its 1497-octet answer comes back as a Type 33 inside
/// the reply window. Then a 1497-octet NPDU that needs no reply goes the
/// other way.
#[tokio::test(start_paused = true)]
async fn a_1497_octet_npdu_round_trips_with_extended_frames() {
    let (serial_a, serial_b) = LoopbackSerial::pair();
    let mut a = MstpTransport::new(serial_a, extended(0, 1));
    let mut b = MstpTransport::new(serial_b, extended(1, 1));
    let mut from_b = a.start().await.unwrap();
    let mut from_a = b.start().await.unwrap();

    let request = npdu(MAX_EXTENDED_MPDU_DATA, true, 0x3C);
    let answer = npdu(MAX_EXTENDED_MPDU_DATA, false, 0xA5);
    a.send_unicast(&request, &[1]).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(10), from_a.recv())
        .await
        .expect("the request arrives")
        .unwrap();
    assert_eq!(received.npdu.as_ref(), request.as_slice());
    assert_eq!(received.source_mac.as_slice(), &[0]);
    received
        .reply_tx
        .expect("a request carries a reply channel")
        .send(Bytes::from(answer.clone()))
        .unwrap();
    let replied = tokio::time::timeout(Duration::from_secs(10), from_b.recv())
        .await
        .expect("the answer arrives")
        .unwrap();
    assert_eq!(replied.npdu.as_ref(), answer.as_slice());

    let back = npdu(MAX_EXTENDED_MPDU_DATA, false, 0x0F);
    b.send_unicast(&back, &[0]).await.unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(10), from_b.recv())
        .await
        .expect("the NPDU arrives")
        .unwrap();
    assert_eq!(delivered.npdu.as_ref(), back.as_slice());

    // The answer went out in the reply window, not as a deferred one.
    let a_counts = a.diagnostics().snapshot();
    let b_counts = b.diagnostics().snapshot();
    assert_eq!(a_counts.mac.parser_errors + b_counts.mac.parser_errors, 0);
    assert_eq!(b_counts.late_answers_queued, 0);
    assert_eq!(a_counts.wait_for_reply_timeouts, 0);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}

/// Without extended frames a port refuses an NPDU over 501 octets at the
/// send call, as before; with them, one over 1497.
#[tokio::test]
async fn a_long_npdu_is_refused_by_what_the_port_sends() {
    for (config, refused) in [
        (config(0, 7), MAX_STANDARD_MPDU_DATA + 1),
        (extended(0, 7), MAX_EXTENDED_MPDU_DATA + 1),
    ] {
        let (serial, _peer) = LoopbackSerial::pair();
        let mut transport = MstpTransport::new(serial, config);
        let _npdus = transport.start().await.unwrap();
        let error = transport
            .send_unicast(&npdu(refused, false, 1), &[5])
            .await
            .unwrap_err();
        let limit = refused - 1;
        assert!(
            matches!(&error, Error::Encoding(text) if text.contains(&format!("maximum {limit}"))),
            "{error}"
        );
        assert!(matches!(
            transport.send_broadcast(&npdu(refused, false, 2)).await,
            Err(Error::Encoding(_))
        ));
        transport
            .send_unicast(&npdu(limit, false, 3), &[5])
            .await
            .unwrap();
        assert_eq!(transport.diagnostics().snapshot().outbound_oversize, 2);
        transport.stop().await.unwrap();
    }
}

/// On a port that hears its own frames, the echo of a 1516-octet extended
/// frame comes back over three 512-octet reads, and the transport strips it
/// whole: nothing of it reaches the core as a frame from this station's
/// address or an invalid frame, and the token pass follows T_turnaround
/// after the frame, as after a standard one.
#[tokio::test(start_paused = true)]
async fn the_echo_of_an_extended_frame_is_stripped_whole() {
    let config = MstpConfig {
        extended_frames: true,
        ..config(3, 4)
    };
    let mut bus = Bus::start_with(config, ECHOING).await;
    let token = bus.join(4).await;
    let long = npdu(MAX_EXTENDED_MPDU_DATA, false, 0x77);
    let wire = encode(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        4,
        3,
        &long,
    );
    assert!(wire.len() > 2 * 512, "the echo spans three reads");
    bus.transport.send_unicast(&long, &[4]).await.unwrap();
    bus.return_token(&token, 4).await;
    let sent = bus.next().await;
    assert_eq!(
        (sent.frame_type, sent.destination, &sent.data),
        (
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            4,
            &long
        )
    );
    let pass = bus.next().await;
    assert_eq!((pass.frame_type, pass.destination), (FrameType::TOKEN, 4));
    assert!(pass.start >= sent.end + bus.turnaround);
    assert!(pass.start <= sent.end + bus.turnaround + Duration::from_millis(1));
    let counts = bus.transport.diagnostics().snapshot();
    assert_eq!(counts.mac.own_address_frames, 0);
    assert_eq!(counts.mac.parser_errors, 0);
    assert_eq!((counts.echo_mismatches, counts.echo_missing), (0, 0));
    assert!(bus.npdus.try_recv().is_err(), "nothing went up");
    bus.stop().await;
}

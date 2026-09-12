//! Hub-only Proprietary-Message transit, using independent raw wire
//! expectations. Registered unicast AND broadcast stay opaque (AB.5.3.2/AB.5.3.3
//! forwarding); the shape validator only owns hub-local and node destinations.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::response_silence_tests::Snapshot;
use super::shutdown_blocked_tests::ControlledPeer;
use super::unknown_transit_tests::{
    barrier, connect_limits, matrix_barrier, matrix_pair, matrix_receive, raw, recv, send, stopped,
};
use super::*;
use crate::sc_frame::BROADCAST_VMAC;
use std::time::Duration;

fn valid_proprietary() -> Vec<u8> {
    vec![0x00, 0x2B, 0x42, 0xAA, 0xBB]
}

fn minimal_proprietary() -> Vec<u8> {
    vec![0x12, 0x34, 0x56]
}

async fn addressed_valid(function: u8, dest: Vmac) {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    b.connect(0x43).await;
    let mut a = ControlledPeer::open(&tls, &hub).await;
    a.connect(0x42).await;
    let body = valid_proprietary();
    send(&mut a, raw(function, 0, None, Some(dest), 0, &body)).await;
    let received = tokio::time::timeout(Duration::from_millis(200), b.ws.next()).await;
    stopped(&mut hub).await;
    // Unicast strips the destination and stamps the origin; broadcast keeps
    // the destination and stamps the origin so the receiver sees broadcast.
    let expected = if dest == BROADCAST_VMAC {
        raw(function, 0, Some([0x42; 6]), Some(BROADCAST_VMAC), 0, &body)
    } else {
        raw(function, 0, Some([0x42; 6]), None, 0, &body)
    };
    assert!(
        matches!(&received, Ok(Some(Ok(Message::Binary(data)))) if data.as_ref() == expected),
        "function {function:#04x} dest {dest:02x?}: addressed well-formed frame must transit, not be locally rejected; got {received:?}"
    );
}

#[tokio::test]
async fn proprietary_transit_addressed_wellformed_unicast() {
    addressed_valid(12, [0x43; 6]).await;
}

#[tokio::test]
async fn proprietary_transit_addressed_wellformed_broadcast() {
    addressed_valid(12, BROADCAST_VMAC).await;
}

#[tokio::test]
async fn proprietary_transit_opaque_options_shapes_and_no_echo() {
    let tls = TestTls::new();
    let (mut hub, mut a, mut b) = matrix_pair(&tls, true, 0x42, 0x43).await;
    let mut c = ControlledPeer::open(&tls, &hub).await;
    c.connect(0x44).await;
    let mut cases = 0;
    for id in [0, u16::MAX] {
        for (flags, options) in [
            (0, vec![]),
            (2, vec![0x5E]),
            (2, vec![0xFE, 0, 0, 0x1F]),
            (3, vec![0xE2, 0, 0, 0x1F, 0xFE, 0, 1, 0xBB, 0x1F]),
        ] {
            // Transit stays opaque: even forbidden shapes relay when the
            // envelope is an eligible registered unicast. The destination
            // validator owns the NAK, not the forwarding hub.
            for payload in [
                &b""[..],
                &minimal_proprietary()[..],
                &valid_proprietary()[..],
                &[0xFF; 9][..],
            ] {
                let mut body = options.clone();
                body.extend_from_slice(payload);
                let wire = raw(12, id, None, Some([0x43; 6]), flags, &body);
                assert_eq!(decode_sc_message(&wire).unwrap().payload.as_ref(), payload);
                send(&mut a, wire).await;
                assert_eq!(
                    recv(&mut b).await,
                    raw(12, id, Some([0x42; 6]), None, flags, &body)
                );
                barrier(&mut a).await;
                barrier(&mut c).await;
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 32);
    stopped(&mut hub).await;
}

#[tokio::test]
async fn proprietary_transit_broadcast_fanout_except_source() {
    let tls = TestTls::new();
    let (mut hub, mut a, mut b) = matrix_pair(&tls, true, 0x42, 0x43).await;
    let mut c = ControlledPeer::open(&tls, &hub).await;
    c.connect(0x44).await;
    for payload in [minimal_proprietary(), valid_proprietary()] {
        send(
            &mut a,
            raw(12, 0x1234, None, Some(BROADCAST_VMAC), 0, &payload),
        )
        .await;
        // Both other peers receive the broadcast with origin-stamp and
        // preserved broadcast destination; the source gets no echo.
        assert_eq!(
            recv(&mut b).await,
            raw(
                12,
                0x1234,
                Some([0x42; 6]),
                Some(BROADCAST_VMAC),
                0,
                &payload
            )
        );
        assert_eq!(
            recv(&mut c).await,
            raw(
                12,
                0x1234,
                Some([0x42; 6]),
                Some(BROADCAST_VMAC),
                0,
                &payload
            )
        );
        barrier(&mut a).await;
    }
    stopped(&mut hub).await;
}

#[tokio::test]
async fn proprietary_transit_local_preregistered_silence_and_state_matrix() {
    let tls = TestTls::new();
    let mut cases = [0; 2];
    for registered in [false, true] {
        for id in [0, u16::MAX] {
            for origin in [None, Some([0x43; 6]), Some([0; 6]), Some(BROADCAST_VMAC)] {
                for dest in [
                    None,
                    Some([0x43; 6]),
                    Some(BROADCAST_VMAC),
                    Some([0; 6]),
                    Some([0x77; 6]),
                    Some([0x42; 6]),
                ] {
                    // Accepted transit (registered, no explicit origin, any
                    // present dest except self-unicast) is tested separately.
                    if registered && origin.is_none() && matches!(dest, Some(v) if v != [0x42; 6]) {
                        continue;
                    }
                    let (mut hub, mut a, mut b) = matrix_pair(&tls, registered, 0x42, 0x43).await;
                    let expires = a.deadline.expires();
                    let before: Vec<_> = hub
                        .clients
                        .lock()
                        .await
                        .iter()
                        .map(|(vmac, client)| (*vmac, Snapshot::capture(client)))
                        .collect();
                    // Well-formed local draws the generic proprietary-unknown
                    // code; malformed bodies draw their shape codes. Both stay
                    // local. Broadcast and reserved origins stay silent.
                    for (flags, body, code) in [
                        (0, valid_proprietary(), 144u16),
                        (0, vec![], 149),
                        (0, vec![0x00, 0x2B], 147),
                    ] {
                        let case = format!("proprietary registered={registered} id={id} origin={origin:02x?} dest={dest:02x?} flags={flags} body={body:02x?}");
                        send(&mut a, raw(12, id, origin, dest, flags, &body)).await;
                        if (!registered || dest.is_none())
                            && dest != Some(BROADCAST_VMAC)
                            && origin != Some([0; 6])
                            && origin != Some(BROADCAST_VMAC)
                        {
                            let [hi, lo] = code.to_be_bytes();
                            matrix_receive(
                                &mut a,
                                &raw(0, id, None, origin, 0, &[12, 1, 0, 0, 7, hi, lo]),
                                &case,
                            )
                            .await;
                        }
                        matrix_barrier(&mut a, &format!("{case} source")).await;
                        matrix_barrier(&mut b, &format!("{case} observer")).await;
                        assert_eq!(a.deadline.expires(), expires, "{case}");
                        assert_eq!(a.deadline.is_committed(), registered, "{case}");
                        let map = hub.clients.lock().await;
                        assert_eq!(map.len(), if registered { 2 } else { 1 }, "{case}");
                        for (vmac, snapshot) in &before {
                            snapshot.unchanged(map.get(vmac).unwrap());
                        }
                        cases[usize::from(registered)] += 1;
                    }
                    if !registered {
                        a.connect(0x42).await;
                        assert!(a.deadline.is_committed());
                        assert_eq!(a.deadline.expires(), expires);
                    }
                    stopped(&mut hub).await;
                }
            }
        }
    }
    // Registered skips 2 IDs * 1 eligible origin * 4 non-self dests * 3 bodies
    // worth of accepted transit; preregistered covers the full matrix.
    assert_eq!(cases, [144, 120]);
}

#[tokio::test]
async fn proprietary_transit_result_for_12_relay_and_others_drop() {
    let tls = TestTls::new();
    let mut cases = [0; 2];
    for registered in [false, true] {
        for id in [0, u16::MAX] {
            for origin in [None, Some([0x42; 6]), Some([0; 6]), Some(BROADCAST_VMAC)] {
                for dest in [
                    None,
                    Some([0x42; 6]),
                    Some([0x43; 6]),
                    Some([0x77; 6]),
                    Some([0; 6]),
                    Some(BROADCAST_VMAC),
                ] {
                    let (mut hub, mut b, mut a) = matrix_pair(&tls, registered, 0x43, 0x42).await;
                    let expires = b.deadline.expires();
                    for result in [
                        vec![12, 0],
                        vec![12, 1, 0xFE, 0, 7, 0, 144, b'x', 0xC3, 0xA9],
                    ] {
                        let mut body = vec![0xFE, 0, 0, 0x1F];
                        body.extend_from_slice(&result);
                        let case = format!("ResultFor=12 registered={registered} id={id} origin={origin:02x?} dest={dest:02x?} result={result:02x?}");
                        send(&mut b, raw(0, id, origin, dest, 2, &body)).await;
                        if registered && origin.is_none() && dest == Some([0x42; 6]) {
                            matrix_receive(
                                &mut a,
                                &raw(0, id, Some([0x43; 6]), None, 2, &body),
                                &case,
                            )
                            .await;
                        }
                        matrix_barrier(&mut b, &format!("{case} source")).await;
                        matrix_barrier(&mut a, &format!("{case} observer")).await;
                        assert_eq!(b.deadline.expires(), expires);
                        assert_eq!(b.deadline.is_committed(), registered);
                        assert_eq!(
                            hub.clients.lock().await.len(),
                            if registered { 2 } else { 1 }
                        );
                        cases[usize::from(registered)] += 1;
                    }
                    stopped(&mut hub).await;
                }
            }
        }
    }
    assert_eq!(cases, [96, 96]);
}

#[tokio::test]
async fn proprietary_transit_encoded_caps_not_npdu_and_ingress_boundary() {
    let tls = TestTls::new();
    let (mut hub, mut a, mut c) = matrix_pair(&tls, true, 0x42, 0x44).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    connect_limits(&mut b, 0x43, 1600, 1).await;
    for (dest, cap) in [
        ([0x43; 6], 1600),
        ([0x44; 6], usize::from(HUB_MAX_BVLC_LENGTH)),
    ] {
        for extra in [0, 1] {
            // Opaque proprietary bodies are never NPDUs.
            let mut body = valid_proprietary();
            body.resize(cap - 10 + extra, b'x');
            assert!(body.len() > 1497);
            send(&mut a, raw(12, 0, None, Some(dest), 0, &body)).await;
            if extra == 0 {
                let target = if dest == [0x43; 6] { &mut b } else { &mut c };
                assert_eq!(
                    recv(target).await,
                    raw(12, 0, Some([0x42; 6]), None, 0, &body)
                );
            }
            barrier(&mut a).await;
            barrier(&mut b).await;
            barrier(&mut c).await;
        }
    }
    // Broadcast preserves the destination with origin-stamp; a small body
    // fits every recipient cap here (large-broadcast caps share the same
    // BVLC-only path as the unicast cases above).
    let body = valid_proprietary();
    send(&mut a, raw(12, 1, None, Some(BROADCAST_VMAC), 0, &body)).await;
    assert_eq!(
        recv(&mut b).await,
        raw(12, 1, Some([0x42; 6]), Some(BROADCAST_VMAC), 0, &body)
    );
    assert_eq!(
        recv(&mut c).await,
        raw(12, 1, Some([0x42; 6]), Some(BROADCAST_VMAC), 0, &body)
    );
    // Generic decoding remains the only syntax prerequisite for transit.
    for wire in [
        vec![12],
        vec![12, 0x80, 0, 0],
        vec![12, 4, 0, 0, 0x43],
        vec![12, 2, 0, 0, 0x9E],
    ] {
        assert!(decode_sc_message(&wire).is_err());
        send(&mut a, wire).await;
    }
    // ResultFor12 retains the exact final BVLC cap and existing Result syntax.
    for extra in [0, 1] {
        let mut body = vec![12, 1, 0, 0, 7, 0, 144];
        body.resize(1590 + extra, b'x');
        send(&mut a, raw(0, u16::MAX, None, Some([0x43; 6]), 0, &body)).await;
        if extra == 0 {
            assert_eq!(
                recv(&mut b).await,
                raw(0, u16::MAX, Some([0x42; 6]), None, 0, &body)
            );
        }
        barrier(&mut a).await;
        barrier(&mut b).await;
        barrier(&mut c).await;
    }
    stopped(&mut hub).await;
}

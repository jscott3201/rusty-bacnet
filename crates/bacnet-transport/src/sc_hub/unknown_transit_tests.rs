//! Independent raw vectors through the supervised hub handler over mTLS.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::shutdown_blocked_tests::ControlledPeer;
use super::*;
use crate::sc_frame::BROADCAST_VMAC;
use std::time::Duration;

pub(super) fn raw(
    function: u8,
    id: u16,
    origin: Option<Vmac>,
    dest: Option<Vmac>,
    flags: u8,
    body: &[u8],
) -> Vec<u8> {
    let mut wire = vec![
        function,
        (u8::from(origin.is_some()) * 8) | (u8::from(dest.is_some()) * 4) | flags,
    ];
    wire.extend_from_slice(&id.to_be_bytes());
    if let Some(vmac) = origin {
        wire.extend_from_slice(&vmac);
    }
    if let Some(vmac) = dest {
        wire.extend_from_slice(&vmac);
    }
    wire.extend_from_slice(body);
    wire
}

pub(super) async fn send(peer: &mut ControlledPeer, wire: Vec<u8>) {
    peer.ws.send(Message::Binary(wire.into())).await.unwrap();
}

pub(super) async fn recv(peer: &mut ControlledPeer) -> Vec<u8> {
    match poll_io(peer.ws.next()).await.unwrap().unwrap() {
        Message::Binary(data) => data.to_vec(),
        other => panic!("expected binary, got {other:?}"),
    }
}

pub(super) async fn barrier(peer: &mut ControlledPeer) {
    // Invalid Disconnect has a known, activity-free NAK. In-order dispatch
    // makes this a no-extra-output assertion rather than a timing-only silence.
    send(peer, vec![8, 0, 0x55, 0x66, 0x42]).await;
    assert_eq!(recv(peer).await, [0, 0, 0x55, 0x66, 8, 1, 0, 0, 7, 0, 7]);
}

pub(super) async fn stopped(hub: &mut CountedHub) {
    poll_io(hub.hub.stop()).await;
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    assert!(hub.clients.lock().await.is_empty());
    assert_eq!(hub.hub.tasks.len(), 0);
}

// Semantic matrices must not consume an entire real Connect deadline across
// hundreds of cases. Each fresh fixture handles only two body/result variants.
// The observer is registered before the source's five-second deadline starts.
pub(super) async fn matrix_pair(
    tls: &TestTls,
    registered: bool,
    source_id: u8,
    observer_id: u8,
) -> (CountedHub, ControlledPeer, ControlledPeer) {
    let hub = CountedHub::start(tls, ScHubHandshakeTimeouts::default()).await;
    let mut observer = ControlledPeer::open(tls, &hub).await;
    observer.connect(observer_id).await;
    let mut source = ControlledPeer::open(tls, &hub).await;
    if registered {
        source.connect(source_id).await;
    }
    (hub, source, observer)
}

pub(super) async fn matrix_receive(peer: &mut ControlledPeer, expected: &[u8], case: &str) {
    let received = poll_io(peer.ws.next()).await;
    assert!(
        matches!(&received, Some(Ok(Message::Binary(data))) if data.as_ref() == expected),
        "{case}: expected {expected:02x?}, got {received:?}; now={:?}, expires={:?}, \
         expired={}, committed={}, received_frames={}",
        tokio::time::Instant::now(),
        peer.deadline.expires(),
        peer.deadline.expired(),
        peer.deadline.is_committed(),
        peer.deadline.received.load(Ordering::Acquire),
    );
}

pub(super) async fn matrix_barrier(peer: &mut ControlledPeer, case: &str) {
    send(peer, vec![8, 0, 0x55, 0x66, 0x42]).await;
    matrix_receive(peer, &[0, 0, 0x55, 0x66, 8, 1, 0, 0, 7, 0, 7], case).await;
}

pub(super) async fn connect_limits(peer: &mut ControlledPeer, id: u8, bvlc: u16, npdu: u16) {
    let Message::Binary(wire) = request([id; 6], [id; 16]) else {
        unreachable!()
    };
    let mut wire = wire.to_vec();
    wire[26..28].copy_from_slice(&bvlc.to_be_bytes());
    wire[28..30].copy_from_slice(&npdu.to_be_bytes());
    send(peer, wire).await;
    assert_eq!(recv(peer).await[0], 7);
}

#[tokio::test]
async fn unknown_transit_addressed_unicast_reaches_peer() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut source = ControlledPeer::open(&tls, &hub).await;
    source.connect(0x42).await;
    let mut target = ControlledPeer::open(&tls, &hub).await;
    target.connect(0x43).await;
    let mut wire = vec![0x0D, 4, 0x22, 0x33];
    wire.extend_from_slice(&[0x43; 6]);
    wire.extend_from_slice(&[0xDE, 0xAD]);
    source.ws.send(Message::Binary(wire.into())).await.unwrap();
    let received = tokio::time::timeout(Duration::from_millis(200), target.ws.next()).await;
    // Join every supervised worker even on the intended compiled RED failure.
    poll_io(hub.hub.stop()).await;
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    assert!(hub.clients.lock().await.is_empty());
    assert_eq!(hub.hub.tasks.len(), 0);
    let mut expected = vec![0x0D, 8, 0x22, 0x33];
    expected.extend_from_slice(&[0x42; 6]);
    expected.extend_from_slice(&[0xDE, 0xAD]);
    assert!(
        matches!(received, Ok(Some(Ok(Message::Binary(data)))) if data.as_ref() == expected),
        "addressed Unknown must transit, not produce a hub-local NAK"
    );
}

#[tokio::test]
async fn unknown_transit_all_243_functions_unicast_broadcast_raw_options_no_echo() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut a = ControlledPeer::open(&tls, &hub).await;
    a.connect(0x42).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    b.connect(0x43).await;
    let mut c = ControlledPeer::open(&tls, &hub).await;
    c.connect(0x44).await;
    for function in 0x0D..=0xFF {
        for dest in [[0x43; 6], BROADCAST_VMAC] {
            // Every raw function, both directions, including an empty body.
            let body = if function % 2 == 0 {
                &b"\xDE\xAD\xFF"[..]
            } else {
                &[]
            };
            send(
                &mut a,
                raw(function, u16::from(function), None, Some(dest), 0, body),
            )
            .await;
            let expected = raw(
                function,
                u16::from(function),
                Some([0x42; 6]),
                (dest == BROADCAST_VMAC).then_some(dest),
                0,
                body,
            );
            assert_eq!(recv(&mut b).await, expected);
            if dest == BROADCAST_VMAC {
                assert_eq!(recv(&mut c).await, expected);
            }
            barrier(&mut a).await;
            barrier(&mut c).await;
        }
    }
    for function in [0x0D, 0x42, 0xFF] {
        for id in [0, u16::MAX] {
            for dest in [[0x43; 6], BROADCAST_VMAC] {
                for (flags, options) in [
                    (0, vec![]),
                    (2, vec![0x5E]),
                    (1, vec![0x3E, 0, 0]),
                    (3, vec![0xE2, 0, 0, 0x1F, 0xFE, 0, 1, 0xBB, 0x1F]),
                ] {
                    for payload in [&[][..], &[0xFF, 0, 1][..]] {
                        let mut body = options.clone();
                        body.extend_from_slice(payload);
                        let wire = raw(function, id, None, Some(dest), flags, &body);
                        assert_eq!(decode_sc_message(&wire).unwrap().payload.as_ref(), payload);
                        send(&mut a, wire).await;
                        let expected = raw(
                            function,
                            id,
                            Some([0x42; 6]),
                            (dest == BROADCAST_VMAC).then_some(dest),
                            flags,
                            &body,
                        );
                        assert_eq!(recv(&mut b).await, expected);
                        if dest == BROADCAST_VMAC {
                            assert_eq!(recv(&mut c).await, expected);
                        }
                        barrier(&mut a).await;
                        barrier(&mut c).await;
                    }
                }
            }
        }
    }
    stopped(&mut hub).await;
}

#[tokio::test]
async fn unknown_transit_local_preregistered_and_invalid_envelopes_no_state_effects() {
    use super::response_silence_tests::Snapshot;
    let tls = TestTls::new();
    let mut cases = [0; 2];
    for registered in [false, true] {
        for function in [0x0D, 0x42, 0xFF] {
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
                        if registered
                            && dest.is_some()
                            && origin.is_none()
                            && dest != Some([0x42; 6])
                        {
                            continue;
                        }
                        let (mut hub, mut a, mut b) =
                            matrix_pair(&tls, registered, 0x42, 0x43).await;
                        let expires = a.deadline.expires();
                        let before: Vec<_> = hub
                            .clients
                            .lock()
                            .await
                            .iter()
                            .map(|(vmac, client)| (*vmac, Snapshot::capture(client)))
                            .collect();
                        for body in [
                            &[0xE2, 0, 0, 0x1F, 0x7E, 0, 0][..],
                            &[0xE2, 0, 0, 0x1F, 0x7E, 0, 0, 0xFF][..],
                        ] {
                            let case = format!("local registered={registered} function={function:#04x} id={id} origin={origin:02x?} dest={dest:02x?} body={body:02x?}");
                            send(&mut a, raw(function, id, origin, dest, 3, body)).await;
                            let eligible = (!registered || dest.is_none())
                                && dest != Some(BROADCAST_VMAC)
                                && origin != Some([0; 6])
                                && origin != Some(BROADCAST_VMAC);
                            if eligible {
                                matrix_receive(
                                    &mut a,
                                    &raw(0, id, None, origin, 0, &[function, 1, 0, 0, 7, 0, 143]),
                                    &format!("{case} source NAK"),
                                )
                                .await;
                            }
                            matrix_barrier(&mut a, &format!("{case} source barrier")).await;
                            // Origin metadata never redirects the local NAK.
                            matrix_barrier(&mut b, &format!("{case} observer barrier")).await;
                            assert_eq!(a.deadline.expires(), expires);
                            assert_eq!(a.deadline.is_committed(), registered);
                            let map = hub.clients.lock().await;
                            assert_eq!(map.len(), if registered { 2 } else { 1 });
                            for (vmac, snapshot) in &before {
                                snapshot.unchanged(map.get(vmac).unwrap());
                            }
                            cases[usize::from(registered)] += 1;
                        }
                        if !registered {
                            // The same socket can still Connect after local diagnostics.
                            a.connect(0x42).await;
                            assert!(a.deadline.is_committed());
                            assert_eq!(a.deadline.expires(), expires);
                            assert_eq!(hub.clients.lock().await.len(), 2);
                        }
                        stopped(&mut hub).await;
                    }
                }
            }
        }
    }
    assert_eq!(cases, [288, 240]);
    // Known-but-unhandled remains the old connection-local 7/150 fallback.
    let (mut hub, mut a, mut b) = matrix_pair(&tls, true, 0x42, 0x43).await;
    for function in [4, 5, 12] {
        send(&mut a, raw(function, 7, None, Some([0x43; 6]), 0, &[])).await;
        assert_eq!(
            recv(&mut a).await,
            raw(0, 7, None, None, 0, &[function, 1, 0, 0, 7, 0, 150])
        );
        barrier(&mut b).await;
    }
    stopped(&mut hub).await;
}

#[tokio::test]
async fn unknown_transit_encoded_caps_not_npdu_caps_and_ingress_boundary() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut a = ControlledPeer::open(&tls, &hub).await;
    a.connect(0x42).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    connect_limits(&mut b, 0x43, 1600, 1).await;
    let mut c = ControlledPeer::open(&tls, &hub).await;
    connect_limits(&mut c, 0x44, 6000, 1).await;
    for dest in [[0x43; 6], BROADCAST_VMAC] {
        let broadcast = dest == BROADCAST_VMAC;
        for extra in [0, 1] {
            let payload = vec![0xFF; 1600 - if broadcast { 16 } else { 10 } + extra];
            assert!(payload.len() > 1497);
            send(&mut a, raw(0xFF, 0, None, Some(dest), 0, &payload)).await;
            let expected = raw(
                0xFF,
                0,
                Some([0x42; 6]),
                broadcast.then_some(dest),
                0,
                &payload,
            );
            assert_eq!(expected.len(), 1600 + extra);
            if extra == 0 {
                assert_eq!(recv(&mut b).await, expected);
            }
            if broadcast {
                assert_eq!(recv(&mut c).await, expected);
            }
            barrier(&mut a).await;
            barrier(&mut b).await;
            barrier(&mut c).await;
        }
    }
    for dest in [[0x44; 6], BROADCAST_VMAC] {
        for extra in [0, 1] {
            let payload = vec![0xFF; usize::from(HUB_MAX_BVLC_LENGTH) - 10 + extra];
            send(&mut a, raw(0x0D, u16::MAX, None, Some(dest), 0, &payload)).await;
            if extra == 0 {
                assert_eq!(
                    recv(&mut c).await,
                    raw(
                        0x0D,
                        u16::MAX,
                        Some([0x42; 6]),
                        (dest == BROADCAST_VMAC).then_some(dest),
                        0,
                        &payload
                    )
                );
            }
            barrier(&mut a).await;
            barrier(&mut b).await;
            barrier(&mut c).await;
        }
    }
    // ResultForUnknown applies the recipient BVLC cap, not its Max-NPDU=1.
    for extra in [0, 1] {
        let mut body = vec![0x0D, 1, 0, 0, 7, 0, 143];
        body.resize(1590 + extra, b'x');
        send(&mut a, raw(0, 8, None, Some([0x43; 6]), 0, &body)).await;
        if extra == 0 {
            assert_eq!(
                recv(&mut b).await,
                raw(0, 8, Some([0x42; 6]), None, 0, &body)
            );
        }
        barrier(&mut a).await;
        barrier(&mut b).await;
    }
    stopped(&mut hub).await;
}

#[tokio::test]
async fn unknown_transit_result_ack_nak_routing_and_invalid_result_silence() {
    let tls = TestTls::new();
    let mut cases = [0; 2];
    for registered in [false, true] {
        for function in [0x0D, 0x42, 0xFF] {
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
                        let (mut hub, mut b, mut a) =
                            matrix_pair(&tls, registered, 0x43, 0x42).await;
                        let expires = b.deadline.expires();
                        for result in [
                            vec![function, 0],
                            vec![function, 1, 0xFE, 0, 7, 0, 143, b'x', 0xC3, 0xA9],
                        ] {
                            let mut body = vec![0xFE, 0, 0, 0x1F]; // MU/MoreOptions/empty Header Data
                            body.extend_from_slice(&result);
                            let case = format!("result registered={registered} function={function:#04x} id={id} origin={origin:02x?} dest={dest:02x?} body={body:02x?}");
                            send(&mut b, raw(0, id, origin, dest, 2, &body)).await;
                            if registered && origin.is_none() && dest == Some([0x42; 6]) {
                                matrix_receive(
                                    &mut a,
                                    &raw(0, id, Some([0x43; 6]), None, 2, &body),
                                    &format!("{case} observer relay"),
                                )
                                .await;
                            }
                            matrix_barrier(&mut b, &format!("{case} source barrier")).await;
                            matrix_barrier(&mut a, &format!("{case} observer barrier")).await;
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
    }
    assert_eq!(cases, [288, 288]);
    let (mut hub, mut b, mut a) = matrix_pair(&tls, true, 0x43, 0x42).await;
    for body in [
        vec![],
        vec![0x42],
        vec![0x42, 2],
        vec![0x42, 0, 0],
        vec![0x42, 1, 0, 0, 7, 0],
        vec![0x42, 1, 0, 0, 7, 0, 143, 0xFF],
    ] {
        send(&mut b, raw(0, 0, None, Some([0x42; 6]), 0, &body)).await;
    }
    send(
        &mut b,
        raw(0, 0, None, Some([0x42; 6]), 1, &[0x1E, 0x42, 0]),
    )
    .await;
    for function in [0, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
        send(&mut b, raw(0, 0, None, Some([0x42; 6]), 0, &[function, 0])).await;
    }
    barrier(&mut b).await;
    barrier(&mut a).await;
    // Existing EncapsulatedNpdu Result relay is still accepted.
    send(&mut b, raw(0, 7, None, Some([0x42; 6]), 0, &[1, 0])).await;
    assert_eq!(
        recv(&mut a).await,
        raw(0, 7, Some([0x43; 6]), None, 0, &[1, 0])
    );
    stopped(&mut hub).await;
}

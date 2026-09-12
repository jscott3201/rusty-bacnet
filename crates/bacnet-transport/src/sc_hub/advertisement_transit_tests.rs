//! Hub-only Advertisement/Solicitation transit, using independent raw wire
//! expectations. Registered unicast stays opaque (AB.5.3.2 forwarding);
//! the shape validator only owns hub-local and node destinations.
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

fn valid_advertisement() -> Vec<u8> {
    vec![1, 1, 0x05, 0xC4, 0x05, 0xC4]
}

async fn addressed_empty(function: u8) {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    b.connect(0x43).await;
    let mut a = ControlledPeer::open(&tls, &hub).await;
    a.connect(0x42).await;
    let body = if function == 4 {
        valid_advertisement()
    } else {
        vec![]
    };
    send(&mut a, raw(function, 0, None, Some([0x43; 6]), 0, &body)).await;
    let received = tokio::time::timeout(Duration::from_millis(200), b.ws.next()).await;
    // Compiled RED must release every supervised worker before asserting.
    stopped(&mut hub).await;
    let expected = raw(function, 0, Some([0x42; 6]), None, 0, &body);
    assert!(
        matches!(&received, Ok(Some(Ok(Message::Binary(data)))) if data.as_ref() == expected),
        "function {function:#04x}: addressed well-formed frame must transit, not be locally rejected; got {received:?}"
    );
}

#[tokio::test]
async fn advertisement_transit_addressed_wellformed_request() {
    addressed_empty(4).await;
}

#[tokio::test]
async fn advertisement_transit_addressed_wellformed_solicitation() {
    addressed_empty(5).await;
}

#[tokio::test]
async fn advertisement_transit_opaque_options_shapes_and_no_echo() {
    let tls = TestTls::new();
    let (mut hub, mut a, mut b) = matrix_pair(&tls, true, 0x42, 0x43).await;
    let mut c = ControlledPeer::open(&tls, &hub).await;
    c.connect(0x44).await;
    let mut cases = 0;
    for function in [4, 5] {
        for id in [0, u16::MAX] {
            for (flags, options) in [
                (0, vec![]),
                (2, vec![0x5E]),
                (2, vec![0xFE, 0, 0, 0x1F]), // MU, MoreOptions, empty HeaderData
                (3, vec![0xE2, 0, 0, 0x1F, 0xFE, 0, 1, 0xBB, 0x1F]),
            ] {
                // Transit stays opaque: even forbidden shapes relay when the
                // envelope is an eligible registered unicast. The destination
                // validator owns the NAK, not the forwarding hub.
                for payload in [
                    &b""[..],
                    &valid_advertisement()[..],
                    &[9, 9, 0, 1, 0, 1][..],
                    &[0xFF; 9][..],
                ] {
                    let mut body = options.clone();
                    body.extend_from_slice(payload);
                    let wire = raw(function, id, None, Some([0x43; 6]), flags, &body);
                    assert_eq!(decode_sc_message(&wire).unwrap().payload.as_ref(), payload);
                    send(&mut a, wire).await;
                    assert_eq!(
                        recv(&mut b).await,
                        raw(function, id, Some([0x42; 6]), None, flags, &body)
                    );
                    barrier(&mut a).await;
                    barrier(&mut c).await;
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 64);
    stopped(&mut hub).await;
}

#[tokio::test]
async fn advertisement_transit_local_preregistered_invalid_silence_and_state_matrix() {
    let tls = TestTls::new();
    let mut cases = [0; 2];
    for registered in [false, true] {
        for function in [4, 5] {
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
                            && origin.is_none()
                            && matches!(dest, Some(v) if v != [0x42; 6] && v != BROADCAST_VMAC)
                        {
                            continue; // accepted transit activity is tested separately
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
                        // Well-formed local requests draw UNEXPECTED_DATA; the
                        // malformed body draws its shape code. Both stay local.
                        let valid_body: Vec<u8> = if function == 4 {
                            valid_advertisement()
                        } else {
                            vec![]
                        };
                        for (flags, body, code) in [
                            (0, valid_body.clone(), 150u16),
                            (0, vec![], if function == 4 { 149 } else { 150 }),
                        ] {
                            let case = format!("advertisement registered={registered} function={function} id={id} origin={origin:02x?} dest={dest:02x?} flags={flags} body={body:02x?}");
                            send(&mut a, raw(function, id, origin, dest, flags, &body)).await;
                            if (!registered || dest.is_none())
                                && dest != Some(BROADCAST_VMAC)
                                && origin != Some([0; 6])
                                && origin != Some(BROADCAST_VMAC)
                            {
                                let [hi, lo] = code.to_be_bytes();
                                matrix_receive(
                                    &mut a,
                                    &raw(0, id, None, origin, 0, &[function, 1, 0, 0, 7, hi, lo]),
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
    }
    // Registered skips 2 functions * 2 IDs * 3 accepted destinations * 2 bodies.
    assert_eq!(cases, [192, 168]);
}

#[tokio::test]
async fn advertisement_transit_result_for_4_and_5_relay_and_others_drop() {
    let tls = TestTls::new();
    let mut cases = [0; 2];
    for registered in [false, true] {
        for result_for in [4, 5] {
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
                            vec![result_for, 0],
                            vec![result_for, 1, 0xFE, 0, 7, 0, 150, b'x', 0xC3, 0xA9],
                        ] {
                            let mut body = vec![0xFE, 0, 0, 0x1F];
                            body.extend_from_slice(&result);
                            let case = format!("ResultFor={result_for} registered={registered} id={id} origin={origin:02x?} dest={dest:02x?} result={result:02x?}");
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
    }
    assert_eq!(cases, [192, 192]);
    // Results for responses and other known functions stay dropped.
    // Proprietary (0x0C) graduated to its own transit family and relays.
    let (mut hub, mut b, mut a) = matrix_pair(&tls, true, 0x43, 0x42).await;
    for result_for in [0, 3, 6, 7, 8, 9, 10, 11] {
        send(
            &mut b,
            raw(0, 0, None, Some([0x42; 6]), 0, &[result_for, 0]),
        )
        .await;
    }
    for body in [
        vec![],
        vec![4],
        vec![4, 2],
        vec![4, 1, 0, 0, 7, 0],
        vec![5, 1, 0, 0, 7, 0, 150, 0xFF],
    ] {
        send(&mut b, raw(0, 0, None, Some([0x42; 6]), 0, &body)).await;
    }
    send(&mut b, raw(0, 0, None, Some([0x42; 6]), 1, &[0x1E, 4, 0])).await;
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

#[tokio::test]
async fn advertisement_transit_encoded_caps_not_npdu_and_ingress_boundary() {
    let tls = TestTls::new();
    let (mut hub, mut a, mut c) = matrix_pair(&tls, true, 0x42, 0x44).await;
    let mut b = ControlledPeer::open(&tls, &hub).await;
    connect_limits(&mut b, 0x43, 1600, 1).await;
    for function in [4, 5] {
        for (dest, cap) in [
            ([0x43; 6], 1600),
            ([0x44; 6], usize::from(HUB_MAX_BVLC_LENGTH)),
        ] {
            for extra in [0, 1] {
                // Opaque advertisement bodies are never NPDUs.
                let mut body = valid_advertisement();
                body.resize(cap - 10 + extra, b'x');
                assert!(body.len() > 1497);
                send(&mut a, raw(function, 0, None, Some(dest), 0, &body)).await;
                if extra == 0 {
                    let target = if dest == [0x43; 6] { &mut b } else { &mut c };
                    assert_eq!(
                        recv(target).await,
                        raw(function, 0, Some([0x42; 6]), None, 0, &body)
                    );
                }
                barrier(&mut a).await;
                barrier(&mut b).await;
                barrier(&mut c).await;
            }
        }
        // Generic decoding remains the only syntax prerequisite for transit.
        for wire in [
            vec![function],
            vec![function, 0x80, 0, 0],
            vec![function, 4, 0, 0, 0x43],
            vec![function, 2, 0, 0, 0x9E],
        ] {
            assert!(decode_sc_message(&wire).is_err());
            send(&mut a, wire).await;
        }
    }
    // ResultFor4/5 retain the exact final BVLC cap and existing Result syntax.
    for result_for in [4, 5] {
        for extra in [0, 1] {
            let mut body = vec![result_for, 1, 0, 0, 7, 0, 150];
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
    }
    stopped(&mut hub).await;
}

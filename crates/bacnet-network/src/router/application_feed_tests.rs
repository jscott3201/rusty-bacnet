//! The router's application entity split between a client and a server
//! (#2069): where each delivery goes, invoke IDs the two stacks share, and
//! a stalled consumer; and the ports following the router's limits (#2070).

use std::borrow::Cow;

use bacnet_npdu::{Address, NpduLimits};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant, advance, timeout};

use super::super::application_transactions::{
    CAPACITY, CLOSED, LIFETIME, Owner, Side, Transactions,
};
use super::super::priority_harness::*;
use super::super::{RouterApplicationPorts, RouterOptions};
use super::{QUEUE, Route, STALL, classify, outbound};
use crate::layer::{AdmissionReceiver, ReceivedApdu};

const STATION_7: Address<'static> = Address {
    network: 2000,
    mac: &[7],
};

/// Every PDU type, with the server bit clear and set.
#[test]
fn each_pdu_type_goes_where_it_belongs() {
    use Route::*;
    let client = To(Side::Client);
    let server = To(Side::Server);
    let answer = |ends| Answer { invoke_id: 9, ends };
    let cases: [(&[u8], Route); 23] = [
        // Confirmed-Requests: ReadProperty and audit notifications (32) to
        // the server, COV notifications (1) to the client, event
        // notifications (2) to both, segmented ones too.
        (&[0x00, 0x05, 9, 0x0C], server),
        (&[0x00, 0x05, 9, 0x01], client),
        (&[0x00, 0x05, 9, 0x02], ServerAnswers),
        (&[0x08, 0x05, 9, 0, 1, 0x02], ServerAnswers),
        (&[0x00, 0x05, 9, 0x20], server),
        (&[0x08, 0x05, 9, 0, 1, 0x0C], server),
        (&[0x00, 0x05, 9], server),
        // Unconfirmed-Requests, either bit.
        (&[0x10, 0x00], Both),
        (&[0x11, 0x08], Both),
        // Answers end the request, but a Complex-ACK with more to follow.
        (&[0x20, 9, 0x0F], answer(true)),
        (&[0x21, 9, 0x0F], answer(true)),
        (&[0x30, 9, 0x0C], answer(true)),
        (&[0x3C, 9, 0, 1, 0x0C], answer(false)),
        (&[0x38, 9, 0, 1, 0x0C], answer(true)),
        (&[0x50, 9, 0x0C], answer(true)),
        (&[0x60, 9, 0x01], answer(true)),
        // Segment-ACK and Abort by their server bit.
        (&[0x41, 9, 0, 1], answer(false)),
        (&[0x40, 9, 0, 1], server),
        (&[0x71, 9, 0x04], answer(true)),
        (&[0x70, 9, 0x04], server),
        // Reserved types, and too short to answer.
        (&[0x80, 9], Neither),
        (&[0xF0, 9], Neither),
        (&[0x20], Neither),
    ];
    for (apdu, route) in cases {
        assert_eq!(classify(apdu), route, "{apdu:02x?}");
    }
    assert_eq!(classify(&[]), Neither);
}

/// Requests to one device under one invoke ID from both stacks: the later
/// goes out under the next free ID, and the answers come back to each,
/// also for a while after the final one.
#[tokio::test(start_paused = true)]
async fn shared_invoke_ids_are_kept_apart() {
    let transactions = Transactions::default();
    assert_eq!(transactions.request(Side::Server, STATION_7, 3), Some(3));
    assert_eq!(transactions.request(Side::Client, STATION_7, 3), Some(4));
    // A retry, or the next segment, keeps its ID.
    assert_eq!(transactions.request(Side::Client, STATION_7, 3), Some(4));
    // The client's own 4 now needs another.
    assert_eq!(transactions.request(Side::Client, STATION_7, 4), Some(5));
    // Another device has every ID free.
    let other = Address {
        network: 2000,
        mac: &[8],
    };
    assert_eq!(transactions.request(Side::Client, other, 3), Some(3));
    // Its requester's Segment-ACK follows the same ID; an unknown one
    // goes as it is.
    assert_eq!(transactions.requester(Side::Client, STATION_7, 3, false), 4);
    assert_eq!(transactions.requester(Side::Server, STATION_7, 9, false), 9);

    // A segment of the answer leaves the request open; the last closes it.
    let client_3 = Owner::Stack(Side::Client, 3);
    assert_eq!(transactions.answer(2000, &[7], 4, false), client_3);
    assert_eq!(transactions.answer(2000, &[7], 4, true), client_3);
    // Closed, it still translates the final Segment-ACK, and a repeat of
    // the final segment, and that ACK's repeat.
    assert_eq!(transactions.requester(Side::Client, STATION_7, 3, false), 4);
    assert_eq!(transactions.answer(2000, &[7], 4, true), client_3);
    assert_eq!(transactions.requester(Side::Client, STATION_7, 3, false), 4);
    assert_eq!(
        transactions.answer(2000, &[7], 3, true),
        Owner::Stack(Side::Server, 3)
    );
    // And keeps 4 from the other stack meanwhile.
    assert_eq!(transactions.request(Side::Server, STATION_7, 4), Some(6));
    // A requester's Abort closes its request, which still matches.
    assert_eq!(transactions.requester(Side::Client, STATION_7, 4, true), 5);
    assert_eq!(
        transactions.answer(2000, &[7], 5, true),
        Owner::Stack(Side::Client, 4)
    );
    assert_eq!(transactions.answer(2000, &[7], 77, true), Owner::Unknown);

    // Once the closed ones are gone: an answer under 6 is the server's
    // open one; one under 4 matches nothing, but the server knows its
    // request under 6 as 4, so it can't go up unchanged.
    advance(CLOSED).await;
    assert_eq!(
        transactions.answer(2000, &[7], 6, false),
        Owner::Stack(Side::Server, 4)
    );
    assert_eq!(transactions.answer(2000, &[7], 4, true), Owner::Ambiguous);
    assert_eq!(transactions.answer(2000, &[7], 3, true), Owner::Unknown);
    // A closed request's own stack reopens it under the same ID.
    transactions.answer(2000, &[7], 6, true);
    assert_eq!(transactions.request(Side::Server, STATION_7, 4), Some(6));
    advance(CLOSED).await;
    assert_eq!(
        transactions.answer(2000, &[7], 6, true),
        Owner::Stack(Side::Server, 4),
        "open again, so kept past CLOSED"
    );
    assert_eq!(
        transactions.len(),
        2,
        "the server's 4 and the other device's"
    );
}

/// The bytes a stack sends, with the ID they go out under.
#[tokio::test]
async fn outbound_apdus_carry_the_wire_id() {
    let transactions = Transactions::default();
    let read = [0x00, 0x05, 3, 0x0C];
    assert!(matches!(
        outbound(&transactions, Side::Server, STATION_7, &read).unwrap(),
        Cow::Borrowed(_)
    ));
    let sent = outbound(&transactions, Side::Client, STATION_7, &read).unwrap();
    assert_eq!(&sent[..], [0x00, 0x05, 4, 0x0C]);
    // The client's Segment-ACK and Abort as requester, translated.
    let sent = outbound(&transactions, Side::Client, STATION_7, &[0x40, 3, 0, 1]).unwrap();
    assert_eq!(&sent[..], [0x40, 4, 0, 1]);
    // As the server, they answer the device's own request: unchanged.
    let sent = outbound(&transactions, Side::Client, STATION_7, &[0x41, 3, 0, 1]).unwrap();
    assert_eq!(&sent[..], [0x41, 3, 0, 1]);
    let sent = outbound(&transactions, Side::Client, STATION_7, &[0x70, 3, 0x04]).unwrap();
    assert_eq!(&sent[..], [0x70, 4, 0x04]);
    // A broadcast is no transaction.
    let everyone = Address {
        network: 2000,
        mac: &[],
    };
    let sent = outbound(&transactions, Side::Client, everyone, &read).unwrap();
    assert_eq!(&sent[..], read);
    assert_eq!(
        transactions.len(),
        2,
        "the server's request, and the client's, closed by its Abort"
    );

    // Every ID to the device in use: a new request is refused.
    for id in 0..=u8::MAX {
        let _ = transactions.request(Side::Server, STATION_7, id);
    }
    let error = outbound(
        &transactions,
        Side::Client,
        STATION_7,
        &[0x00, 0x05, 9, 0x0C],
    )
    .unwrap_err();
    assert!(
        matches!(&error, bacnet_types::error::Error::Transport(io) if io.kind() == std::io::ErrorKind::WouldBlock),
        "{error:?}"
    );
}

/// Requests are forgotten once idle for the lifetime, and the table holds
/// so many, pushing out the one used least recently.
#[tokio::test(start_paused = true)]
async fn open_requests_expire_and_are_bounded() {
    let transactions = Transactions::default();
    transactions.request(Side::Server, STATION_7, 1);
    advance(LIFETIME - Duration::from_secs(1)).await;
    // Used again: its lifetime starts over.
    transactions.request(Side::Server, STATION_7, 1);
    advance(LIFETIME - Duration::from_secs(1)).await;
    assert_eq!(
        transactions.answer(2000, &[7], 1, false),
        Owner::Stack(Side::Server, 1)
    );
    advance(LIFETIME).await;
    assert_eq!(
        transactions.answer(2000, &[7], 1, true),
        Owner::Unknown,
        "expired"
    );

    for index in 0..=CAPACITY {
        let mac = (index as u16).to_be_bytes();
        let to = Address {
            network: 2000,
            mac: &mac,
        };
        transactions.request(Side::Client, to, 0);
    }
    assert_eq!(transactions.len(), CAPACITY);
    assert_eq!(
        transactions.answer(2000, &[0, 0], 0, true),
        Owner::Unknown,
        "pushed out"
    );
    assert_eq!(
        transactions.answer(2000, &[0, 1], 0, true),
        Owner::Stack(Side::Client, 0)
    );
}

/// The two ports on a router with two driven ports, both started.
async fn split() -> (
    super::super::BACnetRouter,
    RouterApplicationPorts,
    [mpsc::Receiver<ReceivedNpdu>; 2],
    Vec<Peer>,
) {
    let (router, apdus, peers) = start(2).await;
    let mut ports = router.application_ports(apdus);
    let client = ports.client.start().await.unwrap();
    let server = ports.server.start().await.unwrap();
    (router, ports, [client, server], peers)
}

/// The next NPDU on `rx`, within a second.
async fn next(rx: &mut mpsc::Receiver<ReceivedNpdu>) -> Vec<u8> {
    timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("an NPDU")
        .expect("open")
        .npdu
        .to_vec()
}

/// Through the router: the server's request and the client's to one
/// device share an ID, and each answer reaches the stack that asked, under
/// the ID it chose. Requests and notifications go to their stacks.
#[tokio::test]
async fn each_answer_reaches_its_asker() {
    let (mut router, ports, [mut client, mut server], mut peers) = split().await;
    // The server's ConfirmedEventNotification and the client's
    // ReadProperty, both invoke ID 3, to station 7 on network 2000.
    let to_7 = [1, 0x24, 0x07, 0xD0, 1, 7, 255];
    for (port, apdu, wire) in [
        (&ports.server, [0x00, 0x05, 3, 0x02], 3),
        (&ports.client, [0x00, 0x05, 3, 0x0C], 4),
    ] {
        let npdu = [&to_7[..], &apdu].concat();
        let (sent, result) = tokio::join!(
            async {
                let sent = sending(&mut peers[1]).await;
                sent.complete.send(Ok(())).unwrap();
                sent.npdu
            },
            port.send_unicast(&npdu, &[0xEE])
        );
        result.unwrap();
        assert_eq!(sent, [1, 0x04, 0x00, 0x05, wire, apdu[3]]);
    }
    // Station 7 answers both: the Simple-ACK under 3 to the server, the
    // Complex-ACK under 4 to the client, given back as 3.
    receive(&peers[1], vec![1, 0, 0x20, 3, 0x02], 0).await;
    receive(&peers[1], vec![1, 0, 0x30, 4, 0x0C, 0xAA], 0).await;
    let from_7 = [1, 0x08, 0x07, 0xD0, 1, 7];
    assert_eq!(
        next(&mut server).await,
        [&from_7[..], &[0x20, 3, 0x02]].concat()
    );
    assert_eq!(
        next(&mut client).await,
        [&from_7[..], &[0x30, 3, 0x0C, 0xAA]].concat()
    );
    // A request goes to the server, a notification to the client, and an
    // I-Am to both.
    receive(&peers[1], vec![1, 0x04, 0x00, 0x05, 1, 0x0C], 0).await;
    receive(&peers[1], vec![1, 0x04, 0x00, 0x05, 2, 0x01], 0).await;
    receive(&peers[1], vec![1, 0, 0x10, 0x00], 0).await;
    assert_eq!(&next(&mut server).await[6..], [0x00, 0x05, 1, 0x0C]);
    assert_eq!(&next(&mut client).await[6..], [0x00, 0x05, 2, 0x01]);
    assert_eq!(&next(&mut server).await[6..], [0x10, 0x00]);
    assert_eq!(&next(&mut client).await[6..], [0x10, 0x00]);
    // Each keeps the reply plan of the request it got.
    assert_eq!(ports.server.saved_replies(), 1);
    assert_eq!(ports.client.saved_replies(), 1);
    // An answer nobody asked for goes to the client.
    receive(&peers[1], vec![1, 0, 0x20, 77, 0x0C], 0).await;
    assert_eq!(&next(&mut client).await[6..], [0x20, 77, 0x0C]);
    assert!(server.try_recv().is_err());
    router.stop().await.unwrap();
}

/// A server that stops taking what its port holds: the client waits for
/// it once, for [`STALL`], and then gets everything, while what comes for
/// the server is dropped until it has room again.
#[tokio::test(start_paused = true)]
async fn a_stalled_consumer_holds_the_other_up_once() {
    let (mut router, _ports, [mut client, mut server], peers) = split().await;
    let who_is = || vec![1, 0, 0x10, 0x08];
    // QUEUE fill the server's queue; the client takes each.
    for _ in 0..QUEUE {
        receive(&peers[0], who_is(), 0).await;
        next(&mut client).await;
    }
    let started = Instant::now();
    for _ in 0..3 * QUEUE {
        receive(&peers[0], who_is(), 0).await;
        next(&mut client).await;
    }
    let waited = started.elapsed();
    assert!(
        waited >= STALL && waited < 2 * STALL,
        "held up for {waited:?}"
    );
    // The server got only what fitted; it drains, and has room again.
    for _ in 0..QUEUE {
        server.recv().await.unwrap();
    }
    assert!(server.try_recv().is_err());
    receive(&peers[0], who_is(), 0).await;
    next(&mut client).await;
    assert_eq!(&next(&mut server).await[6..], [0x10, 0x08]);
    router.stop().await.unwrap();
}

/// With neither stack taking anything, the feed stops taking from the
/// router's queue, which keeps its accounting and fills.
#[tokio::test(start_paused = true)]
async fn with_no_room_anywhere_the_routers_queue_holds() {
    let options = RouterOptions::new().track_admission();
    let (mut router, apdus, peers): (_, AdmissionReceiver<ReceivedApdu>, _) =
        start_any(2, options).await;
    let counters = apdus.counters();
    let mut ports = router.application_ports(apdus);
    let _client = ports.client.start().await.unwrap();
    let _server = ports.server.start().await.unwrap();
    for _ in 0..QUEUE + 3 {
        receive(&peers[0], vec![1, 0, 0x10, 0x08], 0).await;
    }
    advance(10 * STALL).await;
    // Each queue holds QUEUE; the rest wait in the router's queue, counted.
    tokio::task::yield_now().await;
    assert_eq!(counters.snapshot().current_depth, 3);
    router.stop().await.unwrap();
}

/// A port follows the router ports' limits, capped, and publishes each
/// change once started; its receive capacity stays as it was made.
#[tokio::test]
async fn the_ports_follow_the_routers_limits() {
    let (mut router, apdus, _peers) = start(2).await;
    let RouterApplicationPorts { client, server, .. } = router.application_ports(apdus);
    let mut capped = server.with_max_npdu(NpduLimits::both(1000));
    let mut client = client;
    let mut changes = client.max_npdu_changes().unwrap();
    let mut capped_changes = capped.max_npdu_changes().unwrap();
    let _rx = client.start().await.unwrap();
    let _rx = capped.start().await.unwrap();
    assert_eq!(client.max_npdu(), NpduLimits::both(1497));
    assert_eq!(capped.max_npdu(), NpduLimits::both(1000));

    // Port 1 falls to 501.
    router
        .shared
        .set_port_limits(bacnet_router_core::PortId(1), NpduLimits::both(501));
    assert_eq!(client.max_npdu(), NpduLimits::both(501));
    assert_eq!(client.egress_apdu_limit(), 480);
    assert_eq!(client.local_receive_apdu_capacity(), 1476);
    assert_eq!(capped.max_npdu(), NpduLimits::both(501));
    assert_eq!(capped.local_receive_apdu_capacity(), 979);
    timeout(Duration::from_secs(1), changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*changes.borrow_and_update(), NpduLimits::both(501));
    timeout(Duration::from_secs(1), capped_changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*capped_changes.borrow_and_update(), NpduLimits::both(501));

    // Back up: the cap holds.
    router
        .shared
        .set_port_limits(bacnet_router_core::PortId(1), NpduLimits::both(1497));
    timeout(Duration::from_secs(1), capped_changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*capped_changes.borrow(), NpduLimits::both(1000));
    assert_eq!(client.max_npdu(), NpduLimits::both(1497));
    router.stop().await.unwrap();
}

/// Send `apdu` from `port` to station 7 on network 2000, expecting a reply
/// when `expecting`, and return the APDU the router sent there.
async fn to_station_7(
    port: &super::super::RouterApplicationPort,
    peer: &mut Peer,
    apdu: &[u8],
    expecting: bool,
) -> Vec<u8> {
    let control = if expecting { 0x24 } else { 0x20 };
    let npdu = [&[1, control, 0x07, 0xD0, 1, 7, 255][..], apdu].concat();
    let (sent, result) = tokio::join!(
        async {
            let sent = sending(peer).await;
            sent.complete.send(Ok(())).unwrap();
            assert_eq!(sent.to, Some(vec![7]));
            sent.npdu
        },
        port.send_unicast(&npdu, &[0xEE])
    );
    result.unwrap();
    sent[2..].to_vec()
}

/// A segmented Complex-ACK to the client's request, which went out under
/// another ID: every segment, repeats included, comes up under the
/// client's ID, and every Segment-ACK, the final one and its repeat
/// included, goes out under the wire ID. The server's answer, and its
/// repeat, still reach the server.
#[tokio::test]
async fn a_translated_segmented_answer_keeps_its_ids_to_the_end() {
    let (mut router, ports, [mut client, mut server], mut peers) = split().await;
    let peer = &mut peers[1];
    assert_eq!(
        to_station_7(&ports.server, peer, &[0x00, 0x05, 3, 0x02], true).await,
        [0x00, 0x05, 3, 0x02]
    );
    assert_eq!(
        to_station_7(&ports.client, peer, &[0x00, 0x05, 3, 0x0C], true).await,
        [0x00, 0x05, 4, 0x0C]
    );
    let first = [0x3C, 4, 0, 1, 0x0C, 0xAA];
    let last = [0x38, 4, 1, 1, 0x0C, 0xBB];
    receive(&peers[1], [&[1, 0][..], &first].concat(), 0).await;
    assert_eq!(&next(&mut client).await[6..], [0x3C, 3, 0, 1, 0x0C, 0xAA]);
    let peer = &mut peers[1];
    assert_eq!(
        to_station_7(&ports.client, peer, &[0x40, 3, 0, 1], false).await,
        [0x40, 4, 0, 1]
    );
    // The final segment closes the request; its Segment-ACK, the repeat of
    // the segment and the repeat of the ACK all keep the IDs.
    for _ in 0..2 {
        receive(&peers[1], [&[1, 0][..], &last].concat(), 0).await;
        assert_eq!(&next(&mut client).await[6..], [0x38, 3, 1, 1, 0x0C, 0xBB]);
        let peer = &mut peers[1];
        assert_eq!(
            to_station_7(&ports.client, peer, &[0x40, 3, 1, 1], false).await,
            [0x40, 4, 1, 1]
        );
    }
    // The server's Simple-ACK, and a repeat of it after it closed.
    for _ in 0..2 {
        receive(&peers[1], vec![1, 0, 0x20, 3, 0x02], 0).await;
        assert_eq!(&next(&mut server).await[6..], [0x20, 3, 0x02]);
    }
    assert!(client.try_recv().is_err());
    router.stop().await.unwrap();
}

/// A ConfirmedEventNotification goes to both stacks: the server's answer is
/// sent through the saved plan, the client's dropped.
#[tokio::test]
async fn an_event_notification_is_answered_by_the_server_alone() {
    let (mut router, ports, [mut client, mut server], mut peers) = split().await;
    let notification = [0x00, 0x05, 9, 0x02, 0x09, 0x01];
    receive(&peers[1], [&[1, 0x04][..], &notification].concat(), 0).await;
    assert_eq!(&next(&mut server).await[6..], notification);
    assert_eq!(&next(&mut client).await[6..], notification);
    assert_eq!(ports.server.saved_replies(), 1);
    assert_eq!(ports.client.saved_replies(), 1);

    let ack = [1, 0x20, 0x07, 0xD0, 1, 7, 255, 0x20, 9, 0x02];
    // The client's: swallowed, nothing sent.
    ports.client.send_unicast(&ack, &[0xEE]).await.unwrap();
    assert_eq!(ports.client.saved_replies(), 0);
    // The server's: sent to station 7 through the plan.
    let peer = &mut peers[1];
    let (sent, result) = tokio::join!(
        async {
            let sent = sending(peer).await;
            sent.complete.send(Ok(())).unwrap();
            (sent.to, sent.npdu)
        },
        ports.server.send_unicast(&ack, &[0xEE])
    );
    result.unwrap();
    assert_eq!(sent.0, Some(vec![7]));
    assert_eq!(&sent.1[2..], [0x20, 9, 0x02]);
    assert!(peers[1].sent.try_recv().is_err(), "one answer only");
    assert_eq!(router.counters().originated, 0);
    router.stop().await.unwrap();
}

/// The client's answer to a ConfirmedEventNotification, sent through the
/// plan the client's port kept because the server's couldn't take it: one
/// Simple-ACK to station 7, and nothing more.
async fn the_client_answers(
    router: &super::super::BACnetRouter,
    ports: &RouterApplicationPorts,
    peer: &mut Peer,
) {
    assert_eq!(ports.client.saved_replies(), 1);
    let ack = [1, 0x20, 0x07, 0xD0, 1, 7, 255, 0x20, 9, 0x02];
    let (sent, result) = tokio::join!(
        async {
            let sent = sending(peer).await;
            sent.complete.send(Ok(())).unwrap();
            (sent.to, sent.npdu)
        },
        ports.client.send_unicast(&ack, &[0xEE])
    );
    result.unwrap();
    assert_eq!(sent.0, Some(vec![7]));
    assert_eq!(&sent.1[2..], [0x20, 9, 0x02]);
    assert!(peer.sent.try_recv().is_err(), "one answer only");
    assert_eq!(router.counters().originated, 0, "through the plan");
}

const EVENT_NOTIFICATION: [u8; 6] = [0x00, 0x05, 9, 0x02, 0x09, 0x01];

/// With the server's port not started, the client answers an event
/// notification.
#[tokio::test]
async fn without_a_server_the_client_answers_an_event_notification() {
    let (mut router, apdus, mut peers) = start(2).await;
    let mut ports = router.application_ports(apdus);
    let mut client = ports.client.start().await.unwrap();
    receive(&peers[1], [&[1, 0x04][..], &EVENT_NOTIFICATION].concat(), 0).await;
    assert_eq!(&next(&mut client).await[6..], EVENT_NOTIFICATION);
    assert_eq!(ports.server.saved_replies(), 0);
    the_client_answers(&router, &ports, &mut peers[1]).await;
    router.stop().await.unwrap();
}

/// With the server's queue full, the client answers an event notification.
#[tokio::test(start_paused = true)]
async fn with_the_server_full_the_client_answers_an_event_notification() {
    let (mut router, ports, [mut client, mut server], mut peers) = split().await;
    for _ in 0..QUEUE {
        receive(&peers[0], vec![1, 0, 0x10, 0x08], 0).await;
        next(&mut client).await;
    }
    receive(&peers[1], [&[1, 0x04][..], &EVENT_NOTIFICATION].concat(), 0).await;
    assert_eq!(&next(&mut client).await[6..], EVENT_NOTIFICATION);
    assert_eq!(ports.server.saved_replies(), 0);
    the_client_answers(&router, &ports, &mut peers[1]).await;
    // The server got only the Who-Is it had room for.
    for _ in 0..QUEUE {
        assert_eq!(&server.recv().await.unwrap().npdu[6..], [0x10, 0x08]);
    }
    assert!(server.try_recv().is_err());
    router.stop().await.unwrap();
}

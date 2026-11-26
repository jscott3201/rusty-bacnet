//! A client seat on endpoint ingress (#2617): each rule of who receives
//! what, that a client which stops reading holds nothing else up, and how
//! the link stops around a seat.
use super::*;
use crate::coordinator::{
    AdmissionOutcome, CanonicalPeer, LeaseHolder, LeaseMetadata, OutboundTransactionCoordinator,
    TerminalPolicy,
};
use crate::seat::{ClientSeat, SeatRoles};
use bacnet_types::enums::ConfirmedServiceChoice;

/// The link source every injected APDU comes from.
const PEER: [u8; 1] = [0x11];

const WHO_IS: &[u8] = &[0x10, 0x08];
const READ_PROPERTY: &[u8] = &[0x00, 0x05, 0x21, 0x0c];
const COV_NOTIFICATION: &[u8] = &[0x00, 0x05, 0x22, 0x01];
const EVENT_NOTIFICATION: &[u8] = &[0x00, 0x05, 0x23, 0x02];
/// A first segment of a ConfirmedCOVNotification: the service choice sits
/// after the sequence number and window size.
const SEGMENTED_COV_NOTIFICATION: &[u8] = &[0x0c, 0x05, 0x24, 0x00, 0x01, 0x01, 0xaa];
/// A first segment of a ConfirmedEventNotification.
const SEGMENTED_EVENT_NOTIFICATION: &[u8] = &[0x0c, 0x05, 0x25, 0x00, 0x01, 0x02, 0xaa];

struct Seated {
    endpoint: EndpointIngress<TestTransport>,
    ingress: IngressReceivers,
    link: Link,
    coordinator: Arc<OutboundTransactionCoordinator>,
    seat: ClientSeat<TestTransport>,
}

async fn seated(server: bool, capacity: usize) -> Seated {
    let (transport, link) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 8);
    let ingress = endpoint.start().await.unwrap();
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let seat = endpoint
        .client_seat(Arc::clone(&coordinator), SeatRoles::new(server), capacity)
        .unwrap();
    Seated {
        endpoint,
        ingress,
        link,
        coordinator,
        seat,
    }
}

/// `apdu` from [`PEER`] with a reply sender, as MS/TP hands up a frame whose sender
/// waits for a reply.
async fn inject_expecting_reply(link: &Link, apdu: &[u8]) -> oneshot::Receiver<Bytes> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut received = received_npdu(apdu);
    received.reply_tx = Some(reply_tx);
    link.sender.send(received).await.unwrap();
    reply_rx
}

fn requester(peer: &[u8]) -> LeaseMetadata {
    LeaseMetadata::requester(
        CanonicalPeer::direct(peer),
        ConfirmedServiceChoice::READ_PROPERTY,
        TerminalPolicy::EitherAck,
    )
}

/// Nothing is queued for the seat or for either of the endpoint's queues.
fn assert_quiet(seated: &mut Seated) {
    assert!(seated.seat.apdus.try_recv().is_err());
    assert!(seated.ingress.inbound_requests.try_recv().is_err());
    assert!(seated.ingress.terminal_or_segment.try_recv().is_err());
    assert!(seated.ingress.policy_outcomes.try_recv().is_err());
}

#[tokio::test]
async fn an_unconfirmed_request_reaches_the_server_and_a_copy_reaches_the_seat() {
    let mut seated = seated(true, 8).await;
    let _reply = inject_expecting_reply(&seated.link, WHO_IS).await;

    let served = receive(&mut seated.ingress.inbound_requests).await;
    let copy = receive(&mut seated.seat.apdus).await;
    assert_eq!(served.apdu.as_ref(), WHO_IS);
    assert_eq!(copy.apdu.as_ref(), WHO_IS);
    assert_eq!(copy.source_mac.as_slice(), PEER);
    // The server's is the APDU that came, with its reply sender; the seat's
    // is a copy that can answer nothing.
    assert!(served.reply_tx.is_some());
    assert!(copy.reply_tx.is_none() && copy.direct_response.is_none());
    assert_quiet(&mut seated);
    assert_eq!(seated.endpoint.client_seat_counters().apdus, 1);
}

#[tokio::test]
async fn beside_a_server_only_its_own_notifications_are_the_seats() {
    let mut seated = seated(true, 8).await;

    // Any other confirmed service is the server's alone.
    inject(&seated.link, READ_PROPERTY).await;
    assert_eq!(
        receive(&mut seated.ingress.inbound_requests)
            .await
            .apdu
            .as_ref(),
        READ_PROPERTY
    );
    assert_quiet(&mut seated);

    // A ConfirmedCOVNotification is the client's, reply sender and all,
    // segmented or not.
    for apdu in [COV_NOTIFICATION, SEGMENTED_COV_NOTIFICATION] {
        let _reply = inject_expecting_reply(&seated.link, apdu).await;
        let received = receive(&mut seated.seat.apdus).await;
        assert_eq!(received.apdu.as_ref(), apdu);
        assert!(received.reply_tx.is_some());
        assert_quiet(&mut seated);
    }

    // A ConfirmedEventNotification is the server's to answer; the client
    // hears a copy.
    let _reply = inject_expecting_reply(&seated.link, EVENT_NOTIFICATION).await;
    let served = receive(&mut seated.ingress.inbound_requests).await;
    let copy = receive(&mut seated.seat.apdus).await;
    assert_eq!(served.apdu.as_ref(), EVENT_NOTIFICATION);
    assert!(served.reply_tx.is_some());
    assert_eq!(copy.apdu.as_ref(), EVENT_NOTIFICATION);
    assert!(copy.reply_tx.is_none());
    assert_quiet(&mut seated);

    // Of one in segments the client hears nothing segment by segment: the
    // server hands it the whole request once reassembled (#2743).
    let _reply = inject_expecting_reply(&seated.link, SEGMENTED_EVENT_NOTIFICATION).await;
    let served = receive(&mut seated.ingress.inbound_requests).await;
    assert_eq!(served.apdu.as_ref(), SEGMENTED_EVENT_NOTIFICATION);
    assert!(served.reply_tx.is_some());
    assert_quiet(&mut seated);
}

#[tokio::test]
async fn with_no_server_every_confirmed_request_is_the_seats() {
    let mut seated = seated(false, 8).await;
    for apdu in [READ_PROPERTY, COV_NOTIFICATION, EVENT_NOTIFICATION] {
        let _reply = inject_expecting_reply(&seated.link, apdu).await;
        let received = receive(&mut seated.seat.apdus).await;
        assert_eq!(received.apdu.as_ref(), apdu);
        assert!(received.reply_tx.is_some());
        assert_quiet(&mut seated);
    }
}

#[tokio::test]
async fn an_answer_goes_to_the_role_that_holds_its_lease_and_nothing_is_claimed() {
    let mut seated = seated(true, 8).await;
    let coordinator = Arc::clone(&seated.coordinator);
    let client = coordinator.reserve(requester(&PEER)).unwrap();
    let server = coordinator
        .reserve(LeaseMetadata::notification(
            CanonicalPeer::direct(&PEER),
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        ))
        .unwrap();
    let elsewhere = coordinator.reserve(requester(&[0x22])).unwrap();
    let peer = CanonicalPeer::direct(&PEER);
    assert_eq!(
        coordinator.holder_of(&peer, client.invoke_id()),
        Some(LeaseHolder::Client)
    );
    assert_eq!(
        coordinator.holder_of(&peer, server.invoke_id()),
        Some(LeaseHolder::Server)
    );
    assert_eq!(coordinator.holder_of(&peer, elsewhere.invoke_id()), None);
    assert_eq!(coordinator.holder_of(&peer, 200), None);

    // The full client's answers, in every form an answer takes: Simple-ACK,
    // Complex-ACK, a Complex-ACK segment, Error, Reject, and an Abort and a
    // Segment-ACK from the answering side.
    let id = client.invoke_id();
    let answers: [&[u8]; 7] = [
        &[0x20, id, 0x0c],
        &[0x30, id, 0x0c],
        &[0x3c, id, 0x00, 0x01, 0x0c],
        &[0x50, id, 0x0c, 0x91, 0x00, 0x91, 0x00],
        &[0x60, id, 0x00],
        &[0x71, id, 0x00],
        &[0x41, id, 0x00, 0x01],
    ];
    for apdu in answers {
        inject(&seated.link, apdu).await;
        assert_eq!(receive(&mut seated.seat.apdus).await.apdu.as_ref(), apdu);
        assert_quiet(&mut seated);
    }
    // The lookup claimed nothing: the client's own admission still does.
    let simple_ack = bacnet_encoding::apdu::decode_apdu(Bytes::from_static(&[0x20, 0, 0x0c]));
    let Ok(Apdu::SimpleAck(mut ack)) = simple_ack else {
        panic!("a Simple-ACK decodes");
    };
    ack.invoke_id = id;
    assert!(matches!(
        coordinator.admit(&peer, &Apdu::SimpleAck(ack)).unwrap(),
        AdmissionOutcome::Admitted(_)
    ));

    // An Abort or Segment-ACK from a requester is about a request this node
    // serves, whatever lease its invoke ID matches. The client sees a copy
    // of the Abort, which may end a request it reassembles (#2646).
    for apdu in [&[0x70, id, 0x00][..], &[0x40, id, 0x00, 0x01]] {
        inject(&seated.link, apdu).await;
        assert_eq!(
            receive(&mut seated.ingress.terminal_or_segment)
                .await
                .apdu
                .as_ref(),
            apdu
        );
        if apdu[0] == 0x70 {
            let copy = receive(&mut seated.seat.apdus).await;
            assert_eq!(copy.apdu.as_ref(), apdu);
            assert!(copy.direct_response.is_none() && copy.router_response.is_none());
        }
        assert_quiet(&mut seated);
    }

    // The server role's answers, an answer for a lease another peer holds,
    // and one no lease holds, all stay with the endpoint, which counts what
    // nothing claims.
    for id in [server.invoke_id(), elsewhere.invoke_id(), 200] {
        let apdu = [0x20, id, 0x0c];
        inject(&seated.link, &apdu).await;
        assert_eq!(
            receive(&mut seated.ingress.terminal_or_segment)
                .await
                .apdu
                .as_ref(),
            apdu
        );
        assert_quiet(&mut seated);
    }
}

#[tokio::test]
async fn a_routed_answer_finds_its_lease_by_the_source_network() {
    let mut seated = seated(true, 8).await;
    let station = [0x05];
    let lease = seated
        .coordinator
        .reserve(LeaseMetadata::requester(
            CanonicalPeer::routed(7, &station),
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::EitherAck,
        ))
        .unwrap();
    let apdu = [0x20, lease.invoke_id(), 0x0c];
    let npdu = Npdu {
        source: Some(NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&station),
        }),
        payload: Bytes::copy_from_slice(&apdu),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    let mut received = received_npdu(&apdu);
    received.npdu = buffer.freeze();
    seated.link.sender.send(received).await.unwrap();

    assert_eq!(receive(&mut seated.seat.apdus).await.apdu.as_ref(), apdu);
    assert_quiet(&mut seated);
}

#[tokio::test]
async fn a_malformed_apdu_never_reaches_the_seat() {
    let mut seated = seated(false, 8).await;
    // A truncated Confirmed-Request, and a PDU type past the defined ones.
    for apdu in [&[0x00, 0x05][..], &[0x80, 0x01]] {
        inject(&seated.link, apdu).await;
        let outcome = receive(&mut seated.ingress.policy_outcomes).await;
        assert_eq!(outcome.received.apdu.as_ref(), apdu);
        assert_quiet(&mut seated);
    }
    assert_eq!(seated.endpoint.client_seat_counters().apdus, 0);
}

/// The seat's three tiers (#2635): copies of Unconfirmed-Requests fill it
/// only to half, the client's COV notifications down to its last quarter,
/// and answers to its requests all of it. A burst of I-Ams shuts out
/// neither the client's notifications nor its answers.
#[tokio::test]
async fn the_seat_takes_answers_before_notifications_before_copies() {
    let mut seated = seated(true, 8).await;
    for _ in 0..10 {
        inject(&seated.link, WHO_IS).await;
        receive(&mut seated.ingress.inbound_requests).await;
    }
    let counters = seated.endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (4, 6));

    // Two COV notifications get in behind a full share of copies; a third
    // finds only the answers' quarter left.
    let mut replies = Vec::new();
    for _ in 0..3 {
        replies.push(inject_expecting_reply(&seated.link, COV_NOTIFICATION).await);
    }
    // Two answers get in behind both; a third finds the queue full.
    let lease = seated.coordinator.reserve(requester(&PEER)).unwrap();
    let answer = [0x20, lease.invoke_id(), 0x0c];
    for _ in 0..3 {
        inject(&seated.link, &answer).await;
    }
    inject(&seated.link, READ_PROPERTY).await;
    receive(&mut seated.ingress.inbound_requests).await;
    let counters = seated.endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (8, 8));

    let queued: Vec<Bytes> = std::iter::from_fn(|| seated.seat.apdus.try_recv().ok())
        .map(|received| received.apdu)
        .collect();
    assert!(queued[..4].iter().all(|apdu| apdu.as_ref() == WHO_IS));
    assert!(
        queued[4..6]
            .iter()
            .all(|apdu| apdu.as_ref() == COV_NOTIFICATION)
    );
    assert!(queued[6..].iter().all(|apdu| apdu.as_ref() == answer));
    assert_eq!(queued.len(), 8);
    // The refused notification's reply sender was released.
    assert_eq!(
        replies[2].try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    // The endpoint's own counts are untouched: none of its queues was full.
    assert_eq!(
        seated.endpoint.full_drop_counters(),
        IngressDropCounters::default()
    );
}

/// Each tier keeps a slot of its own where the capacity allows: one slot
/// shared by all at 1; copies one and answers one at 2; one each at 3; two,
/// one and one at 4.
#[tokio::test]
async fn the_seats_tiers_at_small_capacities() {
    for (capacity, tiers) in [
        (1, [1, 0, 0]),
        (2, [1, 0, 1]),
        (3, [1, 1, 1]),
        (4, [2, 1, 1]),
        (8, [4, 2, 2]),
    ] {
        let mut seated = seated(true, capacity).await;
        let lease = seated.coordinator.reserve(requester(&PEER)).unwrap();
        let answer = [0x20, lease.invoke_id(), 0x0c];
        for _ in 0..capacity + 1 {
            inject(&seated.link, WHO_IS).await;
            receive(&mut seated.ingress.inbound_requests).await;
        }
        let mut replies = Vec::new();
        for _ in 0..capacity + 1 {
            replies.push(inject_expecting_reply(&seated.link, COV_NOTIFICATION).await);
        }
        for _ in 0..capacity + 1 {
            inject(&seated.link, &answer).await;
        }
        inject(&seated.link, &[0x80]).await;
        receive(&mut seated.ingress.policy_outcomes).await;

        let mut taken = [0; 3];
        while let Ok(received) = seated.seat.apdus.try_recv() {
            let tier = match received.apdu[0] >> 4 {
                1 => 0,
                0 => 1,
                _ => 2,
            };
            taken[tier] += 1;
        }
        assert_eq!(taken, tiers, "capacity {capacity}");
        assert_eq!(taken.iter().sum::<usize>(), capacity);
    }
}

/// With no server role every Confirmed-Request goes whole to the seat, and
/// it too fills the seat only down to the reserve: a burst of reads can't
/// crowd out the answer to the client's own request (#2635).
#[tokio::test]
async fn confirmed_requests_leave_a_serverless_seat_room_for_the_clients_answers() {
    let mut seated = seated(false, 8).await;
    let mut replies = Vec::new();
    for _ in 0..10 {
        replies.push(inject_expecting_reply(&seated.link, READ_PROPERTY).await);
    }
    let lease = seated.coordinator.reserve(requester(&PEER)).unwrap();
    let answer = [0x20, lease.invoke_id(), 0x0c];
    inject(&seated.link, &answer).await;
    // A marker the policy queue takes, classified after the rest.
    inject(&seated.link, &[0x80]).await;
    receive(&mut seated.ingress.policy_outcomes).await;

    let counters = seated.endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (7, 4));
    let queued: Vec<ReceivedApdu> =
        std::iter::from_fn(|| seated.seat.apdus.try_recv().ok()).collect();
    assert!(queued[..6].iter().all(|r| r.apdu.as_ref() == READ_PROPERTY));
    assert_eq!(queued[6].apdu.as_ref(), answer);
    // The four dropped requests' reply senders were released, never handed
    // to the endpoint in the client's place.
    let released = replies
        .iter_mut()
        .map(|reply| reply.try_recv())
        .filter(|reply| *reply == Err(oneshot::error::TryRecvError::Closed))
        .count();
    assert_eq!(released, 4);
    assert!(seated.ingress.inbound_requests.try_recv().is_err());
    // Counted once, by the seat: the endpoint's own counts never saw them.
    assert_eq!(
        seated.endpoint.full_drop_counters(),
        IngressDropCounters::default()
    );
}

#[tokio::test]
async fn a_client_that_stops_reading_is_skipped_and_counted() {
    // A queue of one keeps no reserve (#2635).
    let mut seated = seated(true, 1).await;
    // The client never reads: its queue of one fills, and the server still
    // gets every request, at once.
    for _ in 0..5 {
        inject(&seated.link, WHO_IS).await;
        assert_eq!(
            receive(&mut seated.ingress.inbound_requests)
                .await
                .apdu
                .as_ref(),
            WHO_IS
        );
    }
    inject(&seated.link, READ_PROPERTY).await;
    assert_eq!(
        receive(&mut seated.ingress.inbound_requests)
            .await
            .apdu
            .as_ref(),
        READ_PROPERTY
    );
    // An APDU that is the seat's alone is dropped too, never handed to the
    // endpoint in the client's place: not a notification, which the server
    // role would answer, and not an answer to the client's request, which
    // the endpoint's answer handling would claim for the wrong role.
    let lease = seated.coordinator.reserve(requester(&PEER)).unwrap();
    inject(&seated.link, COV_NOTIFICATION).await;
    inject(&seated.link, &[0x20, lease.invoke_id(), 0x0c]).await;
    // The next request is the first thing the endpoint sees after them.
    inject(&seated.link, READ_PROPERTY).await;
    assert_eq!(
        receive(&mut seated.ingress.inbound_requests)
            .await
            .apdu
            .as_ref(),
        READ_PROPERTY
    );
    assert!(seated.ingress.inbound_requests.try_recv().is_err());
    assert!(seated.ingress.terminal_or_segment.try_recv().is_err());
    assert!(seated.ingress.policy_outcomes.try_recv().is_err());

    let counters = seated.endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (1, 6));
    // Only the one that fit is the client's.
    assert_eq!(receive(&mut seated.seat.apdus).await.apdu.as_ref(), WHO_IS);
    assert!(seated.seat.apdus.try_recv().is_err());
}

#[tokio::test]
async fn one_client_at_a_time_and_a_seat_whose_client_is_gone_takes_nothing() {
    let Seated {
        endpoint,
        mut ingress,
        link,
        coordinator,
        seat,
    } = seated(true, 8).await;
    let second = endpoint.client_seat(Arc::clone(&coordinator), SeatRoles::new(true), 8);
    assert!(matches!(
        second.unwrap_err(),
        Error::LimitReached(text)
            if text.to_string() == "the endpoint already has a full client: drop it before making another"
    ));

    // With the client gone its notification goes where it went before any
    // seat, and the endpoint takes another client.
    drop(seat);
    inject(&link, COV_NOTIFICATION).await;
    assert_eq!(
        receive(&mut ingress.inbound_requests).await.apdu.as_ref(),
        COV_NOTIFICATION
    );
    let mut again = endpoint
        .client_seat(coordinator, SeatRoles::new(true), 8)
        .unwrap();
    inject(&link, COV_NOTIFICATION).await;
    assert_eq!(
        receive(&mut again.apdus).await.apdu.as_ref(),
        COV_NOTIFICATION
    );
    assert!(matches!(
        endpoint.client_seat(
            Arc::new(OutboundTransactionCoordinator::new()),
            SeatRoles::new(true),
            0
        ),
        Err(Error::InvalidConfig(text))
            if text.to_string() == "client seat queue capacity must be greater than zero"
    ));
}

/// An I-Am-Router-To-Network for network 7, from [`PEER`].
fn router_announcement() -> ReceivedNpdu {
    let mut received = received_npdu(WHO_IS);
    received.npdu = Bytes::from_static(&[0x01, 0x80, 0x01, 0x00, 0x07]);
    received
}

#[tokio::test]
async fn network_controls_reach_the_seat_on_a_link_with_no_number_owner() {
    let mut seated = seated(true, 8).await;
    assert!(seated.ingress.network_controls.is_none());
    seated
        .link
        .sender
        .send(router_announcement())
        .await
        .unwrap();

    let control = receive(&mut seated.seat.controls).await;
    assert_eq!(control.source_mac.as_slice(), PEER);
    assert_eq!(control.npdu.payload.as_ref(), [0x00, 0x07]);
    assert_quiet(&mut seated);
}

#[tokio::test]
async fn network_controls_reach_the_number_owner_and_the_seat() {
    let (sender, receiver) = mpsc::channel(8);
    let transport = TestTransport::builder()
        .local_mac(&[0xaa])
        .inbound(receiver)
        .number_controls()
        .build();
    let mut endpoint = EndpointIngress::new(transport, 8);
    let mut ingress = endpoint.start().await.unwrap();
    let mut owner = ingress.network_controls.take().expect("a Number owner");
    // Before any seat, and with one whose client reads nothing, the owner
    // gets every control.
    sender.send(router_announcement()).await.unwrap();
    assert_eq!(receive(&mut owner).await.ingress_sequence, 1);
    let mut seat = endpoint
        .client_seat(
            Arc::new(OutboundTransactionCoordinator::new()),
            SeatRoles::new(true),
            8,
        )
        .unwrap();
    for sequence in 2..=(2 + crate::seat::CLIENT_SEAT_CONTROL_QUEUE as u64) {
        sender.send(router_announcement()).await.unwrap();
        assert_eq!(receive(&mut owner).await.ingress_sequence, sequence);
    }
    assert_eq!(receive(&mut seat.controls).await.ingress_sequence, 2);
    assert_eq!(endpoint.client_seat_counters().control_full_drops, 1);
}

#[tokio::test]
async fn stop_ends_the_seat_and_waits_for_the_tasks_that_hold_the_layer() {
    let Seated {
        mut endpoint,
        link,
        seat,
        ..
    } = seated(true, 8).await;
    let ClientSeat {
        network,
        mut apdus,
        holds,
        ..
    } = seat;
    // A client's task: it keeps the layer and a hold until its queue closes,
    // and lets go of the layer first.
    let hold = holds.hold().expect("a hold while the endpoint runs");
    let released = Arc::new(Notify::new());
    let release = Arc::clone(&released);
    let task = tokio::spawn(async move {
        let held = (network, hold);
        assert!(apdus.recv().await.is_none());
        release.notified().await;
        drop(held.0);
        drop(held.1);
    });

    let mut stop = Box::pin(endpoint.stop());
    // The queue closed, but the task still holds the layer: stop waits, and
    // the transport is not stopped under it.
    assert!(timeout(Duration::from_millis(50), &mut stop).await.is_err());
    assert_eq!(link.stops(), 0);
    released.notify_one();
    assert!(matches!(stop.await.unwrap(), ClassifierExit::Cancelled));
    task.await.unwrap();
    assert_eq!(link.stops(), 1);
    assert!(holds.hold().is_none(), "no hold once the endpoint stopped");
}

#[tokio::test]
async fn stop_aborts_a_seated_clients_task_that_would_never_end_by_itself() {
    let Seated {
        mut endpoint,
        link,
        seat,
        ..
    } = seated(true, 8).await;
    let ClientSeat { network, holds, .. } = seat;
    // A client's task stuck in the middle of a turn: it holds the layer and
    // never looks at its queue again. The client named it to the endpoint.
    let hold = holds.hold().unwrap();
    let task = tokio::spawn(async move {
        let _held = (network, hold);
        std::future::pending::<()>().await;
    });
    holds.abort_on_stop(task.abort_handle());

    let exit = timeout(WAIT, endpoint.stop())
        .await
        .expect("the stop doesn't wait on the stuck task");
    assert!(matches!(exit.unwrap(), ClassifierExit::Cancelled));
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(link.stops(), 1);
}

#[tokio::test]
async fn a_dropped_client_frees_the_seat_before_its_tasks_have_ended() {
    let Seated {
        endpoint,
        coordinator,
        seat,
        ..
    } = seated(true, 8).await;
    // The client's task keeps the seat's queue, as an aborted task does
    // until the runtime next runs it; the client itself is what `alive`
    // stands for.
    let alive: Arc<dyn std::any::Any + Send + Sync> = Arc::new(());
    seat.holds.alive_while(Arc::downgrade(&alive));
    let again = |endpoint: &EndpointIngress<TestTransport>| {
        endpoint.client_seat(Arc::clone(&coordinator), SeatRoles::new(true), 8)
    };
    assert!(again(&endpoint).is_err(), "the client lives");
    drop(alive);
    // No yield between the drop and the next seat.
    let next = again(&endpoint).expect("the dropped client's seat is free");
    assert!(again(&endpoint).is_err(), "and taken again");
    drop((seat, next));
}

/// The error a send through a stopped layer gets (#2617): its transport's
/// stopped state (#1833), not running, since the test transport starts
/// again.
fn is_stopped(error: &Error) -> bool {
    matches!(
        error,
        Error::State(bacnet_types::error::Lifecycle::NotRunning, _)
    )
}

#[tokio::test]
async fn one_stop_stops_the_link_while_a_seat_still_keeps_the_layer() {
    let Seated {
        mut endpoint,
        link,
        seat,
        coordinator,
        ..
    } = seated(true, 8).await;
    // A seat nobody started a client on keeps the layer too, but no hold:
    // the stop neither waits for it nor needs it dropped (#2617).
    assert!(matches!(
        timeout(WAIT, endpoint.stop()).await.unwrap().unwrap(),
        ClassifierExit::Cancelled
    ));
    assert_eq!(link.stops(), 1);
    assert!(seat.holds.hold().is_none());
    assert!(matches!(
        endpoint.client_seat(coordinator, SeatRoles::new(true), 8),
        Err(Error::State(error::Lifecycle::Closed, _))
    ));
    // The layer the seat still keeps has stopped: what goes through it,
    // or straight to its transport, fails as a stopped link's send does.
    let sent = link.counters.sent().len();
    let through_layer = seat
        .network
        .broadcast_apdu(WHO_IS, false, NetworkPriority::Normal)
        .await
        .unwrap_err();
    assert!(is_stopped(&through_layer), "{through_layer}");
    let direct = seat.network.transport().send_broadcast(WHO_IS).await;
    assert!(is_stopped(&direct.unwrap_err()));
    assert_eq!(link.counters.sent().len(), sent, "nothing reached the link");
    // Dropping the seat afterwards changes nothing.
    drop(seat);
    assert_eq!(link.stops(), 1);
    assert!(endpoint.stop().await.is_err(), "stopped once");
}

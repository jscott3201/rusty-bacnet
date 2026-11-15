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
    let narrow = coordinator.reserve(requester(&PEER)).unwrap();
    coordinator
        .mark_endpoint_requester(narrow.invoke_id())
        .unwrap();
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
        coordinator.holder_of(&peer, narrow.invoke_id()),
        Some(LeaseHolder::EndpointRequester)
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
    // serves, whatever lease its invoke ID matches.
    for apdu in [&[0x70, id, 0x00][..], &[0x40, id, 0x00, 0x01]] {
        inject(&seated.link, apdu).await;
        assert_eq!(
            receive(&mut seated.ingress.terminal_or_segment)
                .await
                .apdu
                .as_ref(),
            apdu
        );
        assert_quiet(&mut seated);
    }

    // The endpoint requester's and the server role's answers, an answer for
    // a lease another peer holds, and one no lease holds, all stay with the
    // endpoint, which counts what nothing claims.
    for id in [
        narrow.invoke_id(),
        server.invoke_id(),
        elsewhere.invoke_id(),
        200,
    ] {
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

#[tokio::test]
async fn a_client_that_stops_reading_is_skipped_and_counted() {
    let mut seated = seated(true, 2).await;
    // The client never reads: its queue of two fills, and the server still
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
    assert_eq!((counters.apdus, counters.full_drops), (2, 5));
    // Only the two that fit are the client's.
    for _ in 0..2 {
        assert_eq!(receive(&mut seated.seat.apdus).await.apdu.as_ref(), WHO_IS);
    }
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
    assert!(second.unwrap_err().to_string().contains("already has"));

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
    assert!(
        endpoint
            .client_seat(
                Arc::new(OutboundTransactionCoordinator::new()),
                SeatRoles::new(true),
                0
            )
            .is_err()
    );
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

#[tokio::test]
async fn stop_fails_while_a_client_keeps_the_layer_and_finishes_once_it_is_dropped() {
    let Seated {
        mut endpoint,
        link,
        seat,
        coordinator,
        ..
    } = seated(true, 8).await;
    // A seat nobody started a client on holds the layer too, but no hold:
    // the stop doesn't wait for it.
    let error = timeout(WAIT, endpoint.stop()).await.unwrap().unwrap_err();
    assert!(error.to_string().contains("full client still holds"));
    assert!(endpoint.link_held());
    assert_eq!(link.stops(), 0);
    assert!(seat.holds.hold().is_none());
    assert!(
        endpoint
            .client_seat(coordinator, SeatRoles::new(true), 8)
            .is_err()
    );
    // Still held: the same answer, and still no transport stop.
    assert!(endpoint.stop().await.is_err());
    assert_eq!(link.stops(), 0);

    drop(seat);
    assert!(matches!(
        endpoint.stop().await.unwrap(),
        ClassifierExit::Cancelled
    ));
    assert!(!endpoint.link_held());
    assert_eq!(link.stops(), 1);
    assert!(endpoint.stop().await.is_err(), "stopped once");
}

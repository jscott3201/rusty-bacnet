//! Stopping the full client and the session in either order, with a request
//! open, and the reply paths a frame keeps when it expects a reply.
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::time::Instant;

/// A read of the peer's AI-1 that gets no answer: the node hears nothing
/// while the read is open. Returns the task once the request is on the link.
async fn open_read(
    lab: &Lab,
    client: &Arc<BACnetClient<TestTransport>>,
) -> tokio::task::JoinHandle<Result<(), bacnet_types::error::Error>> {
    lab.wire.hold(&Wire::mac(NODE));
    let sent = apdus(&lab.node_sent).len();
    let reader = Arc::clone(client);
    let read = tokio::spawn(async move {
        reader
            .read_property(
                &Wire::mac(PEER),
                ai1(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
            .map(|_| ())
    });
    eventually(WAIT, "the read on the link", async || {
        apdus(&lab.node_sent).len() > sent
    })
    .await;
    read
}

/// No task of the test's runtime is left but the test's own.
async fn assert_no_task_left() {
    eventually(WAIT, "every spawned task ended", async || {
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            == 0
    })
    .await;
}

/// Whether `result` failed as a send on a stopped transport does (#2617):
/// the transport's state, not running, since the test transport starts
/// again (#1833).
fn stopped<T: std::fmt::Debug>(result: Result<T, bacnet_types::error::Error>) -> bool {
    matches!(
        result,
        Err(bacnet_types::error::Error::State(
            bacnet_types::error::Lifecycle::NotRunning,
            _
        ))
    )
}

#[tokio::test(start_paused = true)]
async fn the_session_stopping_first_fails_open_requests_and_stops_the_link_under_a_live_client() {
    for _ in 0..20 {
        let mut lab = lab(SessionRole::Both).await;
        let client = Arc::new(lab.session.client().unwrap());
        let read = open_read(&lab, &client).await;

        // One stop, with the client still alive (#2617): it ends the
        // client's tasks, so the open read fails now, not at its timeout,
        // and it stops the link the client still holds.
        let asked = Instant::now();
        lab.session.stop().await.unwrap();
        assert!(read.await.unwrap().is_err());
        assert_eq!(asked.elapsed(), Duration::ZERO);
        assert_eq!(lab.node_link.stops(), 1);
        assert!(!lab.session.is_running());
        assert!(lab.session.client().is_err());
        // From here the client sends nothing: a request, a Who-Is in any
        // scope and a network message each fail at once, and no frame of
        // theirs reaches the link.
        let (sent, asked) = (lab.node_sent.len(), Instant::now());
        let late = client
            .read_property(
                &Wire::mac(PEER),
                ai1(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await;
        // A confirmed request says what an unconfirmed send says.
        let late = late.unwrap_err().to_string();
        assert!(late.contains("seat has closed"), "{late}");
        assert!(client.who_is(Broadcast::Global, None).await.is_err());
        assert!(
            client
                .who_is_directed(&Wire::mac(PEER), None)
                .await
                .is_err()
        );
        assert!(client.who_is(Broadcast::Network(7), None).await.is_err());
        assert!(client.who_is_router_to_network(None).await.is_err());
        assert!(client.what_is_network_number(None).await.is_err());
        // Nor does the link itself, reached through the client.
        let transport = client.transport();
        assert!(stopped(transport.send_broadcast(&[0x01, 0x00]).await));
        assert!(stopped(
            transport
                .send_unicast(&[0x01, 0x00], &Wire::mac(PEER))
                .await
        ));
        assert_eq!(asked.elapsed(), Duration::ZERO);
        assert_eq!(lab.node_sent.len(), sent, "no frame went out");

        // The client's own stop succeeds and stops nothing more, and its
        // drop is harmless.
        let mut client = Arc::try_unwrap(client).ok().expect("the only handle");
        client.stop().await.unwrap();
        drop(client);
        assert_eq!(lab.node_link.stops(), 1);
        assert!(lab.session.stop().await.is_err(), "stopped once");
        assert_eq!(lab.session.active_leases(), 0);
        lab.peer.stop().await.unwrap();
        drop(lab);
        assert_no_task_left().await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_client_stopping_first_leaves_the_session_and_its_link_running() {
    for _ in 0..20 {
        let lab = lab(SessionRole::Both).await;
        let client = Arc::new(lab.session.client().unwrap());
        let read = open_read(&lab, &client).await;
        // While one full client lives, the session makes no second.
        assert!(lab.session.client().is_err());

        // A caller's request borrows the client, so it is given up before
        // the client can be stopped: its lease goes back to the node's pool.
        read.abort();
        assert!(read.await.unwrap_err().is_cancelled());
        let mut client = Arc::try_unwrap(client).ok().expect("the only handle");
        client.stop().await.unwrap();
        assert_eq!(lab.node_link.stops(), 0);
        drop(client);
        // The stopped client's tasks were joined by its stop, so the seat
        // is free as soon as the client is dropped. (A client dropped
        // without a stop is the next test's.)
        let at_once = lab.session.client().expect("the seat is free");
        drop(at_once);
        lab.wire.release(&Wire::mac(NODE));
        eventually(WAIT, "the stopped client's lease released", async || {
            lab.session.active_leases() == 0
        })
        .await;

        // The session still runs: its server role answers, and it takes a
        // new client.
        assert!(lab.session.is_running());
        let other = other_client(&lab.wire).await;
        other
            .read_property(
                &Wire::mac(NODE),
                oid(ObjectType::DEVICE, NODE_DEVICE),
                PropertyIdentifier::OBJECT_NAME,
                None,
            )
            .await
            .unwrap();
        let again = lab.session.client().unwrap();
        again
            .read_property(
                &Wire::mac(PEER),
                ai1(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
            .unwrap();

        drop((again, other));
        stop(lab).await;
        assert_no_task_left().await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_client_dropped_without_a_stop_frees_the_seat_at_once() {
    let lab = lab(SessionRole::Both).await;
    let peer = Wire::mac(PEER);
    let mut client = lab.session.client().unwrap();
    for _ in 0..20 {
        // The drop aborts the client's tasks, which still hold the seat's
        // queue until the runtime next runs them. The session takes the
        // next client anyway, with no yield in between, and it works.
        drop(client);
        client = lab
            .session
            .client()
            .expect("the dropped client's seat is free");
        client
            .read_property(&peer, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap();
    }
    assert_eq!(lab.session.active_leases(), 0);
    assert_eq!(lab.session.policy_counters().unclaimed_terminal, 0);

    drop(client);
    stop(lab).await;
    assert_no_task_left().await;
}

/// On a link that takes the Number controls, a client would ask for the
/// network's number when it stores a routed device while the number is
/// unknown. Once its seat has closed it asks nothing.
#[tokio::test(start_paused = true)]
async fn a_client_whose_seat_closed_does_not_ask_for_the_network_number() {
    let mut lab = lab_with(SessionRole::Both, true).await;
    let client = lab.session.client().unwrap();
    assert_eq!(client.network_number(), None);
    let routed = || bacnet_client::discovery::RoutedDeviceConfig {
        instance: 777,
        router_mac: Wire::mac(9).to_vec(),
        remote_network: 5,
        remote_mac: vec![0x09],
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
    };

    lab.session.stop().await.unwrap();
    let sent = lab.node_sent.len();
    client.add_routed_device(routed()).await.unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lab.node_sent.len(), sent, "no frame went out");

    drop(client);
    lab.peer.stop().await.unwrap();

    // The same call on a running session's client does ask, once: the
    // check above is not passing for want of a question.
    let mut lab = lab_with(SessionRole::Both, true).await;
    let client = lab.session.client().unwrap();
    client.add_routed_device(routed()).await.unwrap();
    assert_eq!(
        network_messages(&lab.node_sent),
        [bacnet_npdu::NetworkMessageType::WHAT_IS_NETWORK_NUMBER]
    );
    drop(client);
    lab.session.stop().await.unwrap();
    lab.peer.stop().await.unwrap();
}

/// Dropping the session without a stop ends the client and aborts the
/// link, though the client still holds it (#2617): no live client keeps a
/// dropped session's link up.
#[tokio::test(start_paused = true)]
async fn dropping_the_session_ends_the_client_and_aborts_the_link_it_still_holds() {
    let lab = lab(SessionRole::Both).await;
    let client = Arc::new(lab.session.client().unwrap());
    let read = open_read(&lab, &client).await;
    let Lab {
        session,
        node_link,
        node_sent,
        mut peer,
        ..
    } = lab;

    let asked = Instant::now();
    drop(session);
    // At once, with no turn of the runtime: an abort, not a stop.
    assert_eq!((node_link.stops(), node_link.aborts()), (0, 1));
    assert!(read.await.unwrap().is_err());
    assert_eq!(asked.elapsed(), Duration::ZERO);
    // The client sends nothing more, through its seat or its link.
    let sent = node_sent.len();
    assert!(client.who_is(Broadcast::Global, None).await.is_err());
    let transport = client.transport();
    assert!(stopped(transport.send_broadcast(&[0x01, 0x00]).await));
    assert!(stopped(
        transport
            .send_unicast(&[0x01, 0x00], &Wire::mac(PEER))
            .await
    ));
    assert_eq!(node_sent.len(), sent, "no frame went out");
    assert_eq!(node_link.stops(), 0);
    drop(client);
    peer.stop().await.unwrap();
    drop(peer);
    assert_no_task_left().await;
}

#[tokio::test(start_paused = true)]
async fn a_full_client_needs_a_running_session() {
    let wire = Wire::new();
    let config = SessionConfig::default();
    let mut session =
        EndpointSession::new(wire.port(NODE).build(), SessionRole::Both, config.clone())
            .unwrap()
            .device(bacnet_endpoint::DeviceIdentity::new(901, 42).unwrap());
    assert!(session.client().is_err(), "before start");
    session.start().await.unwrap();
    let client = session.client().unwrap();
    drop(client);
    session.stop().await.unwrap();
    assert!(session.client().is_err(), "after stop");

    // A session with a source Audit Reporter has one too once it runs; its
    // reads and writes are audited (#2617).
    let audited = EndpointSession::new(wire.port(OTHER).build(), SessionRole::ClientOnly, config)
        .unwrap()
        .with_source_audit_reporter(oid(ObjectType::AUDIT_REPORTER, 1));
    let error = audited.client().err().expect("not running");
    assert!(error.to_string().contains("not running"));
}

/// The session's stop must not wait on the full client's dispatch task: one
/// stuck in the middle of a turn, here in a send the link never finishes,
/// is aborted.
#[tokio::test(start_paused = true)]
async fn the_sessions_stop_does_not_wait_on_a_client_task_stuck_in_a_turn() {
    let mut lab = lab(SessionRole::Both).await;
    let client = lab.session.client().unwrap();
    let peer = Wire::mac(PEER);
    let mut notifications = client.cov_notifications();
    client
        .subscribe_cov(&peer, 9, ai1(), true, Some(600))
        .await
        .unwrap();
    notifications.recv().await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    // The client's next acknowledgment never leaves the link's send.
    lab.node_link.block_next_send();
    lab.peer
        .set_present_value_local(&ai1(), bacnet_types::primitives::PropertyValue::Real(30.0))
        .await
        .unwrap();
    lab.node_link.wait_blocked().await;

    let asked = Instant::now();
    tokio::time::timeout(WAIT, lab.session.stop())
        .await
        .expect("the stop doesn't wait on the stuck task")
        .expect("one stop, with the client still alive (#2617)");
    assert_eq!(asked.elapsed(), Duration::ZERO);
    assert_eq!(lab.node_link.stops(), 1);
    drop(client);
    lab.node_link.release_sends(1);
    lab.peer.stop().await.unwrap();
}

/// A frame from node [`OTHER`] that expects a reply, as MS/TP hands one up,
/// and where the reply would come back.
fn expecting_reply(
    apdu: &Apdu,
) -> (
    bacnet_transport::port::ReceivedNpdu,
    oneshot::Receiver<Bytes>,
) {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut received = wire::frame(&Wire::mac(OTHER), npdu_of(apdu, true), false);
    received.reply_tx = Some(reply_tx);
    (received, reply_rx)
}

#[tokio::test(start_paused = true)]
async fn a_frame_expecting_a_reply_is_answered_on_its_own_path_by_the_role_it_is_for() {
    let lab = lab(SessionRole::Both).await;
    let client = lab.session.client().unwrap();
    let mut notifications = client.cov_notifications();
    let node = Wire::mac(NODE);
    let to_other = |log: &SendLog| {
        apdus(log)
            .into_iter()
            .filter(|(to, _)| *to == Wire::mac(OTHER))
            .count()
    };

    // A confirmed COV notification is the client's: its acknowledgment goes
    // back as the frame's own reply, and nothing is sent beside it.
    let mut parameters = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: 4,
        initiating_device_identifier: oid(ObjectType::DEVICE, 3003),
        monitored_object_identifier: ai1(),
        time_remaining: 60,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x41, 0xa0, 0x00, 0x00],
            priority: None,
        }],
    }
    .encode(&mut parameters)
    .unwrap();
    let notification = confirmed(
        11,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        &parameters,
    );
    let (received, reply) = expecting_reply(&notification);
    lab.wire.deliver(&node, received);
    let reply = decode_npdu(reply.await.expect("the client's reply")).unwrap();
    assert!(matches!(
        decode_apdu(reply.payload).unwrap(),
        Apdu::SimpleAck(ack) if ack.invoke_id == 11
    ));
    assert_eq!(
        notifications
            .recv()
            .await
            .unwrap()
            .subscriber_process_identifier(),
        4
    );
    assert_eq!(to_other(&lab.node_sent), 0);

    // A read of the node's Device is the server role's: answered as the
    // frame's reply too.
    let read = confirmed(
        12,
        ConfirmedServiceChoice::READ_PROPERTY,
        &read_name(oid(ObjectType::DEVICE, NODE_DEVICE)),
    );
    let (received, reply) = expecting_reply(&read);
    lab.wire.deliver(&node, received);
    let reply = decode_npdu(reply.await.expect("the server role's reply")).unwrap();
    assert!(matches!(
        decode_apdu(reply.payload).unwrap(),
        Apdu::ComplexAck(ack) if ack.invoke_id == 12
    ));
    assert_eq!(to_other(&lab.node_sent), 0);

    drop(client);
    stop(lab).await;
}

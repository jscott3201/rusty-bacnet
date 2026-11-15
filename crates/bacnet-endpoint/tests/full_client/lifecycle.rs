//! Stopping the full client and the session in either order, with a request
//! open, and the reply paths a frame keeps when it expects a reply.
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
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

#[tokio::test(start_paused = true)]
async fn the_session_stopping_first_fails_open_requests_and_waits_for_the_client_to_go() {
    for _ in 0..20 {
        let mut lab = lab(SessionRole::Both).await;
        let client = Arc::new(lab.session.full_client().unwrap());
        let read = open_read(&lab, &client).await;

        // The session's stop ends the client's tasks, so the open read
        // fails now, not at its timeout. The client itself still holds the
        // link, so the stop says so and leaves the transport running.
        let asked = Instant::now();
        let error = lab.session.stop().await.expect_err("the link is held");
        assert!(error.to_string().contains("full client still holds"));
        assert!(read.await.unwrap().is_err());
        assert_eq!(asked.elapsed(), Duration::ZERO);
        assert_eq!(lab.node_link.stops(), 0);
        assert!(!lab.session.is_running());
        assert!(lab.session.full_client().is_err());
        // A request made now fails; it is not left waiting.
        let late = client
            .read_property(
                &Wire::mac(PEER),
                ai1(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await;
        assert!(late.is_err());

        // The client's own stop succeeds and stops nothing of the node's.
        let mut client = Arc::try_unwrap(client).ok().expect("the only handle");
        client.stop().await.unwrap();
        assert_eq!(lab.node_link.stops(), 0);
        assert!(lab.session.stop().await.is_err(), "still held");

        // With the client gone the same call finishes the stop.
        drop(client);
        lab.session.stop().await.unwrap();
        assert_eq!(lab.node_link.stops(), 1);
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
        let client = Arc::new(lab.session.full_client().unwrap());
        let read = open_read(&lab, &client).await;
        // While one full client lives, the session makes no second.
        assert!(lab.session.full_client().is_err());

        // A caller's request borrows the client, so it is given up before
        // the client can be stopped: its lease goes back to the node's pool.
        read.abort();
        assert!(read.await.unwrap_err().is_cancelled());
        let mut client = Arc::try_unwrap(client).ok().expect("the only handle");
        client.stop().await.unwrap();
        assert_eq!(lab.node_link.stops(), 0);
        drop(client);
        lab.wire.release(&Wire::mac(NODE));
        eventually(WAIT, "the stopped client's lease released", async || {
            lab.session.active_leases() == 0
        })
        .await;

        // The session still runs: its narrow client reads, its server role
        // answers, and it takes a new full client.
        assert!(lab.session.is_running());
        let narrow = lab.session.client().unwrap();
        narrow
            .read_property(
                &Wire::mac(PEER),
                ai1(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
            .unwrap();
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
        let again = lab.session.full_client().unwrap();
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
async fn dropping_the_session_ends_the_client_and_the_last_owner_releases_the_link() {
    let lab = lab(SessionRole::Both).await;
    let client = Arc::new(lab.session.full_client().unwrap());
    let read = open_read(&lab, &client).await;
    let Lab {
        session,
        node_link,
        mut peer,
        ..
    } = lab;

    let asked = Instant::now();
    drop(session);
    assert!(read.await.unwrap().is_err());
    assert_eq!(asked.elapsed(), Duration::ZERO);
    assert_eq!((node_link.stops(), node_link.aborts()), (0, 0));
    drop(client);
    eventually(WAIT, "the transport released", async || {
        node_link.aborts() == 1
    })
    .await;
    peer.stop().await.unwrap();
    drop(peer);
    assert_no_task_left().await;
}

#[tokio::test(start_paused = true)]
async fn a_full_client_needs_a_running_session_with_no_source_audit() {
    let wire = Wire::new();
    let config = SessionConfig::default();
    let mut session =
        EndpointSession::new(wire.port(NODE).build(), SessionRole::Both, config.clone()).unwrap();
    assert!(session.full_client().is_err(), "before start");
    session.start().await.unwrap();
    let client = session.full_client().unwrap();
    drop(client);
    session.stop().await.unwrap();
    assert!(session.full_client().is_err(), "after stop");

    // The full client's requests are not audited, so a session that audits
    // what it initiates has none.
    let audited = EndpointSession::new(wire.port(OTHER).build(), SessionRole::ClientOnly, config)
        .unwrap()
        .with_source_audit_reporter(oid(ObjectType::AUDIT_REPORTER, 1));
    let error = audited.full_client().err().expect("refused");
    assert!(error.to_string().contains("source Audit Reporter"));
    assert!(audited.client_seat().is_err());
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
    let client = lab.session.full_client().unwrap();
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
            .notification
            .subscriber_process_identifier,
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

    // With the reply postponed the frame gets no reply, and the server
    // role's answer goes out as a frame of its own, client or no client.
    lab.session.server().unwrap().suspend_next_reply().unwrap();
    let (received, reply) = expecting_reply(&read);
    lab.wire.deliver(&node, received);
    assert!(reply.await.is_err(), "postponed: no reply on the frame");
    eventually(WAIT, "the deferred answer", async || {
        to_other(&lab.node_sent) == 1
    })
    .await;
    let (_, deferred) = apdus(&lab.node_sent).pop().unwrap();
    assert!(matches!(deferred, Apdu::ComplexAck(ack) if ack.invoke_id == 12));

    drop(client);
    stop(lab).await;
}

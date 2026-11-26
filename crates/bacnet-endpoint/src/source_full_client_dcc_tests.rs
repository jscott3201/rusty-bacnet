//! The node's DeviceCommunicationControl and the full client's source
//! Audit (#2617). These hold the node's state through a server seat taken
//! from the session's ingress, as a seated server does, so each case sets
//! it by hand; `source_server_tests.rs` drives it through the full server
//! the session builds. Beside them, a request whose retry its stopped link
//! refused, recorded by the same rule (#1833).
use super::*;
use bacnet_client::client::ClientOptions;
use bacnet_endpoint_core::seat::ServerSeat;

/// A server seat on the session, whose hold on the node's
/// DeviceCommunicationControl state the test sets.
fn node_dcc(session: &EndpointSession<TestTransport>) -> ServerSeat<TestTransport> {
    session
        .ingress
        .as_ref()
        .unwrap()
        .server_seat(Arc::clone(&session.coordinator))
        .unwrap()
}

/// One audited read answered, and its record.
async fn read_and_record(
    client: &Client,
    peer: &NetworkLayer<TestTransport>,
    requests: &mut bacnet_network::layer::ApduReceiver,
    records: &mut bacnet_network::layer::ApduReceiver,
) -> BACnetAuditNotification {
    let read = start_read(
        client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(requests).await;
    let (invoke, rp) = read_request(&request);
    send(peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
    notification(&receive(records).await, false).0
}

/// While the node restricts initiation, an audited request is refused as
/// any request is, with nothing sent: before admission, so it takes no
/// slot and no sequence number, and nothing is recorded. A client started
/// outside the node's state keeps initiating, audited.
#[tokio::test(start_paused = true)]
async fn disable_initiation_refuses_an_audited_request_before_admission() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
    let seat = node_dcc(&session);
    let first = read_and_record(&client, &peer, &mut requests, &mut records).await;
    assert_eq!(
        first.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );

    seat.dcc.set(true);
    let refused = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    assert!(matches!(
        timeout(WAIT, refused).await.unwrap().unwrap(),
        Err(Error::InitiationDisabled { sent: false })
    ));
    assert_eq!(source.available_operations(), 64);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(requests.try_recv().is_err());
    assert!(records.try_recv().is_err());

    // A client its operator starts outside the node's state is audited
    // all the same.
    drop(client);
    let exempt = Arc::new(
        session
            .client_with(ClientOptions::default().with_node_dcc_override(true))
            .unwrap(),
    );
    let record = read_and_record(&exempt, &peer, &mut requests, &mut records).await;
    assert_eq!(
        record.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(1))
    );
    assert_eq!(record.result, None);
    drop(exempt);

    seat.dcc.set(false);
    let client = client_of(&session);
    let after = read_and_record(&client, &peer, &mut requests, &mut records).await;
    assert_eq!(
        after.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(2))
    );
    drop(client);
    drop(seat);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A request waiting for the database when the node restricts initiation
/// is refused once it has the database, before it takes a sequence number.
#[tokio::test(start_paused = true)]
async fn a_request_waiting_for_the_database_takes_no_sequence_number() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let seat = node_dcc(&session);
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
    let first = read_and_record(&client, &peer, &mut requests, &mut records).await;
    assert_eq!(
        first.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    let db = Arc::clone(session.database.as_ref().unwrap());
    let guard = db.write().await;
    let waiting = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    eventually(WAIT, "the read holds a slot", async || {
        source.available_operations() != 64
    })
    .await;
    seat.dcc.set(true);
    drop(guard);
    assert!(matches!(
        timeout(WAIT, waiting).await.unwrap().unwrap(),
        Err(Error::InitiationDisabled { sent: false })
    ));
    seat.dcc.set(false);
    let next = read_and_record(&client, &peer, &mut requests, &mut records).await;
    assert_eq!(
        next.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(1))
    );
    drop(client);
    drop(seat);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A request waiting for a slot when the node restricts initiation never
/// goes out: once a slot frees it is refused, with nothing sent and
/// nothing recorded, and takes no sequence number. The one whose answer
/// freed the slot is recorded.
#[tokio::test(start_paused = true)]
async fn a_slot_waiter_does_not_go_out_once_initiation_is_disabled() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let seat = node_dcc(&session);
    // One more than the peer's lane holds (#2652).
    let reads: Vec<_> = (0..=LANE_SHARE)
        .map(|_| {
            start_read(
                &client,
                peer.local_mac(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
        })
        .collect();
    let mut sent = Vec::new();
    for _ in 0..LANE_SHARE {
        sent.push(receive(&mut requests).await);
    }
    seat.dcc.set(true);
    // A request made now is refused at once, while its lane's slots are
    // held: at its registration, before it would wait for one.
    let late = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::OBJECT_NAME,
        None,
    );
    assert!(matches!(
        timeout(Duration::from_millis(10), late)
            .await
            .expect("refused at once")
            .unwrap(),
        Err(Error::InitiationDisabled { sent: false })
    ));
    // Held while the waiter is admitted: it is refused before it would
    // wait for the database (#2755).
    let db = Arc::clone(session.database.as_ref().unwrap());
    let guard = db.write().await;
    let request = sent.pop().unwrap();
    let (invoke, rp) = read_request(&request);
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    // The freed slot goes to the waiter, which is refused at admission.
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut waiter = None;
    for read in reads {
        if read.is_finished() {
            let result = read.await.unwrap();
            if let Err(Error::InitiationDisabled { sent }) = &result {
                waiter = Some(*sent);
            }
        } else {
            read.abort();
        }
    }
    drop(guard);
    let waiter = match waiter {
        Some(sent) => sent,
        None => {
            panic!("the waiter's read ended with the typed error")
        }
    };
    assert!(!waiter, "nothing was sent for the waiter");
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(requests.try_recv().is_err(), "the waiter never went out");
    assert!(records.try_recv().is_err());
    // The admitted took sequence numbers 0 up and the waiter none: once
    // the node enables initiation, the next record has the next number.
    seat.dcc.set(false);
    let next = read_and_record(&client, &peer, &mut requests, &mut records).await;
    assert_eq!(
        next.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(LANE_SHARE as u16))
    );
    drop(client);
    drop(seat);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// An audited request already sent when the node restricts initiation
/// gets no retry: it ends with `InitiationDisabled { sent: true }`, and,
/// since it was sent, it is recorded as failed, as a timeout is, with
/// SERVICES, COMMUNICATION_DISABLED.
#[tokio::test(start_paused = true)]
async fn an_audited_request_whose_retry_is_withheld_is_recorded_as_failed() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 1_000;
    session.client_config.apdu_retries = 2;
    session.start().await.unwrap();
    let client = client_of(&session);
    let seat = node_dcc(&session);
    let read = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, _) = read_request(&request);
    seat.dcc.set(true);
    assert!(matches!(
        timeout(WAIT, read).await.unwrap().unwrap(),
        Err(Error::InitiationDisabled { sent: true })
    ));
    assert!(requests.try_recv().is_err(), "no retry went");
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(
        record.result,
        Some((ErrorClass::SERVICES, ErrorCode::COMMUNICATION_DISABLED))
    );
    eventually(WAIT, "the invoke ID is free", async || {
        session.coordinator.active_count().unwrap() == 0
    })
    .await;
    drop(client);
    drop(seat);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A request that went out, whose link then refused its retry for its
/// state, as a link that stopped does (#1833): the request ends cut off as
/// one that went out, and the source Audit records it once, with
/// COMMUNICATION, OTHER, as it records one whose retry
/// DeviceCommunicationControl withheld. The link here refuses only frames
/// to the peer, so the record reaches its recipient. A request whose first
/// send the link refuses afterwards went nowhere, and records nothing.
#[tokio::test(start_paused = true)]
async fn a_request_its_link_stranded_is_recorded_as_cut_off() {
    use bacnet_transport::test_transport::{SendMode, SentFrame};
    use std::sync::atomic::{AtomicBool, Ordering};
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let refusing = Arc::new(AtomicBool::new(false));
    let to_peer = peer.local_mac().to_vec();
    let link = TestTransport::builder()
        .inner(crate::session::bus_port::bus_port())
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .on_send({
            let refusing = Arc::clone(&refusing);
            move |frame: SentFrame| {
                let refused =
                    refusing.load(Ordering::SeqCst) && frame.mac.as_slice() == to_peer.as_slice();
                async move {
                    match refused {
                        true => Err(bacnet_transport::port::stopped()),
                        false => Ok(()),
                    }
                }
            }
        })
        .build();
    let mut session = session_on(link, database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 1_000;
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let client = client_of(&session);
    let present = PropertyIdentifier::PRESENT_VALUE;
    let read = start_read(&client, peer.local_mac(), present, None);
    let (invoke, _) = read_request(&receive(&mut requests).await);
    refusing.store(true, Ordering::SeqCst);
    let ended = timeout(WAIT, read).await.unwrap().unwrap();
    assert!(
        matches!(ended, Err(Error::RequestCutOff { sent: true })),
        "{ended:?}"
    );
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(
        record.result,
        Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
    );
    let quiet = timeout(Duration::from_millis(100), records.recv()).await;
    assert!(quiet.is_err(), "one record");
    // The link refuses the next request's first send: nothing went out.
    let refused = start_read(&client, peer.local_mac(), present, None);
    let refused = timeout(WAIT, refused).await.unwrap().unwrap();
    assert!(matches!(refused, Err(Error::State(..))), "{refused:?}");
    let quiet = timeout(Duration::from_millis(100), records.recv()).await;
    assert!(quiet.is_err(), "no record for a request that went nowhere");
    assert!(requests.try_recv().is_err(), "no retry went");
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

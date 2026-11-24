//! The full client's audited requests past the slots, and their pacing
//! (#2617, #2652): a request past its lane's share, or past the 64, waits
//! for a slot in its caller's task, and takes its turn at the destination
//! once it holds one.
use super::*;
use tokio::time::Instant;

/// One more audited read than a lane's share, to one peer: the share goes,
/// the next waits for that peer's lane and goes once one of them ends, and
/// all are recorded. A waiter whose caller gives up sends nothing and holds
/// no slot.
#[tokio::test(start_paused = true)]
async fn a_request_past_its_lanes_share_waits_for_that_lane() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
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
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(requests.try_recv().is_err(), "the next waits for its lane");
    assert_eq!(source.available_operations(), SLOTS - LANE_SHARE);
    let quitter = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::OBJECT_NAME,
        None,
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    quitter.abort();
    let _ = quitter.await;
    // Each answer frees one of the lane's slots; the first lets the
    // waiter in.
    let mut recorded = 0;
    while let Some(request) = sent.pop() {
        let (invoke, rp) = read_request(&request);
        send(&peer, &request.source_mac, ack(invoke, &rp)).await;
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.invoke_id, Some(invoke));
        recorded += 1;
        if recorded == 1 {
            sent.push(receive(&mut requests).await);
        }
    }
    assert_eq!(recorded, LANE_SHARE + 1);
    for read in reads {
        assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        requests.try_recv().is_err(),
        "the waiter that gave up sent nothing"
    );
    assert!(records.try_recv().is_err());
    eventually(WAIT, "every slot is back", async || {
        source.available_operations() == SLOTS
    })
    .await;
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// With all 64 slots held, by 16 stations that never answer, a read of
/// another station waits for one and goes as soon as one comes back: when
/// the first of those requests times out.
#[tokio::test(start_paused = true)]
async fn past_the_64_slots_a_request_waits_for_one() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 4_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
    let silent_reads: Vec<_> = (0..SLOTS)
        .map(|n| {
            start_read(
                &client,
                &silent(n / LANE_SHARE),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
        })
        .collect();
    eventually(WAIT, "every slot is held", async || {
        source.available_operations() == 0
    })
    .await;
    let asked = Instant::now();
    let read = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    // Four seconds each, with no retries in these tests.
    let request = timeout(Duration::from_secs(10), requests.recv())
        .await
        .expect("a slot came back")
        .unwrap();
    assert!(asked.elapsed() >= Duration::from_secs(3), "it waited");
    let (invoke, rp) = read_request(&request);
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
    // The silent ones are recorded as they fail; the read too.
    let mut found = false;
    for _ in 0..=SLOTS {
        let (record, _) = notification(&receive(&mut records).await, false);
        found |= record.invoke_id == Some(invoke) && record.result.is_none();
    }
    assert!(found, "the read was recorded");
    for read in silent_reads {
        assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
    }
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A request waiting for a slot when the session stops fails, sends
/// nothing and records nothing, and neither do those it waited behind:
/// past its lane's share, where the peer is sent only the share, and past
/// the 64, where another peer is sent nothing.
#[tokio::test(start_paused = true)]
async fn a_stop_ends_the_wait_for_a_slot() {
    let (peer, mut requests) = network().await;
    let (other, mut others) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
    let mut reads: Vec<_> = (0..SLOTS)
        .map(|n| {
            start_read(
                &client,
                &filling(&peer, n),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
        })
        .collect();
    eventually(WAIT, "every slot is held", async || {
        source.available_operations() == 0
    })
    .await;
    for _ in 0..LANE_SHARE {
        receive(&mut requests).await;
    }
    // One past the peer's share, and one past the 64.
    for mac in [peer.local_mac(), other.local_mac()] {
        reads.push(start_read(
            &client,
            mac,
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ));
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    session.stop().await.unwrap();
    for read in reads {
        assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(requests.try_recv().is_err(), "past the share, nothing sent");
    assert!(others.try_recv().is_err(), "past the 64, nothing sent");
    assert!(records.try_recv().is_err());
    assert_eq!(session.coordinator.active_count().unwrap(), 0);
    drop(client);
    peer.stop().await.unwrap();
    other.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// The minimum request interval holds for an audited client, and the
/// request's turn travels with it: when the caller goes away, the next
/// request to the destination still waits for the first one's end and then
/// the interval.
#[tokio::test(start_paused = true)]
async fn the_turn_at_the_destination_travels_with_an_audited_request() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.min_request_interval_ms = 5_000;
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let first = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first_request = receive(&mut requests).await;
    let sent = tokio::time::Instant::now();
    first.abort();
    let _ = first.await;
    let second = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::OBJECT_NAME,
        None,
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        requests.try_recv().is_err(),
        "the second waits while the first runs"
    );
    let (invoke, rp) = read_request(&first_request);
    send(&peer, &first_request.source_mac, ack(invoke, &rp)).await;
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    let ended = tokio::time::Instant::now();
    let second_request = timeout(Duration::from_secs(20), requests.recv())
        .await
        .expect("the second request")
        .expect("the link is open");
    assert!(
        tokio::time::Instant::now() - ended >= Duration::from_millis(4_900),
        "the second went {:?} after the first was sent",
        tokio::time::Instant::now() - sent
    );
    let (invoke, rp) = read_request(&second_request);
    send(&peer, &second_request.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, second).await.unwrap().unwrap().is_ok());
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A request of a service the node doesn't audit still waits for its
/// turn at the destination, which the client's request path leaves to the
/// node (#2652): it runs in its caller's task once it has it.
#[tokio::test(start_paused = true)]
async fn a_request_the_node_does_not_audit_is_paced_too() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.min_request_interval_ms = 1_000;
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = client_of(&session);
    let read = start_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, rp) = read_request(&request);
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_ok());
    notification(&receive(&mut records).await, false);
    let answered = Instant::now();
    let subscriber = Arc::clone(&client);
    let mac = peer.local_mac().to_vec();
    let subscribe = tokio::spawn(async move {
        subscriber
            .subscribe_cov(&mac[..], 9, target(), false, Some(60))
            .await
    });
    let request = receive(&mut requests).await;
    assert!(
        answered.elapsed() >= Duration::from_secs(1),
        "it went {:?} after the read's answer",
        answered.elapsed()
    );
    let Apdu::ConfirmedRequest(pdu) = decode_apdu(request.apdu.clone()).unwrap() else {
        panic!("SubscribeCOV")
    };
    assert_eq!(pdu.service_choice, ConfirmedServiceChoice::SUBSCRIBE_COV);
    let answer = Apdu::SimpleAck(SimpleAck {
        invoke_id: pdu.invoke_id,
        service_choice: pdu.service_choice,
    });
    send(&peer, &request.source_mac, answer).await;
    assert!(timeout(WAIT, subscribe).await.unwrap().unwrap().is_ok());
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

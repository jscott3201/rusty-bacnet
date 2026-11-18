//! The full client's audited requests past the 64 slots, and their pacing
//! (#2617): a request past the slots waits for one in its caller's task,
//! and the caller's turn at the destination travels with the request.
use super::*;

/// 65 audited reads at once: 64 go, the 65th waits for a slot and goes
/// once one comes back, and all 65 are recorded. A waiter whose caller
/// gives up sends nothing and holds no slot.
#[tokio::test(start_paused = true)]
async fn audited_requests_past_the_slots_wait_for_one() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = full_client(&session);
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
    let reads: Vec<_> = (0..65)
        .map(|_| {
            full_read(
                &client,
                peer.local_mac(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
        })
        .collect();
    let mut sent = Vec::new();
    for _ in 0..64 {
        sent.push(receive(&mut requests).await);
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(requests.try_recv().is_err(), "the 65th waits for a slot");
    assert_eq!(source.available_operations(), 0);
    let quitter = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::OBJECT_NAME,
        None,
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    quitter.abort();
    let _ = quitter.await;
    // Each answer frees a slot; the first lets the 65th in.
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
    assert_eq!(recorded, 65);
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
        source.available_operations() == 64
    })
    .await;
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A request waiting for a slot when the session stops fails, sends
/// nothing and records nothing, and neither do the 64 it waited behind.
#[tokio::test(start_paused = true)]
async fn a_stop_ends_the_wait_for_a_slot() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = full_client(&session);
    let reads: Vec<_> = (0..65)
        .map(|_| {
            full_read(
                &client,
                peer.local_mac(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
        })
        .collect();
    for _ in 0..64 {
        let _ = receive(&mut requests).await;
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    session.stop().await.unwrap();
    for read in reads {
        assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
    }
    assert!(requests.try_recv().is_err(), "the waiter sent nothing");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(records.try_recv().is_err());
    assert_eq!(session.coordinator.active_count().unwrap(), 0);
    drop(client);
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// The minimum request interval holds for an audited client, and the
/// caller's turn travels with its request: when the caller goes away, the
/// next request to the destination still waits for the first one's end
/// and then the interval.
#[tokio::test(start_paused = true)]
async fn the_turn_at_the_destination_travels_with_an_audited_request() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.client_config.min_request_interval_ms = 5_000;
    session.client_config.apdu_timeout_ms = 60_000;
    session.start().await.unwrap();
    let client = full_client(&session);
    let first = full_read(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first_request = receive(&mut requests).await;
    let sent = tokio::time::Instant::now();
    first.abort();
    let _ = first.await;
    let second = full_read(
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

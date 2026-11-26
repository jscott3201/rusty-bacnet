//! Where an admitted audited request is planned (#2755): in its caller's
//! poll when the database is free, as it always was, and in a task the
//! session owns when it is busy; and how either path ends when the plan or
//! the task's spawn is refused, or a stop comes just as the task gets the
//! guard.
use super::*;

/// The sequence number the next record without a clock time takes.
fn next_number(db: &ObjectDatabase) -> u16 {
    db.reserve_event_sequence_number().number()
}

/// A station on this link that is the B/IP broadcast address: an audited
/// read of it is admitted, in its own lane, and refused by its plan.
fn broadcast() -> Station {
    Station::local(&encode_bip_mac([255; 4], PORT))
}

const ONE_STATION: &str = "audited operation requires direct B/IP IPv4 unicast or a routed station";

/// With the database free, a read is planned in the poll that admits it:
/// two reads polled once each, in turn, have taken their numbers in that
/// order before anything else runs. With it busy, a third read's task
/// plans it once the guard is free, and the third takes the next number.
#[tokio::test(start_paused = true)]
async fn a_read_is_planned_at_admission_when_the_database_is_free() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    let (first, second) = (Station::local(&mac(30)), Station::local(&mac(31)));
    let mut reads = [read_of(&client, &first), read_of(&client, &second)];
    for read in &mut reads {
        assert!(poll_once(read).is_pending());
    }
    assert_eq!(next_number(&db.try_read().unwrap()), 2, "both planned");
    let guard = db.write().await;
    let third = Station::local(&mac(32));
    let mut busy = read_of(&client, &third);
    assert!(poll_once(&mut busy).is_pending());
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(next_number(&guard), 2, "the third waits in its task");
    drop(guard);
    let frames = endpoint.frames(3).await;
    let requests: [_; 3] = frames.requests.try_into().expect("three reads");
    assert_eq!(next_number(&db.try_read().unwrap()), 3);
    for request in &requests {
        endpoint.answer(request).await;
    }
    let mut numbered: Vec<_> = (endpoint.frames(3).await.records.iter())
        .map(|record| match &record.source_timestamp {
            Some(BACnetTimeStamp::SequenceNumber(number)) => (*number, record.invoke_id),
            other => panic!("a sequence number, not {other:?}"),
        })
        .collect();
    numbered.sort_unstable();
    let expected: Vec<_> = (0..)
        .zip(&requests)
        .map(|(number, (_, request))| (number, Some(request.invoke_id)))
        .collect();
    assert_eq!(numbered, expected, "30, 31 and 32 numbered in turn");
    for read in reads.into_iter().chain([busy]) {
        assert!(bounded(read).await.is_ok());
    }
    drop(client);
    endpoint.stop().await;
}

/// A plan refused, here for a station that is not one, reaches the caller
/// with its own error on either path: refused in the caller's poll with the
/// database free, or handed back by the task with it busy. Each gives its
/// slot back and sends nothing.
#[tokio::test(start_paused = true)]
async fn a_refused_plan_reaches_its_caller_on_either_path() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    let station = broadcast();
    let mut free = read_of(&client, &station);
    match poll_once(&mut free) {
        Poll::Ready(Err(Error::InvalidArgument(message))) => {
            assert_eq!(message.to_string(), ONE_STATION);
        }
        other => panic!("refused in the admitting poll, not {other:?}"),
    }
    drop(free);
    assert_eq!(source.available_operations(), SLOTS);
    let guard = db.write().await;
    let mut busy = read_of(&client, &station);
    assert!(poll_once(&mut busy).is_pending());
    assert_eq!(source.available_operations(), SLOTS - 1);
    drop(guard);
    match bounded(busy).await {
        Err(Error::InvalidArgument(message)) => assert_eq!(message.to_string(), ONE_STATION),
        other => panic!("handed back refused, not {other:?}"),
    }
    assert_eq!(source.available_operations(), SLOTS);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(endpoint.sent.try_recv().is_err(), "nothing was sent");
    drop(client);
    endpoint.stop().await;
}

/// A session whose task pool has closed between a read's admission and its
/// hand-off (here closed alone, as a stop closes it after sealing the
/// slots): the spawn is refused, and the read ends with the shutdown error
/// and its slot back, on either path, with nothing sent.
#[tokio::test(start_paused = true)]
async fn a_refused_spawn_ends_the_read_with_the_shutdown_error() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    endpoint.session.notifications.as_ref().unwrap().close();
    let station = direct_read();
    let shutdown = |result: Result<ReadPropertyACK, Error>| {
        matches!(result, Err(Error::State(bacnet_types::error::Lifecycle::Closed, message))
            if message.to_string() == "endpoint shutdown")
    };
    assert!(shutdown(bounded(read_of(&client, &station)).await), "free");
    assert_eq!(source.available_operations(), SLOTS);
    let guard = db.write().await;
    assert!(shutdown(bounded(read_of(&client, &station)).await), "busy");
    assert_eq!(source.available_operations(), SLOTS);
    drop(guard);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(endpoint.sent.try_recv().is_err(), "nothing was sent");
    drop(client);
    endpoint.stop().await;
}

/// A stop that begins just after the guard went to a waiting read's task,
/// before the task has run: the task runs first and may send the read, the
/// stop then cuts it off, and the read ends cut off as one that may have
/// been sent (#1833), unrecorded. On the old path the parked caller owned the guard and the
/// stop waited for it for ever.
#[tokio::test(start_paused = true)]
async fn a_stop_just_after_the_task_was_handed_the_guard() {
    let mut endpoint = source(SessionRole::Both, false).await;
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let client = endpoint.the_client();
    let station = direct_read();
    let guard = db.write().await;
    let mut parked = read_of(&client, &station);
    assert!(poll_once(&mut parked).is_pending());
    tokio::time::sleep(Duration::from_millis(1)).await;
    drop(guard);
    bounded(endpoint.session.stop()).await.unwrap();
    assert!(matches!(
        bounded(parked).await,
        Err(Error::RequestCutOff { sent: true })
    ));
    while let Ok(sent) = endpoint.sent.try_recv() {
        assert_eq!(sent.link, mac(PEER), "only the read itself, no record");
        let npdu = decode_npdu(sent.npdu).unwrap();
        assert!(matches!(
            decode_apdu(npdu.payload).unwrap(),
            Apdu::ConfirmedRequest(_)
        ));
    }
}

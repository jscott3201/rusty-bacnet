//! The session's local write of its Audit recipient waits for the database
//! in a task the session owns when the database is busy, not in its
//! caller's (#2759), as an audited request does (#2755); and sampling its
//! policy counters is synchronous. Each recipient write here is polled once
//! by hand and then held, unpolled.
use super::*;

/// Poll `future` once, with a waker that wakes nothing: its caller then
/// stops polling it.
fn poll_now<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

/// What the session sent next, `count` frames: the invoke IDs of its
/// answers, and its Audit records by the station they went to.
async fn answers_and_records(
    sent: &mut mpsc::Receiver<Sent>,
    count: usize,
) -> (Vec<u8>, Vec<Sent>) {
    let (mut answers, mut records) = (Vec::new(), Vec::new());
    for _ in 0..count {
        let frame = bounded(sent.recv()).await.expect("capture link open");
        let npdu = decode_npdu(frame.npdu.clone()).unwrap();
        match decode_apdu(npdu.payload).unwrap() {
            Apdu::ComplexAck(ack) => answers.push(ack.invoke_id),
            _ => records.push(frame),
        }
    }
    records.sort_by_key(|frame| frame.link.to_vec());
    (answers, records)
}

/// A recipient write polled once while the application held the database,
/// and never again, holds up nothing once the guard is free: a request the
/// node serves is answered, and the change's two records go out, the
/// session's task having made the change. Polled again, the call returns.
#[tokio::test(start_paused = true)]
async fn a_recipient_write_whose_caller_stopped_polling_holds_no_guard() {
    let mut endpoint = source(SessionRole::Both, true).await;
    let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
    let guard = db.write().await;
    let next = address(THIS_NETWORK, NEW_SINK);
    let mut parked = Box::pin(endpoint.session.write_audit_recipient(Some(next)));
    assert!(poll_now(parked.as_mut()).is_pending());
    tokio::time::sleep(Duration::from_millis(1)).await;
    drop(guard);
    endpoint.served_read(7).await;
    let (answers, records) = answers_and_records(&mut endpoint.sent, 3).await;
    assert_eq!(answers, [7], "the node served the peer");
    let [old, new]: [Sent; 2] = records.try_into().ok().expect("the change's two records");
    assert_eq!(record(old, SINK), record(new, NEW_SINK));
    assert!(matches!(poll_now(parked.as_mut()), Poll::Ready(Ok(()))));
    drop(parked);
    endpoint.stop().await;
}

/// A recipient write whose caller goes while it waits for the database,
/// before the guard comes free or just as it does, changes nothing and
/// sends nothing: the same write, awaited after, still reports the change
/// from the recipient the session started with.
#[tokio::test(start_paused = true)]
async fn a_recipient_write_whose_caller_went_changes_nothing() {
    for gone_first in [true, false] {
        let mut endpoint = source(SessionRole::ClientOnly, true).await;
        let db = Arc::clone(endpoint.session.database.as_ref().unwrap());
        let guard = db.write().await;
        let next = address(THIS_NETWORK, NEW_SINK);
        let mut parked = Box::pin(endpoint.session.write_audit_recipient(Some(next.clone())));
        assert!(poll_now(parked.as_mut()).is_pending());
        tokio::time::sleep(Duration::from_millis(1)).await;
        if gone_first {
            drop(parked);
            drop(guard);
        } else {
            // The guard goes to the write's wait, and its caller goes
            // before that wait is polled again.
            drop(guard);
            drop(parked);
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(db.try_write().is_ok(), "no guard is held");
        assert!(endpoint.sent.try_recv().is_err(), "nothing was sent");
        bounded(endpoint.session.write_audit_recipient(Some(next)))
            .await
            .unwrap();
        let (_, records) = answers_and_records(&mut endpoint.sent, 2).await;
        let [old, new]: [Sent; 2] = records.try_into().ok().expect("the change's two records");
        assert_eq!(record(old, SINK), record(new, NEW_SINK), "{gone_first}");
        endpoint.stop().await;
    }
}

/// Sampling the policy counters is synchronous, under the lock the
/// dispatch task counts under, so a sample never waits on that task, and
/// the dispatch goes on counting what it handles between samples.
#[tokio::test(start_paused = true)]
async fn policy_counters_follow_the_dispatch_task() {
    let endpoint = source(SessionRole::ClientOnly, true).await;
    let before = endpoint.session.policy_counters();
    endpoint.served_read(7).await;
    eventually(WAIT, "the request no server takes is counted", async || {
        endpoint.session.policy_counters().no_server_role == before.no_server_role + 1
    })
    .await;
    endpoint.stop().await;
}

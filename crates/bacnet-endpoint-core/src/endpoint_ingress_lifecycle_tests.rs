use super::*;
use bacnet_transport::test_transport::{SendMode, TestTransport};
use std::net::SocketAddrV4;
use std::sync::atomic::AtomicBool;
use tokio::sync::Semaphore;

/// `inner` with a stop that waits for `gate` (after telling `entered`) and
/// counts itself in `complete` before the inner stop, and a B/IP broadcast
/// endpoint whose port is 0 until the link has started.
fn gated_stop(
    inner: TestTransport,
    gate: Arc<Semaphore>,
    entered: Arc<Notify>,
    complete: Arc<AtomicUsize>,
) -> TestTransport {
    let started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&started);
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .on_bip_broadcast_endpoint(move || {
            Some(SocketAddrV4::new(
                [192, 168, 1, 255].into(),
                if started.load(Ordering::SeqCst) {
                    30001
                } else {
                    0
                },
            ))
        })
        .on_started(move |_| flag.store(true, Ordering::SeqCst))
        .before_stop(move || {
            entered.notify_one();
            let (gate, complete) = (Arc::clone(&gate), Arc::clone(&complete));
            async move {
                gate.acquire().await.unwrap().forget();
                complete.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
}
#[tokio::test]
async fn ingress_canceled_stop_retains_join_and_actual_post_start_bip_fact() {
    let (inner, handle) = test_transport();
    let gate = Arc::new(Semaphore::new(0));
    let entered = Arc::new(Notify::new());
    let complete = Arc::new(AtomicUsize::new(0));
    let mut endpoint = EndpointIngress::new(
        gated_stop(inner, gate.clone(), entered.clone(), complete.clone()),
        4,
    );
    assert_eq!(endpoint.bip_broadcast_endpoint().unwrap().port(), 0);
    // Before its start it is not running, and can start (#1833).
    assert!(matches!(
        endpoint.stop().await,
        Err(Error::State(error::Lifecycle::NotRunning, ref text))
            if text.to_string() == "endpoint ingress is not running"
    ));
    let receivers = endpoint.start().await.unwrap();
    assert_eq!(receivers.bip_broadcast_endpoint.unwrap().port(), 30001);
    {
        let stop = endpoint.stop();
        tokio::pin!(stop);
        tokio::select! {result=&mut stop=>panic!("stop completed prematurely: {result:?}"), _=entered.notified()=>{}}
    }
    assert_eq!(complete.load(Ordering::SeqCst), 0);
    // The stop that was dropped is still under way.
    assert!(matches!(
        endpoint.client_seat(
            Arc::new(crate::coordinator::OutboundTransactionCoordinator::new()),
            crate::seat::SeatRoles::new(false),
            4,
        ),
        Err(Error::State(error::Lifecycle::Stopping, _))
    ));
    gate.add_permits(1);
    assert!(matches!(
        timeout(WAIT, endpoint.stop()).await.unwrap().unwrap(),
        ClassifierExit::Cancelled
    ));
    assert_eq!(complete.load(Ordering::SeqCst), 1);
    assert_eq!(handle.stops(), 1);
    // Stopped, it is closed for good: an ingress starts once.
    assert!(matches!(
        endpoint.stop().await,
        Err(Error::State(error::Lifecycle::Closed, _))
    ));
    assert!(matches!(
        endpoint.start().await,
        Err(Error::State(error::Lifecycle::AlreadyStarted, _))
    ));
    drop(receivers);
    drop(handle);
}

/// A session task that ended under the ingress, here aborted, leaves it
/// shut for good: its stop says so, with the task's end (#1833).
#[tokio::test]
async fn a_session_task_that_ended_stops_the_ingress_closed() {
    let (inner, _handle) = test_transport();
    let mut endpoint = EndpointIngress::new(inner, 4);
    let _receivers = endpoint.start().await.unwrap();
    endpoint.session_task.as_ref().unwrap().abort();
    let stopped = timeout(WAIT, endpoint.stop()).await.unwrap().unwrap_err();
    assert!(
        matches!(&stopped, Error::State(error::Lifecycle::Closed, text)
            if text.to_string().starts_with("endpoint ingress session failed: ")),
        "{stopped:?}"
    );
}

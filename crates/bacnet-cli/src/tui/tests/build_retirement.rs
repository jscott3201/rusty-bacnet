//! An in-progress build and its non-cancel-safe stop remain owned to completion.

use crate::tui::worker::{EventSink, session};
use bacnet_client::client::BACnetClient;
use bacnet_transport::test_transport::{BIP_LOCAL_MAC, TestTransport};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, watch};

/// How long a step of the test may take before it has failed.
const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A port whose start and stop each tell the test they began and wait until
/// it lets them finish; the stop then marks the port released.
fn controlled_port(
    starting: Arc<Notify>,
    finish_start: Arc<Notify>,
    stopping: Arc<Notify>,
    finish_stop: Arc<Notify>,
    released: Arc<AtomicBool>,
) -> TestTransport {
    TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .start_gate(move || {
            starting.notify_one();
            let finish = Arc::clone(&finish_start);
            async move {
                finish.notified().await;
                Ok(())
            }
        })
        .on_stop(move || {
            stopping.notify_one();
            let (finish, released) = (Arc::clone(&finish_stop), Arc::clone(&released));
            async move {
                finish.notified().await;
                released.store(true, Ordering::SeqCst);
                Ok(())
            }
        })
        .build()
}

#[tokio::test]
async fn cancelled_partial_build_is_finished_and_stopped_without_publishing() {
    let starting = Arc::new(Notify::new());
    let finish_start = Arc::new(Notify::new());
    let stopping = Arc::new(Notify::new());
    let finish_stop = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let port = controlled_port(
        starting.clone(),
        finish_start.clone(),
        stopping.clone(),
        finish_stop.clone(),
        released.clone(),
    );
    let (sink, mut events) = EventSink::channel(1);
    let (cancel, cancelled) = watch::channel(false);
    let sink = sink.for_session(1, cancelled);
    let task = tokio::spawn(async move {
        let builder = BACnetClient::generic_builder().transport(port).build();
        assert!(
            session::finish_build(Box::pin(builder), &sink)
                .await
                .is_none()
        );
    });
    tokio::time::timeout(WAIT, starting.notified())
        .await
        .expect("the build never reached the port's start");
    cancel.send_replace(true);
    assert!(
        !task.is_finished(),
        "cancellation must not drop a partially completed builder"
    );
    finish_start.notify_one();
    tokio::time::timeout(WAIT, stopping.notified())
        .await
        .expect("the cancelled build never stopped the port");
    assert!(
        !task.is_finished(),
        "the stop future remains owned until resource release"
    );
    assert!(!released.load(Ordering::SeqCst));
    finish_stop.notify_one();
    tokio::time::timeout(WAIT, task)
        .await
        .expect("the build task never finished after the stop was released")
        .unwrap();
    assert!(released.load(Ordering::SeqCst));
    assert!(
        events.try_recv().is_err(),
        "cancelled build must publish no connected/device event"
    );
}

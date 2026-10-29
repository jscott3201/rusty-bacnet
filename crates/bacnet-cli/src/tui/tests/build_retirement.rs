//! An in-progress build and its non-cancel-safe stop remain owned to completion.

use crate::tui::worker::{EventSink, session};
use bacnet_client::client::BACnetClient;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, mpsc, watch};

struct ControlledPort {
    starting: Arc<Notify>,
    finish_start: Arc<Notify>,
    stopping: Arc<Notify>,
    finish_stop: Arc<Notify>,
    released: Arc<AtomicBool>,
}
impl TransportPort for ControlledPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.starting.notify_one();
        self.finish_start.notified().await;
        Ok(mpsc::channel(1).1)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.stopping.notify_one();
        self.finish_stop.notified().await;
        self.released.store(true, Ordering::SeqCst);
        Ok(())
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xba, 0xc0]
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}

#[tokio::test]
async fn cancelled_partial_build_is_finished_and_stopped_without_publishing() {
    let starting = Arc::new(Notify::new());
    let finish_start = Arc::new(Notify::new());
    let stopping = Arc::new(Notify::new());
    let finish_stop = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let port = ControlledPort {
        starting: starting.clone(),
        finish_start: finish_start.clone(),
        stopping: stopping.clone(),
        finish_stop: finish_stop.clone(),
        released: released.clone(),
    };
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
    starting.notified().await;
    cancel.send_replace(true);
    assert!(
        !task.is_finished(),
        "cancellation must not drop a partially completed builder"
    );
    finish_start.notify_one();
    stopping.notified().await;
    assert!(
        !task.is_finished(),
        "the stop future remains owned until resource release"
    );
    assert!(!released.load(Ordering::SeqCst));
    finish_stop.notify_one();
    task.await.unwrap();
    assert!(released.load(Ordering::SeqCst));
    assert!(
        events.try_recv().is_err(),
        "cancelled build must publish no connected/device event"
    );
}

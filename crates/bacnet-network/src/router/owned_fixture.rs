//! Gated owners prove attempted starts, explicit cleanup, and child destruction.
//!
//! The port is a [`TestTransport`] whose start, stop, abort and send hooks
//! spawn and tear down a child task that owns a drop-counted resource, and
//! report each step to the test through a [`Probe`].

use super::RouterPort;
use bacnet_transport::test_transport::{StartMode, TestTransport};
use bacnet_types::error::Error;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct Resource(Arc<AtomicUsize>, Option<oneshot::Sender<()>>);
impl Drop for Resource {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        if let Some(dropped) = self.1.take() {
            let _ = dropped.send(());
        }
    }
}

/// Faults a test arms on a port, before the router starts it.
#[derive(Default)]
struct Knobs {
    start_error: Mutex<Option<Error>>,
    stop_error: Mutex<Option<Error>>,
    pending_start: AtomicBool,
    panic_send: AtomicBool,
    panic_stop: AtomicBool,
}

fn take<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).take()
}

pub(super) struct Probe {
    pub(super) stopped: Arc<AtomicUsize>,
    pub(super) aborted: Arc<AtomicUsize>,
    pub(super) dropped: Arc<AtomicUsize>,
    pub(super) stop_entered: oneshot::Receiver<()>,
    pub(super) stop_release: oneshot::Sender<()>,
    pub(super) send_entered: oneshot::Receiver<()>,
    pub(super) starts: Arc<AtomicUsize>,
    pub(super) start_entered: oneshot::Receiver<()>,
    pub(super) start_release: oneshot::Sender<()>,
    pub(super) completed: oneshot::Receiver<()>,
    pub(super) resource_drop: oneshot::Receiver<()>,
    knobs: Arc<Knobs>,
}

impl Probe {
    /// Make the start fail with `error`, after its child is spawned.
    pub(super) fn fail_start(&self, error: Error) {
        *self.knobs.start_error.lock().unwrap() = Some(error);
    }

    /// Hold the start until `start_release` is sent (or dropped).
    pub(super) fn hold_start(&self) {
        self.knobs.pending_start.store(true, Ordering::SeqCst);
    }

    /// Make the stop return `error`, after it cleans up.
    pub(super) fn fail_stop(&self, error: Error) {
        *self.knobs.stop_error.lock().unwrap() = Some(error);
    }

    /// Make the first send panic.
    pub(super) fn panic_on_send(&self) {
        self.knobs.panic_send.store(true, Ordering::SeqCst);
    }

    /// Make the stop panic, once released.
    pub(super) fn panic_on_stop(&self) {
        self.knobs.panic_stop.store(true, Ordering::SeqCst);
    }
}

pub(super) fn port(pending_send: bool) -> (RouterPort<TestTransport>, Probe) {
    port_at(pending_send, &[1])
}

/// [`port`], reporting `mac` as its local MAC.
pub(super) fn port_at(pending_send: bool, mac: &[u8]) -> (RouterPort<TestTransport>, Probe) {
    let starts = Arc::new(AtomicUsize::new(0));
    let (start_tx, start_entered) = oneshot::channel();
    let (start_release, start_rx) = oneshot::channel();
    let (completed_tx, completed) = oneshot::channel();
    let (resource_tx, resource_drop) = oneshot::channel();
    let stopped = Arc::new(AtomicUsize::new(0));
    let aborted = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let (stop_tx, stop_entered) = oneshot::channel();
    let (stop_release, stop_rx) = oneshot::channel();
    let (send_tx, send_entered) = oneshot::channel();
    let knobs = Arc::new(Knobs::default());
    let child: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::default();
    let (start_tx, start_rx) = (Mutex::new(Some(start_tx)), Mutex::new(Some(start_rx)));
    let resource_tx = Mutex::new(Some(resource_tx));
    let (stop_tx, stop_rx) = (Mutex::new(Some(stop_tx)), Mutex::new(Some(stop_rx)));
    let (send_tx, completed_tx) = (Mutex::new(Some(send_tx)), Mutex::new(Some(completed_tx)));
    let transport = TestTransport::builder()
        .local_mac(mac)
        .start(StartMode::Fresh(1))
        .on_start({
            let starts = Arc::clone(&starts);
            move || {
                starts.fetch_add(1, Ordering::SeqCst);
            }
        })
        .start_gate({
            let (knobs, dropped, child) = (Arc::clone(&knobs), Arc::clone(&dropped), child.clone());
            let (start_tx, start_rx) = (Arc::new(start_tx), Arc::new(start_rx));
            let resource_tx = Arc::new(resource_tx);
            move || {
                let resource = Resource(Arc::clone(&dropped), take(&resource_tx));
                *child.lock().unwrap() = Some(tokio::spawn(async move {
                    let _resource = resource;
                    std::future::pending::<()>().await;
                }));
                let _ = take(&start_tx).unwrap().send(());
                let (knobs, start_rx) = (Arc::clone(&knobs), Arc::clone(&start_rx));
                async move {
                    if knobs.pending_start.load(Ordering::SeqCst) {
                        let _ = take(&start_rx).unwrap().await;
                    }
                    take(&knobs.start_error).map_or(Ok(()), Err)
                }
            }
        })
        .on_stop({
            let (knobs, stopped, child) = (Arc::clone(&knobs), Arc::clone(&stopped), child.clone());
            let (stop_tx, stop_rx) = (Arc::new(stop_tx), Arc::new(stop_rx));
            let completed_tx = Arc::new(completed_tx);
            move || {
                stopped.fetch_add(1, Ordering::SeqCst);
                let _ = take(&stop_tx).unwrap().send(());
                let (knobs, stop_rx, child) =
                    (Arc::clone(&knobs), Arc::clone(&stop_rx), child.clone());
                let completed_tx = Arc::clone(&completed_tx);
                async move {
                    let _ = take(&stop_rx).unwrap().await;
                    assert!(
                        !knobs.panic_stop.load(Ordering::SeqCst),
                        "named transport stop panic"
                    );
                    let child = take(&child).unwrap();
                    child.abort();
                    assert!(child.await.unwrap_err().is_cancelled());
                    let _ = take(&completed_tx).unwrap().send(());
                    take(&knobs.stop_error).map_or(Ok(()), Err)
                }
            }
        })
        .on_abort({
            let (aborted, child) = (Arc::clone(&aborted), child.clone());
            move || {
                aborted.fetch_add(1, Ordering::SeqCst);
                if let Some(child) = take(&child) {
                    child.abort();
                }
            }
        })
        .on_send({
            let knobs = Arc::clone(&knobs);
            let send_tx = Arc::new(send_tx);
            move |_| {
                if let Some(entered) = take(&send_tx) {
                    let _ = entered.send(());
                }
                assert!(
                    !knobs.panic_send.load(Ordering::SeqCst),
                    "named sender panic"
                );
                async move {
                    if pending_send {
                        std::future::pending::<()>().await;
                    }
                    Ok(())
                }
            }
        })
        .build();
    (
        RouterPort {
            network_number: 1,
            transport,
        },
        Probe {
            starts,
            start_entered,
            start_release,
            completed,
            resource_drop,
            stopped,
            aborted,
            dropped,
            stop_entered,
            stop_release,
            send_entered,
            knobs,
        },
    )
}

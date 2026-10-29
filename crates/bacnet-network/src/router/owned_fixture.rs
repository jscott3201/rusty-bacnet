//! Gated owners prove attempted starts, explicit cleanup, and child destruction.

use super::RouterPort;
use bacnet_npdu::NpduLimits;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
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

pub(super) struct OwnedPort {
    pub(super) stopped: Arc<AtomicUsize>,
    pub(super) aborted: Arc<AtomicUsize>,
    pub(super) dropped: Arc<AtomicUsize>,
    pub(super) stop_entered: Option<oneshot::Sender<()>>,
    pub(super) stop_release: Option<oneshot::Receiver<()>>,
    pub(super) send_entered: Mutex<Option<oneshot::Sender<()>>>,
    pending_send: bool,
    child: Option<JoinHandle<()>>,
    incoming: Option<mpsc::Sender<ReceivedNpdu>>,
    pub(super) starts: Arc<AtomicUsize>,
    pub(super) start_entered: Option<oneshot::Sender<()>>,
    pub(super) start_release: Option<oneshot::Receiver<()>>,
    pub(super) pending_start: bool,
    pub(super) start_error: Option<Error>,
    pub(super) stop_error: Option<Error>,
    pub(super) panic_send: bool,
    pub(super) panic_stop: bool,
    resource_drop: Option<oneshot::Sender<()>>,
    pub(super) mac: Vec<u8>,
    completed: Option<oneshot::Sender<()>>,
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
}

pub(super) fn port(pending_send: bool) -> (RouterPort<OwnedPort>, Probe) {
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
    (
        RouterPort {
            network_number: 1,
            transport: OwnedPort {
                stopped: Arc::clone(&stopped),
                aborted: Arc::clone(&aborted),
                dropped: Arc::clone(&dropped),
                stop_entered: Some(stop_tx),
                stop_release: Some(stop_rx),
                send_entered: Mutex::new(Some(send_tx)),
                pending_send,
                child: None,
                incoming: None,
                starts: Arc::clone(&starts),
                start_entered: Some(start_tx),
                start_release: Some(start_rx),
                pending_start: false,
                start_error: None,
                stop_error: None,
                panic_send: false,
                panic_stop: false,
                resource_drop: Some(resource_tx),
                mac: vec![1],
                completed: Some(completed_tx),
            },
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
        },
    )
}

impl OwnedPort {
    async fn send(&self) -> Result<(), Error> {
        if let Some(entered) = self.send_entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        assert!(!self.panic_send, "named sender panic");
        if self.pending_send {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

impl TransportPort for OwnedPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let resource = Resource(Arc::clone(&self.dropped), self.resource_drop.take());
        self.child = Some(tokio::spawn(async move {
            let _resource = resource;
            std::future::pending::<()>().await;
        }));
        let _ = self.start_entered.take().unwrap().send(());
        if self.pending_start {
            let _ = self.start_release.take().unwrap().await;
        }
        if let Some(error) = self.start_error.take() {
            return Err(error);
        }
        let (tx, rx) = mpsc::channel(1);
        self.incoming = Some(tx);
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        let _ = self.stop_entered.take().unwrap().send(());
        let _ = self.stop_release.take().unwrap().await;
        assert!(!self.panic_stop, "named transport stop panic");
        self.incoming.take();
        let child = self.child.take().unwrap();
        child.abort();
        assert!(child.await.unwrap_err().is_cancelled());
        let _ = self.completed.take().unwrap().send(());
        match self.stop_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        self.send().await
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        self.send().await
    }
    fn abort(&mut self) {
        self.aborted.fetch_add(1, Ordering::SeqCst);
        self.incoming.take();
        if let Some(child) = self.child.take() {
            child.abort();
        }
    }
    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
    fn max_npdu(&self) -> NpduLimits {
        NpduLimits::default()
    }
}

//! A transport need not await: ready sends must still share the runtime.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_npdu::Mac;
use bacnet_npdu::NpduLimits;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};

use super::super::SendRequest;
use super::super::control_policy::ControlGate;
use super::super::dispatch::Shared;
use super::super::engine::{Engine, OPTIONS, tokio_clock};
use super::super::port_groups::PortGroups;
use super::super::send_queue;
use bacnet_router_core::PortId;

const REFILL_LIMIT: usize = 1024;

fn request() -> SendRequest {
    SendRequest::Broadcast {
        npdu: Bytes::from_static(&[1, 0, 0x10, 8]),
        data_attributes: Vec::new(),

        reject_origin: None,
    }
}

fn shared() -> Arc<Shared> {
    let port = Engine::port(
        1000,
        Mac::from_array([1]),
        NpduLimits::default(),
        PortGroups::default(),
    );
    Arc::new(Shared::new(
        Engine::new(&[port], tokio_clock(), OPTIONS).unwrap(),
        Arc::new(ControlGate::permissive()),
        vec![1000],
    ))
}

struct ReadyPort {
    calls: Arc<AtomicUsize>,
    refill: Option<send_queue::Sender>,
    started: Mutex<Option<oneshot::Sender<()>>>,
}

impl ReadyPort {
    fn send(&self) {
        let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        // Even the broken sender returns Pending after finite work. This
        // fixture cannot hang the runtime when testing its original behavior.
        if count < REFILL_LIMIT
            && let Some(refill) = &self.refill
        {
            refill
                .try_send(bacnet_npdu::NetworkPriority::Normal, request())
                .unwrap();
        }
    }
}

impl TransportPort for ReadyPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        panic!("the sender does not start transports")
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        self.send();
        Ok(())
    }

    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        self.send();
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &[1]
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}

#[tokio::test]
async fn ready_transport_yields_before_draining_a_full_backlog() {
    let (tx, rx) = send_queue::channel(send_queue::CAPACITY);
    for _ in 0..send_queue::CAPACITY {
        tx.try_send(bacnet_npdu::NetworkPriority::Normal, request())
            .unwrap();
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let port = ReadyPort {
        calls: Arc::clone(&calls),
        refill: None,
        started: Mutex::new(None),
    };
    // Poll the sender first. A ready observer can run only once the sender
    // returns Pending. The locked Tokio version's budget is smaller than
    // the 256-item backlog, so cooperation must leave some work queued.
    tokio::select! {
        biased;
        () = super::run(port, PortId(0), rx, shared(), std::sync::Weak::new()) => panic!("a sender still lives"),
        () = std::future::ready(()) => {}
    }
    let sent = calls.load(Ordering::SeqCst);
    assert!(sent > 0, "the sender was polled before the observer");
    assert!(
        sent < send_queue::CAPACITY,
        "one poll sent all {sent} items"
    );
    assert!(matches!(
        tx.try_send(bacnet_npdu::NetworkPriority::Normal, request()),
        Err(TrySendError::Closed(_))
    ));
}

#[tokio::test]
async fn a_replenished_ready_sender_can_be_aborted_and_joined_before_exhaustion() {
    let (tx, rx) = send_queue::channel(send_queue::CAPACITY);
    tx.try_send(bacnet_npdu::NetworkPriority::Normal, request())
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (started, observe) = oneshot::channel();
    let port = ReadyPort {
        calls: Arc::clone(&calls),
        refill: Some(tx.clone()),
        started: Mutex::new(Some(started)),
    };
    let sender = tokio::spawn(super::run(
        port,
        PortId(0),
        rx,
        shared(),
        std::sync::Weak::new(),
    ));
    observe.await.unwrap();
    let sent_before_abort = calls.load(Ordering::SeqCst);
    sender.abort();
    let cancelled = tokio::time::timeout(Duration::from_secs(2), sender)
        .await
        .expect("sender must yield to abort and join")
        .unwrap_err();
    assert!(cancelled.is_cancelled());
    assert!(sent_before_abort > 0);
    assert!(
        sent_before_abort < REFILL_LIMIT,
        "observer only ran after {sent_before_abort} sends exhausted the fixture"
    );
    assert_eq!(calls.load(Ordering::SeqCst), sent_before_abort);
    assert!(matches!(
        tx.try_send(bacnet_npdu::NetworkPriority::Normal, request()),
        Err(TrySendError::Closed(_))
    ));
}

//! A transport need not await: ready sends must still share the runtime.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_npdu::Mac;
use bacnet_npdu::NpduLimits;
use bacnet_transport::test_transport::{StartMode, TestTransport};
use bytes::Bytes;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::oneshot;

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

/// A port whose sends finish at once, never awaiting anything: each counts
/// itself and, up to [`REFILL_LIMIT`], queues another request through
/// `refill`. Even the broken sender returns Pending after finite work, so
/// this fixture cannot hang the runtime when testing its original behavior.
fn ready_port(
    calls: Arc<AtomicUsize>,
    refill: Option<send_queue::Sender>,
    started: Option<oneshot::Sender<()>>,
) -> TestTransport {
    let started = Mutex::new(started);
    TestTransport::builder()
        .start(StartMode::Panic("the sender does not start transports"))
        .on_send(move |_| {
            let count = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(started) = started.lock().unwrap().take() {
                let _ = started.send(());
            }
            if count < REFILL_LIMIT
                && let Some(refill) = &refill
            {
                refill
                    .try_send(bacnet_npdu::NetworkPriority::Normal, request())
                    .unwrap();
            }
            async { Ok(()) }
        })
        .build()
}

#[tokio::test]
async fn ready_transport_yields_before_draining_a_full_backlog() {
    let (tx, rx) = send_queue::channel(send_queue::CAPACITY);
    for _ in 0..send_queue::CAPACITY {
        tx.try_send(bacnet_npdu::NetworkPriority::Normal, request())
            .unwrap();
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let port = ready_port(Arc::clone(&calls), None, None);
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
    let port = ready_port(Arc::clone(&calls), Some(tx.clone()), Some(started));
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

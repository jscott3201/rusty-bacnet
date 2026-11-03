//! Address-Resolution waiters leave `pending` on every exit (#2328), and
//! only the node asked can answer them (#2332).
//!
//! The discovery future is polled by hand so a test can stop it at a chosen
//! await (the hub write or the answer wait) and then drop it, as a caller's
//! deadline would. The clock is paused; nothing sleeps in real time.

use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll, Waker};

const TARGET: Vmac = [0x22; 6];
const OTHER: Vmac = [0x33; 6];
const URI: &str = "wss://peer.example/sc";
const WAIT_MS: u64 = 1_000;

#[derive(Clone, Copy, Debug)]
enum Write {
    Accept,
    Fail,
    Stall,
}

/// Hub socket that records each request's message ID, then accepts the
/// write, fails it or never finishes it.
struct Hub {
    write: Write,
    sent: StdMutex<Vec<u16>>,
    finished: AtomicUsize,
}

impl Hub {
    fn new(write: Write) -> Arc<Self> {
        Arc::new(Self {
            write,
            sent: StdMutex::new(Vec::new()),
            finished: AtomicUsize::new(0),
        })
    }

    fn finished(&self) -> usize {
        self.finished.load(Ordering::SeqCst)
    }

    fn sent(&self) -> Vec<u16> {
        self.sent.lock().unwrap().clone()
    }
}

impl WebSocketPort for Hub {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        let request = crate::sc_frame::decode_sc_message(data).unwrap();
        assert_eq!(request.function, ScFunction::AddressResolution);
        self.sent.lock().unwrap().push(request.message_id);
        let result = match self.write {
            Write::Accept => Ok(()),
            Write::Fail => Err(Error::Encoding("hub write failed".into())),
            Write::Stall => std::future::pending().await,
        };
        self.finished.fetch_add(1, Ordering::SeqCst);
        result
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        std::future::pending().await
    }
}

fn fixture() -> (DirectShared<Hub>, Arc<Mutex<ScConnection>>) {
    let direct = DirectShared::new(Arc::new(DirectMembership::default()));
    let mut conn = ScConnection::new([0x01; 6], [1; 16]);
    conn.state = ScConnectionState::Connected;
    (direct, Arc::new(Mutex::new(conn)))
}

fn waiting(direct: &DirectShared<Hub>) -> usize {
    lock_waiters(&direct.pending).by_id.len()
}

/// An Address-Resolution-ACK as the hub relays it from `from`.
fn ack_from(message_id: u16, from: Option<Vmac>) -> ScMessage {
    ScMessage {
        function: ScFunction::AddressResolutionAck,
        message_id,
        originating_vmac: from,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: bytes::Bytes::from_static(URI.as_bytes()),
    }
}

fn ack(message_id: u16) -> ScMessage {
    ack_from(message_id, Some(TARGET))
}

/// The NAK a node that accepts no direct connections returns, relayed from `from`.
fn nak_from(message_id: u16, from: Option<Vmac>) -> ScMessage {
    let mut nak = super::super::data_attributes::build_bvlc_result_nak(
        message_id,
        ScFunction::AddressResolution,
        0,
        None,
        ErrorClass::COMMUNICATION,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    nak.originating_vmac = from;
    nak
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[tokio::test(start_paused = true)]
async fn cancelled_discovery_leaves_no_waiter() {
    let mut failures = Vec::new();
    // Each stage names the await it stops at by the writes finished there.
    for (stage, write, finished) in [
        ("hub write", Write::Stall, 0),
        ("answer wait", Write::Accept, 1),
    ] {
        let (direct, conn) = fixture();
        let hub = Hub::new(write);
        let mut discovery = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
        if poll_once(discovery.as_mut()).is_ready() {
            failures.push(format!("{stage}: finished instead of waiting"));
            continue;
        }
        let stopped_at = (hub.sent().len(), hub.finished(), waiting(&direct));
        if stopped_at != (1, finished, 1) {
            failures.push(format!(
                "{stage}: (sent, finished, waiting) = {stopped_at:?}, expected (1, {finished}, 1)"
            ));
        }
        drop(discovery);
        if waiting(&direct) != 0 {
            failures.push(format!("{stage}: the cancelled request leaked its waiter"));
        }
        // A late answer for the cancelled request finds nothing to wake.
        direct.fulfill_from_hub_message(&ack(hub.sent()[0]));
        if waiting(&direct) != 0 || direct.cached_uris(&TARGET).await.is_some() {
            failures.push(format!("{stage}: a late answer found a waiter"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[tokio::test(start_paused = true)]
async fn failed_discovery_leaves_no_waiter() {
    for (exit, write) in [("write error", Write::Fail), ("timeout", Write::Accept)] {
        let (direct, conn) = fixture();
        let hub = Hub::new(write);
        let result = direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS).await;
        assert_eq!(result, None, "{exit}");
        assert_eq!(hub.sent().len(), 1, "{exit}");
        assert_eq!(waiting(&direct), 0, "{exit}: failed request leaked");
    }
}

#[tokio::test(start_paused = true)]
async fn cancelled_discovery_frees_its_message_id() {
    let (direct, conn) = fixture();
    let stalled = Hub::new(Write::Stall);
    let mut first = Box::pin(direct.discover_via_hub(TARGET, &stalled, &conn, WAIT_MS));
    assert!(poll_once(first.as_mut()).is_pending());
    drop(first);
    let id = stalled.sent()[0];

    // The message counter comes round to the cancelled request's ID again.
    conn.lock().await.next_message_id = id;
    let hub = Hub::new(Write::Accept);
    let mut second = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
    assert!(poll_once(second.as_mut()).is_pending());
    assert_eq!(hub.sent(), [id], "the cancelled request still held its ID");
    direct.fulfill_from_hub_message(&ack(id));
    assert_eq!(second.await, Some(vec![URI.to_owned()]));
    assert_eq!(waiting(&direct), 0);
}

/// An answered request whose caller hasn't run again must not remove a
/// newer request's waiter that reused its message ID, whether the older
/// caller is then cancelled or finishes normally.
#[tokio::test(start_paused = true)]
async fn stale_request_leaves_newer_waiter_with_same_id() {
    for finish_first in [false, true] {
        let (direct, conn) = fixture();
        let hub = Hub::new(Write::Accept);
        let mut first = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
        assert!(poll_once(first.as_mut()).is_pending());
        let id = hub.sent()[0];
        // The answer takes the first entry; its caller has not run since.
        direct.fulfill_from_hub_message(&ack(id));
        assert_eq!(waiting(&direct), 0);

        conn.lock().await.next_message_id = id;
        let mut second = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
        assert!(poll_once(second.as_mut()).is_pending());
        assert_eq!(hub.sent(), [id, id]);

        if finish_first {
            assert_eq!(first.await, Some(vec![URI.to_owned()]));
        } else {
            drop(first);
        }
        assert_eq!(
            waiting(&direct),
            1,
            "finish_first={finish_first}: the older request removed the newer waiter"
        );
        direct.fulfill_from_hub_message(&ack(id));
        assert_eq!(second.await, Some(vec![URI.to_owned()]));
        assert_eq!(waiting(&direct), 0);
    }
}

/// Answers that don't come from the node asked: a forged one from another
/// node, and one the hub never relays for a request (no originating VMAC).
#[tokio::test(start_paused = true)]
async fn answer_from_another_node_is_ignored() {
    let forged = [
        (
            "ACK from another node",
            ack_from as fn(u16, Option<Vmac>) -> ScMessage,
            Some(OTHER),
        ),
        ("ACK without an origin", ack_from, None),
        ("NAK from another node", nak_from, Some(OTHER)),
        ("NAK without an origin", nak_from, None),
    ];
    for (case, answer, from) in forged {
        let (direct, conn) = fixture();
        let hub = Hub::new(Write::Accept);
        let mut discovery = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
        assert!(poll_once(discovery.as_mut()).is_pending(), "{case}");
        let id = hub.sent()[0];
        direct.fulfill_from_hub_message(&answer(id, from));
        assert!(
            poll_once(discovery.as_mut()).is_pending(),
            "{case}: completed"
        );
        assert_eq!(waiting(&direct), 1, "{case}: the waiter was taken");
        assert_eq!(direct.cached_uris(&TARGET).await, None, "{case}: cached");
        direct.fulfill_from_hub_message(&ack(id));
        assert_eq!(discovery.await, Some(vec![URI.to_owned()]), "{case}");
    }
}

/// A late ACK for a timed-out request must not answer a newer request
/// that reused its message ID for another node.
#[tokio::test(start_paused = true)]
async fn late_answer_leaves_reused_id_waiter_pending() {
    let (direct, conn) = fixture();
    let hub = Hub::new(Write::Accept);
    assert_eq!(
        direct.discover_via_hub(OTHER, &hub, &conn, WAIT_MS).await,
        None
    );
    let id = hub.sent()[0];

    conn.lock().await.next_message_id = id;
    let mut discovery = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
    assert!(poll_once(discovery.as_mut()).is_pending());
    assert_eq!(hub.sent(), [id, id]);
    direct.fulfill_from_hub_message(&ack_from(id, Some(OTHER)));
    assert!(
        poll_once(discovery.as_mut()).is_pending(),
        "the late ACK answered"
    );
    assert_eq!(waiting(&direct), 1);
    assert_eq!(direct.cached_uris(&TARGET).await, None);
    assert_eq!(direct.cached_uris(&OTHER).await, None);
    direct.fulfill_from_hub_message(&ack(id));
    assert_eq!(discovery.await, Some(vec![URI.to_owned()]));
}

#[tokio::test(start_paused = true)]
async fn nak_from_the_node_asked_completes_empty() {
    let (direct, conn) = fixture();
    let hub = Hub::new(Write::Accept);
    let mut discovery = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
    assert!(poll_once(discovery.as_mut()).is_pending());
    direct.fulfill_from_hub_message(&nak_from(hub.sent()[0], Some(TARGET)));
    assert_eq!(discovery.await, Some(Vec::new()));
    assert_eq!(direct.cached_uris(&TARGET).await, Some(Vec::new()));
    assert_eq!(waiting(&direct), 0);
}

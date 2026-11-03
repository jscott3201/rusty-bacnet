//! Address-Resolution waiters leave `pending` on every exit (#2328).
//!
//! The discovery future is polled by hand so a test can stop it at a chosen
//! await (the hub write or the answer wait) and then drop it, as a caller's
//! deadline would. The clock is paused; nothing sleeps in real time.

use super::*;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll, Waker};

const TARGET: Vmac = [0x22; 6];
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
}

impl Hub {
    fn new(write: Write) -> Arc<Self> {
        Arc::new(Self {
            write,
            sent: StdMutex::new(Vec::new()),
        })
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
        match self.write {
            Write::Accept => Ok(()),
            Write::Fail => Err(Error::Encoding("hub write failed".into())),
            Write::Stall => std::future::pending().await,
        }
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
    direct.pending.lock().unwrap().by_id.len()
}

fn ack(message_id: u16) -> ScMessage {
    ScMessage {
        function: ScFunction::AddressResolutionAck,
        message_id,
        originating_vmac: Some(TARGET),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: bytes::Bytes::from_static(URI.as_bytes()),
    }
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[tokio::test(start_paused = true)]
async fn cancelled_discovery_leaves_no_waiter() {
    for (stage, write) in [("hub write", Write::Stall), ("answer wait", Write::Accept)] {
        let (direct, conn) = fixture();
        let hub = Hub::new(write);
        let mut discovery = Box::pin(direct.discover_via_hub(TARGET, &hub, &conn, WAIT_MS));
        assert!(poll_once(discovery.as_mut()).is_pending(), "{stage}");
        assert_eq!(waiting(&direct), 1, "{stage}: no waiter after the insert");
        drop(discovery);
        assert_eq!(waiting(&direct), 0, "{stage}: cancelled request leaked");
        // A late answer for the cancelled request finds nothing to wake.
        direct.fulfill_from_hub_message(&ack(hub.sent()[0]));
        assert_eq!(waiting(&direct), 0, "{stage}");
    }
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

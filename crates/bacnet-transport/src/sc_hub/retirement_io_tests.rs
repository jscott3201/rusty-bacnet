use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::GatedIo;
use super::retirement_tests::{heartbeat_wire, unicast_wire};
use super::shutdown_blocked_tests::ControlledPeer;
use super::*;
use tokio_tungstenite::tungstenite::{
    error::{CapacityError, ProtocolError},
    Error,
};

struct Failure(std::sync::Mutex<Option<Error>>);
impl relay_send::RelayIo for Failure {
    async fn send(&self, _: &mut WsSink, _: Message) -> Result<(), Error> {
        Err(self.0.lock().unwrap().take().unwrap())
    }
}

#[test]
fn relay_failure_classifier_distinguishes_terminal_state_from_capacity() {
    for error in [
        Error::ConnectionClosed,
        Error::AlreadyClosed,
        Error::Protocol(ProtocolError::SendAfterClosing),
        Error::Io(std::io::ErrorKind::BrokenPipe.into()),
        Error::Io(std::io::ErrorKind::ConnectionReset.into()),
        Error::Io(std::io::ErrorKind::UnexpectedEof.into()),
    ] {
        assert!(relay_send::terminal(&error), "{error:?}");
    }
    for error in nonterminal_errors() {
        assert!(!relay_send::terminal(&error), "{error:?}");
    }
}

fn nonterminal_errors() -> Vec<Error> {
    vec![
        Error::WriteBufferFull(Box::new(Message::Binary(Bytes::new()))),
        Error::Capacity(CapacityError::MessageTooLong {
            size: 2,
            max_size: 1,
        }),
        Error::Io(std::io::ErrorKind::WouldBlock.into()),
        Error::Protocol(ProtocolError::ResetWithoutClosingHandshake),
    ]
}

#[tokio::test]
async fn nonterminal_relay_failures_preserve_registered_usable_target() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = ControlledPeer::open(&tls, &hub).await;
    peer.connect(0x42).await;
    let target =
        HubRelaySink::capture([0x42; 6], hub.clients.lock().await.get(&[0x42; 6]).unwrap());
    for error in nonterminal_errors() {
        let io = Failure(std::sync::Mutex::new(Some(error)));
        assert!(
            relay_send::send(&target, &hub.clients, Message::Binary(Bytes::new()), &io)
                .await
                .is_err()
        );
        assert!(!target.closed.load(Ordering::Acquire));
        assert!(hub.clients.lock().await.contains_key(&[0x42; 6]));
        assert_eq!(hub.active.load(Ordering::Acquire), 1);
        peer.ws.send(heartbeat_wire(25)).await.unwrap();
        assert!(matches!(poll_io(peer.ws.next()).await,
            Some(Ok(Message::Binary(data))) if data[0..4] == [0x0B, 0, 0, 25]));
    }
    hub.hub.stop().await;
}

struct GatedFailure {
    entered: Notify,
    release: Notify,
}
impl relay_send::RelayIo for GatedFailure {
    async fn send(&self, _: &mut WsSink, _: Message) -> Result<(), Error> {
        self.entered.notify_one();
        self.release.notified().await;
        Err(Error::Io(std::io::ErrorKind::BrokenPipe.into()))
    }
}

#[tokio::test]
async fn delayed_terminal_failure_cannot_remove_replacement_after_registry_wait() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut old = ControlledPeer::open(&tls, &hub).await;
    old.connect(0x42).await;
    let target =
        HubRelaySink::capture([0x42; 6], hub.clients.lock().await.get(&[0x42; 6]).unwrap());
    let closed = target.closed.clone();
    let io = Arc::new(GatedFailure {
        entered: Notify::new(),
        release: Notify::new(),
    });
    let operation = tokio::spawn({
        let clients = hub.clients.clone();
        let io = io.clone();
        async move {
            relay_send::send(
                &target,
                &clients,
                Message::Binary(Bytes::new()),
                io.as_ref(),
            )
            .await
        }
    });
    poll_io(io.entered.notified()).await;
    let mut replacement = ControlledPeer::open(&tls, &hub).await;
    let held = hub.clients.lock().await;
    replacement
        .ws
        .send(request([0x42; 6], [0x42; 16]))
        .await
        .unwrap();
    until(|| {
        replacement
            .deadline
            .admission_started
            .load(Ordering::Acquire)
    })
    .await;
    // Tokio's registry mutex grants the queued replacement admission before
    // the now-failing send can inspect the registry. Its target capture is old.
    io.release.notify_one();
    until(|| closed.load(Ordering::Acquire)).await;
    drop(held);
    assert!(matches!(poll_io(replacement.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    assert!(poll_io(operation).await.unwrap().is_err());
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    let map = hub.clients.lock().await;
    let current = map.get(&[0x42; 6]).unwrap();
    assert!(Arc::ptr_eq(&current.sink, &replacement.sink));
    assert!(!current.closed.load(Ordering::Acquire));
    drop(map);
    replacement.ws.send(heartbeat_wire(25)).await.unwrap();
    assert!(matches!(poll_io(replacement.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0..4] == [0x0B, 0, 0, 25]));
    hub.hub.stop().await;
}

#[tokio::test]
async fn heartbeat_send_failure_interrupts_source_relay_and_releases_admission() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut source = ControlledPeer::open(&tls, &hub).await;
    source.connect(0x42).await;
    let mut target = ControlledPeer::open(&tls, &hub).await;
    target.connect(0x43).await;
    let held = target.sink.clone().lock_owned().await;
    source.ws.send(unicast_wire([0x43; 6])).await.unwrap();
    until(|| source.deadline.received.load(Ordering::Acquire) == 2).await;
    hub.clients
        .lock()
        .await
        .get(&[0x42; 6])
        .unwrap()
        .last_activity
        .store(0, Ordering::Release);
    let candidate = heartbeat::snapshot(&hub.clients, 100)
        .await
        .into_iter()
        .find(|(attempt, _)| attempt.vmac == [0x42; 6])
        .unwrap()
        .0;
    let io = GatedIo::new(100);
    io.fail.store(true, Ordering::Release);
    let work = tokio::spawn({
        let clients = hub.clients.clone();
        let io = io.clone();
        async move {
            heartbeat::send_request(
                &clients,
                &candidate,
                &std::sync::atomic::AtomicU16::new(10),
                io.as_ref(),
            )
            .await;
        }
    });
    poll_io(io.sent.notified()).await; // real TLS/WebSocket write reached peer
    io.release.notify_one(); // fail at existing completion boundary
    poll_io(work).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    assert!(!hub.clients.lock().await.contains_key(&[0x42; 6]));
    assert!(hub.clients.lock().await.contains_key(&[0x43; 6]));
    drop(held);
    hub.hub.stop().await;
}

#[tokio::test]
async fn target_retirement_cancels_inflight_send_before_io_completion() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = ControlledPeer::open(&tls, &hub).await;
    peer.connect(0x42).await;
    let target =
        HubRelaySink::capture([0x42; 6], hub.clients.lock().await.get(&[0x42; 6]).unwrap());
    let io = Arc::new(GatedFailure {
        entered: Notify::new(),
        release: Notify::new(),
    });
    let operation = tokio::spawn({
        let clients = hub.clients.clone();
        let io = io.clone();
        async move {
            relay_send::send(
                &target,
                &clients,
                Message::Binary(Bytes::new()),
                io.as_ref(),
            )
            .await
        }
    });
    poll_io(io.entered.notified()).await;
    let attempt = heartbeat::Attempt {
        vmac: [0x42; 6],
        sink: peer.sink.clone(),
        generation: 0,
    };
    assert!(
        heartbeat::retire(
            &hub.clients,
            &attempt,
            heartbeat::Retirement::SendFailed,
            &super::heartbeat_test_support::ClockIo(AtomicU64::new(106))
        )
        .await
    );
    // Never release the I/O gate: retirement must drop this pending send,
    // releasing its sink guard so the owner can complete Close and Admission.
    assert!(poll_io(operation).await.unwrap().is_ok());
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
    assert!(hub.clients.lock().await.is_empty());
    hub.hub.stop().await;
}

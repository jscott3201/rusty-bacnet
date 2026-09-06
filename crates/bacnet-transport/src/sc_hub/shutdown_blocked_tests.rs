use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::frame;
use super::*;
use std::time::Duration;

// Real TLS/WebSocket handler attached to the production supervisor's task set.
// Exposing its sink before Connect makes blocked Accept admission deterministic.
pub(super) struct ControlledPeer {
    pub ws: ClientWs,
    pub sink: Arc<Mutex<WsSink>>,
    pub deadline: Arc<deadlines::ConnectDeadline>,
}

impl ControlledPeer {
    pub async fn open(tls: &TestTls, hub: &CountedHub) -> Self {
        let (server, ws, address, accepted) = tls.pair().await;
        let (write, read) = server.split();
        let sink = Arc::new(Mutex::new(write));
        let deadline = Arc::new(deadlines::ConnectDeadline::new(
            accepted + Duration::from_secs(5),
        ));
        let admission = connection::Admission::new(hub.active.clone(), Duration::from_secs(10));
        let operation = deadlines::serve(
            address,
            ([0x10; 6], [0x10; 16]),
            read,
            sink.clone(),
            hub.clients.clone(),
            deadline.clone(),
            || {},
        );
        assert!(hub.hub.tasks.spawner().spawn(async move {
            let _admission = admission;
            operation.await;
        }));
        Self { ws, sink, deadline }
    }

    pub async fn connect(&mut self, id: u8) {
        self.ws.send(request([id; 6], [id; 16])).await.unwrap();
        assert!(matches!(poll_io(self.ws.next()).await,
            Some(Ok(Message::Binary(data))) if data[0] == 7));
    }
}

async fn stopped(hub: &mut CountedHub) {
    poll_io(hub.hub.stop()).await;
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    assert!(hub.clients.lock().await.is_empty());
    assert_eq!(hub.hub.tasks.len(), 0);
}

#[tokio::test]
async fn stop_cancels_committed_handler_waiting_to_send_accept() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let ControlledPeer {
        mut ws,
        sink,
        deadline,
    } = ControlledPeer::open(&tls, &hub).await;
    let socket = Arc::downgrade(&sink);
    let held = sink.lock_owned().await;
    ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    until(|| deadline.is_committed()).await;
    // The normal Connect deadline is retired at commit, even while Accept waits.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    stopped(&mut hub).await;
    assert_eq!(
        socket.strong_count(),
        1,
        "cancelled Accept retained its sink"
    );
    drop(held);
    assert!(socket.upgrade().is_none());
    assert!(matches!(poll_io(ws.next()).await, None | Some(Err(_))));
}

#[tokio::test]
async fn stop_cancels_heartbeat_ack_and_unicast_sink_waits() {
    for unicast in [false, true] {
        let tls = TestTls::new();
        let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
        let mut sender = ControlledPeer::open(&tls, &hub).await;
        sender.connect(0x42).await;
        let mut recipient = ControlledPeer::open(&tls, &hub).await;
        recipient.connect(0x43).await;
        let sink = if unicast {
            recipient.sink.clone()
        } else {
            sender.sink.clone()
        };
        let socket = Arc::downgrade(&sink);
        let held = sink.lock_owned().await;
        let mut message = frame(
            if unicast {
                ScFunction::EncapsulatedNpdu
            } else {
                ScFunction::HeartbeatRequest
            },
            23,
        );
        if unicast {
            message.destination_vmac = Some([0x43; 6]);
            message.payload = Bytes::from_static(&[1, 0]);
        }
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &message);
        sender
            .ws
            .send(Message::Binary(wire.freeze()))
            .await
            .unwrap();
        until(|| sender.deadline.received.load(Ordering::Acquire) == 2).await;
        // The current-thread handler has run to its sink wait after observing
        // the message. The test keeps that sink locked throughout shutdown.
        stopped(&mut hub).await;
        drop(sender.sink);
        drop(recipient.sink);
        assert_eq!(
            socket.strong_count(),
            1,
            "blocked worker still retains sink"
        );
        drop(held);
        assert!(socket.upgrade().is_none());
    }
}

#[tokio::test]
async fn stop_owns_replaced_connection_cleanup_while_old_sink_is_locked() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut old = ControlledPeer::open(&tls, &hub).await;
    old.connect(0x42).await;
    let socket = Arc::downgrade(&old.sink);
    let held = old.sink.lock_owned().await;
    let mut newcomer = ControlledPeer::open(&tls, &hub).await;
    newcomer
        .ws
        .send(request([0x43; 6], [0x42; 16]))
        .await
        .unwrap();
    assert!(matches!(poll_io(newcomer.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    assert!(!hub.clients.lock().await.contains_key(&[0x42; 6]));
    // The retired connection owner retains its old sink during bounded cleanup.
    assert!(socket.strong_count() > 1);
    stopped(&mut hub).await;
    assert_eq!(
        socket.strong_count(),
        1,
        "retired connection cleanup survived stop"
    );
    drop(held);
    assert!(socket.upgrade().is_none());
    assert!(matches!(poll_io(old.ws.next()).await, None | Some(Err(_))));
}

#[tokio::test]
async fn stop_cancels_live_heartbeat_sweep_waiting_on_sink() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = ControlledPeer::open(&tls, &hub).await;
    peer.connect(0x42).await;
    hub.clients
        .lock()
        .await
        .get(&[0x42; 6])
        .unwrap()
        .last_activity
        .store(0, Ordering::Release);
    let socket = Arc::downgrade(&peer.sink);
    let held = peer.sink.lock_owned().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    poll_io(async {
        loop {
            if hub
                .clients
                .lock()
                .await
                .get(&[0x42; 6])
                .unwrap()
                .heartbeat
                .pending
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    stopped(&mut hub).await;
    assert_eq!(socket.strong_count(), 1, "heartbeat worker survived stop");
    drop(held);
    assert!(socket.upgrade().is_none());
}

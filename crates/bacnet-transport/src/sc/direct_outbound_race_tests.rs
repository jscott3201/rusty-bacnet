//! Real TLS publication ordering: local uniqueness and crossed retirement.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;

type Peer =
    tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>;

#[derive(Default)]
struct Gate {
    accept: Notify,
    send: Notify,
}
struct OrderedSocket {
    ws: TlsWebSocket,
    id: usize,
    gate: Option<Arc<Gate>>,
    first_recv: AtomicBool,
    first_npdu: AtomicBool,
    events: mpsc::Sender<(usize, &'static str)>,
}
impl WebSocketPort for OrderedSocket {
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        let bytes = self.ws.recv().await?;
        if self.first_recv.swap(false, Ordering::SeqCst) {
            assert_eq!(
                decode_sc_message(&bytes).unwrap().function,
                ScFunction::ConnectAccept
            );
            if let Some(gate) = &self.gate {
                self.events.send((self.id, "accept")).await.unwrap();
                gate.accept.notified().await;
            }
        }
        Ok(bytes)
    }
    async fn send(&self, bytes: &[u8]) -> Result<(), Error> {
        if decode_sc_message(bytes).unwrap().function == ScFunction::EncapsulatedNpdu
            && self.first_npdu.swap(false, Ordering::SeqCst)
            && let Some(gate) = &self.gate
        {
            self.events.send((self.id, "send")).await.unwrap();
            gate.send.notified().await;
        }
        self.ws.send(bytes).await
    }
}
struct Dials {
    direct: Arc<DirectShared<OrderedSocket>>,
    conn: Arc<Mutex<ScConnection>>,
    gates: [Arc<Gate>; 2],
    events: mpsc::Receiver<(usize, &'static str)>,
    uri: String,
}
impl Dials {
    async fn new(ca: &TestCa, uri: String) -> Self {
        let direct = Arc::new(DirectShared::new(Arc::new(DirectMembership::default())));
        let gates = [Arc::new(Gate::default()), Arc::new(Gate::default())];
        let (events, recv) = mpsc::channel(8);
        let tls = ca.node_config(vec!["client".into()]);
        let dial_gates = gates.clone();
        let next = AtomicUsize::new(0);
        *direct.dialer.lock().await = Some(DirectDialer::Custom(Arc::new(move |uri| {
            let id = next.fetch_add(1, Ordering::SeqCst);
            let gate = dial_gates.get(id).cloned();
            let events = events.clone();
            let tls = tls.clone();
            Box::pin(async move {
                Ok(OrderedSocket {
                    ws: TlsWebSocket::connect_direct(&uri, tls).await?,
                    id,
                    gate,
                    first_recv: AtomicBool::new(true),
                    first_npdu: AtomicBool::new(true),
                    events,
                })
            })
        })));
        let mut conn = ScConnection::new(LOCAL, [1; 16]);
        conn.state = ScConnectionState::Connected;
        Self {
            direct,
            conn: Arc::new(Mutex::new(conn)),
            gates,
            events: recv,
            uri,
        }
    }
    fn start(&self) -> tokio::task::JoinHandle<Result<(), ()>> {
        let direct = self.direct.clone();
        let conn = self.conn.clone();
        let uri = self.uri.clone();
        tokio::spawn(async move {
            direct
                .try_direct_uris(
                    &[uri],
                    REMOTE,
                    NPDU,
                    &[],
                    &conn,
                    DirectTiming {
                        connect_timeout_ms: 3000,
                        heartbeat_ms: (30_000, 60_000),
                    },
                )
                .await
                .map_err(|_| ())
        })
    }
    async fn event(&mut self, id: usize, kind: &'static str) {
        assert_eq!(
            tokio::time::timeout(WAIT, self.events.recv())
                .await
                .unwrap()
                .unwrap(),
            (id, kind)
        );
        assert!(self.direct.membership.current_generations().len() <= 1);
    }
    async fn publish(&mut self, id: usize) -> PooledDirect {
        self.gates[id].accept.notify_one();
        self.event(id, "send").await;
        let pooled = self.direct.pooled_get(&REMOTE, Instant::now()).unwrap();
        assert_eq!(
            self.direct.membership.current_generations(),
            [pooled.member.generation]
        );
        pooled
    }
    async fn released(&self) {
        until(|| self.direct.physical.available_permits() == DIRECT_POOL_MAX_ENTRIES * 2).await;
        assert!(self.direct.membership.current_generations().is_empty());
        assert!(self.direct.pooled_get(&REMOTE, Instant::now()).is_none());
        assert_eq!(
            self.direct.pending_dials.available_permits(),
            DIRECT_POOL_MAX_ENTRIES
        );
    }
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn joined(task: tokio::task::JoinHandle<Result<(), ()>>) -> Result<(), ()> {
    tokio::time::timeout(WAIT, task).await.unwrap().unwrap()
}
#[allow(clippy::result_large_err)]
async fn accept_peer(listener: &tokio::net::TcpListener, ca: &TestCa) -> Peer {
    let (tcp, _) = listener.accept().await.unwrap();
    let tls = ca.acceptor().accept(tcp).await.unwrap();
    let mut ws = tokio_tungstenite::accept_hdr_async(
        tls,
        |_: &tokio_tungstenite::tungstenite::handshake::server::Request,
         mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            response.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                crate::sc_frame::BACNET_SC_DIRECT_SUBPROTOCOL
                    .parse()
                    .unwrap(),
            );
            Ok(response)
        },
    )
    .await
    .unwrap();
    let request = ws.next().await.unwrap().unwrap().into_data();
    let request = decode_sc_message(&request).unwrap();
    assert_eq!(request.function, ScFunction::ConnectRequest);
    let mut payload = Vec::from(REMOTE);
    payload.extend_from_slice(&[2; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());
    let mut bytes = BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: request.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: bytes::Bytes::from(payload),
        },
    );
    ws.send(Message::Binary(bytes.to_vec().into()))
        .await
        .unwrap();
    ws
}
async fn peer_npdu(peer: &mut Peer) {
    let wire = tokio::time::timeout(WAIT, peer.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let message = decode_sc_message(&wire.into_data()).unwrap();
    assert_eq!(message.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(message.payload.as_ref(), NPDU);
}

#[tokio::test]
async fn concurrent_outbound_held_tls_peers_publish_exactly_one_local_generation() {
    let ca = TestCa::generate();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!(
        "wss://localhost:{}/.bacnet/sc",
        listener.local_addr().unwrap().port()
    );
    let mut d = Dials::new(&ca, uri).await;
    let a = d.start();
    let peer_a = accept_peer(&listener, &ca).await;
    d.event(0, "accept").await;
    let b = d.start();
    let mut peer_b = accept_peer(&listener, &ca).await;
    d.event(1, "accept").await;
    assert!(d.direct.membership.current_generations().is_empty());
    // Both real peer sockets stay owned here; neither peer independently
    // replaces/closes the other while local publication ordering is exercised.
    let old = d.publish(0).await;
    d.gates[0].send.notify_one();
    joined(a).await.unwrap();
    let current = d.publish(1).await;
    assert!(!old.member.is_current());
    assert!(old.send(NPDU).await.is_err());
    old.member.retire();
    assert!(current.member.is_current());
    assert_eq!(d.direct.pool.lock().unwrap().len(), 1);
    d.gates[1].send.notify_one();
    joined(b).await.unwrap();
    peer_npdu(&mut peer_b).await;
    joined(d.start()).await.unwrap();
    peer_npdu(&mut peer_b).await;
    until(|| d.direct.physical.available_permits() == DIRECT_POOL_MAX_ENTRIES * 2 - 1).await;
    assert!(current.member.is_current());
    drop((peer_a, peer_b)); // Only this explicit peer closure retires the winner.
    d.released().await;
    d.direct.disable();
}

#[tokio::test]
async fn concurrent_outbound_crossed_accept_order_releases_both_and_redials() {
    let ca = TestCa::generate();
    let remote = Arc::new(DirectMembership::default());
    let config = DirectAcceptConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        REMOTE,
        [2; 16],
        ca.node_config(vec!["localhost".into()]),
    )
    .with_max_established_peers(1);
    let (mut listener, mut rx) = DirectListener::start_shared(config, remote.clone())
        .await
        .unwrap();
    let mut d = Dials::new(&ca, url(&listener)).await;
    let a = d.start();
    // Accept A is received and held locally. Receiving bytes does not prove
    // the remote send future has returned; wait for its subsequent publication.
    d.event(0, "accept").await;
    until(|| remote.current_generations().len() == 1).await;
    let remote_a = remote.current_generations();
    assert_eq!(remote_a.len(), 1);
    assert_eq!(listener.active_connections(), 1);
    let b = d.start();
    d.event(1, "accept").await; // Accept B received and held locally.
    until(|| {
        let current = remote.current_generations();
        current.len() == 1 && current != remote_a
    })
    .await; // Remote has committed B after A and retired A.
    let remote_b = remote.current_generations();
    assert_eq!(remote_b.len(), 1);
    assert_ne!(remote_a, remote_b);
    assert!(listener.active_connections() <= 2);
    assert!(d.direct.membership.current_generations().is_empty());
    let local_b = d.publish(1).await; // Local deliberately receives B first.
    assert_eq!(remote.current_generations(), remote_b);
    d.gates[1].send.notify_one();
    joined(b).await.unwrap();
    receive(&mut rx, LOCAL).await; // B is available before crossed replacement.
    let local_a = d.publish(0).await; // Then local receives delayed A, retiring B.
    assert!(!local_b.member.is_current());
    assert!(local_b.send(NPDU).await.is_err());
    assert!(remote.current_generations().len() <= 1);
    assert!(!remote.current_generations().contains(&remote_a[0]));
    local_b.member.retire(); // Late B cleanup must leave current local A intact.
    assert!(local_a.member.is_current());
    d.gates[0].send.notify_one();
    // Local write success is not evidence that the remote admitted this NPDU.
    let _send_result = joined(a).await;
    d.released().await;
    until(|| listener.active_connections() == 0).await;
    assert!(remote.current_generations().is_empty());
    assert_eq!(remote.counts(), (0, 0));
    assert!(
        rx.try_recv().is_err(),
        "retired remote A cannot admit new NPDU"
    );
    // Honor the real per-URI retry deadline if the crossed write failed. This
    // timer represents retry policy; all race ordering above uses explicit gates.
    let deadline = d
        .direct
        .backoff
        .lock()
        .await
        .entries
        .get(&d.uri)
        .map(|e| e.not_before);
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
    }
    joined(d.start()).await.unwrap();
    receive(&mut rx, LOCAL).await;
    let fresh = d.direct.pooled_get(&REMOTE, Instant::now()).unwrap();
    assert_ne!(fresh.member.generation, local_a.member.generation);
    local_a.member.retire();
    local_b.member.retire();
    assert_eq!(
        d.direct.membership.current_generations(),
        [fresh.member.generation]
    );
    assert_eq!(remote.current_generations().len(), 1);
    joined(d.start()).await.unwrap();
    receive(&mut rx, LOCAL).await;
    d.direct.disable();
    listener.stop().await;
    d.released().await;
}

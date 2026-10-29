//! Real TLS cross-role membership, outbound publication and teardown.
use super::*;
use crate::port::ReceivedNpdu;
use crate::sc_frame::{ScBvlcResult, decode_sc_message};
use crate::sc_tls::direct_accept::direct_accept_tests::TestCa;
use crate::sc_tls::{DirectAcceptConfig, DirectListener, TlsWebSocket};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use tokio::sync::mpsc;

const LOCAL: Vmac = [0x11; 6];
const REMOTE: Vmac = [0x22; 6];
const THIRD: Vmac = [0x33; 6];
const NPDU: &[u8] = &[1, 0, 0x30];
const WAIT: Duration = Duration::from_secs(3);

struct Fixture {
    ca: TestCa,
    direct: DirectShared<TlsWebSocket>,
    conn: Arc<Mutex<ScConnection>>,
    local: DirectListener,
    local_rx: mpsc::Receiver<ReceivedNpdu>,
    remote: DirectListener,
    remote_rx: mpsc::Receiver<ReceivedNpdu>,
    uri: String,
}
fn url(listener: &DirectListener) -> String {
    format!(
        "wss://localhost:{}/.bacnet/sc",
        listener.local_addr().port()
    )
}
impl Fixture {
    async fn new(limit: usize) -> Self {
        let ca = TestCa::generate();
        let membership = Arc::new(DirectMembership::default());
        let tls = ca.node_config(vec!["localhost".into()]);
        let config = |vmac, uuid| {
            DirectAcceptConfig::new("127.0.0.1:0".parse().unwrap(), vmac, uuid, tls.clone())
        };
        let (local, local_rx) = DirectListener::start_shared(
            config(LOCAL, [1; 16]).with_max_established_peers(limit),
            membership.clone(),
        )
        .await
        .unwrap();
        let (remote, remote_rx) = DirectListener::start(config(REMOTE, [2; 16]))
            .await
            .unwrap();
        let uri = url(&remote);
        let direct = DirectShared::new(membership);
        let client_tls = ca.node_config(vec!["node".into()]);
        *direct.dialer.lock().await = Some(DirectDialer::Custom(Arc::new(move |uri| {
            let tls = client_tls.clone();
            Box::pin(async move { TlsWebSocket::connect_direct(&uri, tls).await })
        })));
        let mut conn = ScConnection::new(LOCAL, [1; 16]);
        conn.state = ScConnectionState::Connected;
        Self {
            ca,
            direct,
            conn: Arc::new(Mutex::new(conn)),
            local,
            local_rx,
            remote,
            remote_rx,
            uri,
        }
    }
    async fn send(&self) -> Result<(), ()> {
        self.direct
            .try_direct_uris(
                std::slice::from_ref(&self.uri),
                REMOTE,
                NPDU,
                &[],
                &self.conn,
                DirectTiming {
                    connect_timeout_ms: 1000,
                    heartbeat_ms: (30_000, 60_000),
                },
            )
            .await
            .map_err(|_| ())
    }
    async fn stop(&mut self) {
        self.direct.disable();
        self.local.stop().await;
        self.remote.stop().await;
        tokio::time::timeout(WAIT, async {
            while self.direct.physical.available_permits() != DIRECT_POOL_MAX_ENTRIES * 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
async fn claim(
    listener: &DirectListener,
    ca: &TestCa,
    vmac: Vmac,
    uuid: [u8; 16],
) -> (TlsWebSocket, ScMessage) {
    let ws = TlsWebSocket::connect_direct(&url(listener), ca.node_config(vec!["client".into()]))
        .await
        .unwrap();
    let mut connection = ScConnection::new(vmac, uuid);
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &connection.build_connect_request());
    ws.send(&bytes).await.unwrap();
    let reply = tokio::time::timeout(WAIT, ws.recv())
        .await
        .unwrap()
        .unwrap();
    (ws, decode_sc_message(&reply).unwrap())
}
async fn receive(rx: &mut mpsc::Receiver<ReceivedNpdu>, vmac: Vmac) {
    let received = tokio::time::timeout(WAIT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.source_mac.as_ref(), vmac);
    assert_eq!(received.npdu.as_ref(), NPDU);
}
async fn send_accepted(ws: &TlsWebSocket, vmac: Vmac) {
    let mut conn = ScConnection::new(vmac, [3; 16]);
    let mut bytes = BytesMut::new();
    encode_sc_message(
        &mut bytes,
        &conn.build_direct_encapsulated_npdu(NPDU, &[]).unwrap(),
    );
    ws.send(&bytes).await.unwrap();
}
fn nak(msg: &ScMessage, class: ErrorClass, code: ErrorCode) {
    assert_eq!(msg.message_id, 1);
    assert_eq!(msg.originating_vmac, None);
    assert_eq!(msg.destination_vmac, None);
    assert!(msg.dest_options.is_empty() && msg.data_options.is_empty());
    let mut expected = vec![ScFunction::ConnectRequest.to_raw(), 1, 0];
    expected.extend_from_slice(&class.to_raw().to_be_bytes());
    expected.extend_from_slice(&code.to_raw().to_be_bytes());
    assert_eq!(msg.payload.as_ref(), expected);
    assert!(matches!(
        decode_sc_bvlc_result(msg).unwrap(),
        ScBvlcResult::Nak { .. }
    ));
}

#[tokio::test]
async fn accepted_replaces_outbound_and_old_pool_cleanup_cannot_erase_successor() {
    let mut f = Fixture::new(1).await;
    f.send().await.unwrap();
    receive(&mut f.remote_rx, LOCAL).await;
    let old = f.direct.pooled_get(&REMOTE, Instant::now()).unwrap();
    let (incoming, accept) = claim(&f.local, &f.ca, REMOTE, [2; 16]).await;
    assert_eq!(accept.function, ScFunction::ConnectAccept);
    assert!(!old.member.is_current());
    assert!(old.send(NPDU).await.is_err());
    old.member.retire(); // Actual generation-specific worker/drop cleanup.
    f.direct.pool.lock().unwrap().prune(Instant::now());
    assert_eq!(f.direct.membership.counts(), (1, 0));
    send_accepted(&incoming, REMOTE).await;
    receive(&mut f.local_rx, REMOTE).await;
    f.direct.disable();
    // Disabling dial-out does not clear accepted ownership or intake.
    send_accepted(&incoming, REMOTE).await;
    receive(&mut f.local_rx, REMOTE).await;
    f.stop().await;
}

#[tokio::test]
async fn accepted_full_and_third_peer_conflict_preserve_outbound() {
    let mut f = Fixture::new(1).await;
    f.send().await.unwrap();
    receive(&mut f.remote_rx, LOCAL).await;
    let (third, _) = claim(&f.local, &f.ca, THIRD, [3; 16]).await;
    let (_incoming, refusal) = claim(&f.local, &f.ca, REMOTE, [2; 16]).await;
    nak(&refusal, ErrorClass::RESOURCES, ErrorCode::OTHER);
    let (_incoming, refusal) = claim(&f.local, &f.ca, REMOTE, [3; 16]).await;
    nak(
        &refusal,
        ErrorClass::COMMUNICATION,
        ErrorCode::NODE_DUPLICATE_VMAC,
    );
    let (_incoming, refusal) = claim(&f.local, &f.ca, REMOTE, [4; 16]).await;
    nak(
        &refusal,
        ErrorClass::COMMUNICATION,
        ErrorCode::NODE_DUPLICATE_VMAC,
    );
    f.send().await.unwrap();
    receive(&mut f.remote_rx, LOCAL).await;
    send_accepted(&third, THIRD).await;
    receive(&mut f.local_rx, THIRD).await;
    f.stop().await;
}

#[tokio::test]
async fn accepted_publication_while_outbound_dial_waits_then_outbound_replaces() {
    let mut f = Fixture::new(1).await;
    let (entered, mut entered_rx) = mpsc::channel(1);
    let release = Arc::new(tokio::sync::Notify::new());
    let tls = f.ca.node_config(vec!["node".into()]);
    let gate = release.clone();
    *f.direct.dialer.lock().await = Some(DirectDialer::Custom(Arc::new(move |uri| {
        let tls = tls.clone();
        let entered = entered.clone();
        let gate = gate.clone();
        Box::pin(async move {
            let ws = TlsWebSocket::connect_direct(&uri, tls).await?;
            entered.send(()).await.unwrap();
            gate.notified().await;
            Ok(ws)
        })
    })));
    let incoming = {
        let send = f.send();
        tokio::pin!(send);
        tokio::select! { _ = entered_rx.recv() => {}, _ = &mut send => panic!("dial must wait") }
        let (incoming, accept) = claim(&f.local, &f.ca, REMOTE, [2; 16]).await;
        assert_eq!(accept.function, ScFunction::ConnectAccept);
        release.notify_one();
        send.await.unwrap();
        incoming
    };
    receive(&mut f.remote_rx, LOCAL).await;
    let disconnect = tokio::time::timeout(WAIT, incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&disconnect).unwrap().function,
        ScFunction::DisconnectRequest
    );
    assert_eq!(f.direct.membership.counts(), (0, 0));
    assert!(f.direct.pooled_get(&REMOTE, Instant::now()).is_some());
    f.stop().await;
}

#[tokio::test]
async fn expiry_eviction_disable_release_membership_and_close_workers() {
    let mut f = Fixture::new(1).await;
    f.send().await.unwrap();
    receive(&mut f.remote_rx, LOCAL).await;
    let old = f.direct.pooled_get(&REMOTE, Instant::now()).unwrap();
    assert!(
        f.direct
            .pooled_get(&REMOTE, Instant::now() + DIRECT_POOL_IDLE_TTL + WAIT)
            .is_none()
    );
    assert!(!old.member.is_current());
    f.send().await.unwrap();
    receive(&mut f.remote_rx, LOCAL).await;
    let current = f.direct.pooled_get(&REMOTE, Instant::now()).unwrap();
    old.member.retire(); // Actual generation-specific worker/drop cleanup.
    f.direct.pool.lock().unwrap().prune(Instant::now());
    assert!(current.member.is_current());
    f.direct.pool.lock().unwrap().evict_oldest();
    assert!(!current.member.is_current());
    f.stop().await;
}

#[tokio::test]
async fn idle_outbound_remote_eof_releases_identity_and_physical_slot() {
    let mut f = Fixture::new(1).await;
    f.send().await.unwrap();
    receive(&mut f.remote_rx, LOCAL).await;
    let old = f.direct.pooled_get(&REMOTE, Instant::now()).unwrap();
    f.remote.stop().await;
    // No pooled_get, expiry, outgoing NPDU or local disable drives cleanup.
    tokio::time::timeout(WAIT, async {
        while old.member.is_current()
            || f.direct.physical.available_permits() != DIRECT_POOL_MAX_ENTRIES * 2
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("remote EOF must autonomously retire idle outbound membership");
    let (replacement, accept) = claim(&f.local, &f.ca, REMOTE, [4; 16]).await;
    assert_eq!(accept.function, ScFunction::ConnectAccept);
    old.member.retire(); // Late old owner cleanup still cannot erase the new UUID.
    send_accepted(&replacement, REMOTE).await;
    receive(&mut f.local_rx, REMOTE).await;
    f.stop().await;
}

#[path = "direct_remote_control_tests.rs"]
mod direct_remote_control_tests;

#[path = "direct_outbound_race_tests.rs"]
mod direct_outbound_race_tests;

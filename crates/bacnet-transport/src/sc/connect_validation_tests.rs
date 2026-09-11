use super::*;
use crate::sc::LoopbackWebSocket;
use crate::sc_frame::connect_test_support::{
    invalid_connects, valid_connect, valid_connect_with_options,
};
use tokio::sync::mpsc;
use tokio::time::timeout;

fn sentinel_connection() -> ScConnection {
    let mut conn = ScConnection::new([0x02; 6], [0x12; 16]);
    conn.hub_vmac = Some([0x44; 6]);
    conn.hub_device_uuid = Some([0x55; 16]);
    conn.hub_max_bvlc_length = 3333;
    conn.hub_max_apdu_length = 2222;
    conn
}

fn assert_unchanged(conn: &ScConnection, before: &ScConnection) {
    assert_eq!(conn.state, before.state);
    assert_eq!(
        conn.pending_connect_message_id,
        before.pending_connect_message_id
    );
    assert_eq!(conn.hub_vmac, before.hub_vmac);
    assert_eq!(conn.hub_device_uuid, before.hub_device_uuid);
    assert_eq!(conn.hub_max_bvlc_length, before.hub_max_bvlc_length);
    assert_eq!(conn.hub_max_apdu_length, before.hub_max_apdu_length);
    assert_eq!(conn.local_vmac, before.local_vmac);
    assert_eq!(conn.device_uuid, before.device_uuid);
    assert_eq!(conn.max_bvlc_length, before.max_bvlc_length);
    assert_eq!(conn.max_apdu_length, before.max_apdu_length);
    assert_eq!(conn.next_message_id, before.next_message_id);
    assert_eq!(conn.disconnect_ack_to_send, before.disconnect_ack_to_send);
    assert_eq!(conn.connect_retry_allowed, before.connect_retry_allowed);
}

struct ObservedWebSocket {
    inner: LoopbackWebSocket,
    receiving: mpsc::UnboundedSender<()>,
}

impl WebSocketPort for ObservedWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.inner.send(data).await
    }
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.receiving.send(()).unwrap();
        self.inner.recv().await
    }
}

async fn next_receive(receiving: &mut mpsc::UnboundedReceiver<()>) {
    timeout(Duration::from_secs(2), receiving.recv())
        .await
        .unwrap()
        .expect("handshake must wait for a later Accept after the invalid frame");
}

#[test]
fn connect_accept_reserved_identity_is_transactional() {
    let mut conn = sentinel_connection();
    let id = conn.build_connect_request().message_id;
    let before = conn.clone();
    let mut wire = valid_connect(7, [0; 6]);
    wire[2..4].copy_from_slice(&id.to_be_bytes());
    assert!(!conn.handle_connect_accept(&decode_sc_message(&wire).unwrap()));
    assert_unchanged(&conn, &before);
}

#[test]
fn connect_accept_zero_uuid_is_transactional_in_every_state() {
    // Independent AB.2.11 vector: function/flags/ID, VMAC, nil UUID, limits.
    let wire = [
        7, 0, 0, 1, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0x20, 0, 0x10, 0,
    ];
    let accept = decode_sc_message(&wire).unwrap();
    for state in [
        ScConnectionState::Disconnected,
        ScConnectionState::Connecting,
        ScConnectionState::Connected,
        ScConnectionState::Disconnecting,
    ] {
        let mut conn = sentinel_connection();
        assert_eq!(conn.build_connect_request().message_id, 1);
        conn.state = state;
        conn.connect_retry_allowed = false;
        conn.disconnect_ack_to_send = Some(conn.build_heartbeat_ack(42));
        let before = conn.clone();
        assert!(!conn.handle_connect_accept(&accept));
        assert_unchanged(&conn, &before);
    }
}

#[test]
fn zero_uuid_range_nak_does_not_reseed_vmac() {
    let mut conn = sentinel_connection();
    let id = conn.build_connect_request().message_id;
    let before = conn.local_vmac;
    let mut wire = vec![0, 0, 0, 0, 6, 1, 0, 0, 7, 0, 80];
    wire[2..4].copy_from_slice(&id.to_be_bytes());
    let result =
        crate::sc_frame::decode_sc_bvlc_result(&decode_sc_message(&wire).unwrap()).unwrap();
    assert!(!conn.handle_connect_result(id, &result).unwrap());
    assert_eq!(conn.local_vmac, before);
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[tokio::test]
async fn connect_accept_reserved_identity_is_silently_discarded() {
    let (inner, peer) = LoopbackWebSocket::pair();
    let (receiving, mut received) = mpsc::unbounded_channel();
    let ws = ObservedWebSocket { inner, receiving };
    let conn = Arc::new(Mutex::new(sentinel_connection()));
    let task = tokio::spawn({
        let conn = conn.clone();
        async move { perform_handshake(&ws, &conn, None, 5000).await }
    });
    let request = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    next_receive(&mut received).await;
    let before = conn.lock().await.clone();
    let mut wire = valid_connect(7, [0; 6]);
    wire[2..4].copy_from_slice(&request.message_id.to_be_bytes());
    peer.send(&wire).await.unwrap();
    next_receive(&mut received).await;
    assert_unchanged(&*conn.lock().await, &before);
    assert!(
        timeout(Duration::from_millis(10), peer.recv())
            .await
            .is_err(),
        "invalid Accept elicited a response"
    );
    let mut valid = valid_connect(7, [0x22; 6]);
    valid[2..4].copy_from_slice(&request.message_id.to_be_bytes());
    peer.send(&valid).await.unwrap();
    task.await.unwrap().unwrap();
    let conn = conn.lock().await;
    assert_eq!(conn.state, ScConnectionState::Connected);
    assert_eq!(conn.pending_connect_message_id, None);
    assert_eq!(conn.hub_vmac, Some([0x22; 6]));
    assert_eq!(conn.hub_device_uuid, Some([0x33; 16]));
    assert_eq!(conn.hub_max_bvlc_length, 8192);
    assert_eq!(conn.hub_max_apdu_length, 4096);
}

#[test]
fn connect_accept_invalid_matrix_is_transactional() {
    for case in invalid_connects(7, [0x02; 6]) {
        let mut conn = sentinel_connection();
        let id = conn.build_connect_request().message_id;
        let before = conn.clone();
        let mut wire = case.wire;
        wire[2..4].copy_from_slice(&id.to_be_bytes());
        assert!(
            !conn.handle_connect_accept(&decode_sc_message(&wire).unwrap()),
            "{}",
            case.name
        );
        assert_unchanged(&conn, &before);
    }
}

#[tokio::test]
async fn connect_accept_invalid_matrix_waits_silently_for_valid_accept() {
    let (inner, peer) = LoopbackWebSocket::pair();
    let (receiving, mut received) = mpsc::unbounded_channel();
    let ws = ObservedWebSocket { inner, receiving };
    let conn = Arc::new(Mutex::new(sentinel_connection()));
    let (states, mut state) = watch::channel(ScConnectionState::Disconnected);
    let task = tokio::spawn({
        let conn = conn.clone();
        async move { perform_handshake(&ws, &conn, Some(&states), 5000).await }
    });
    let request = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    next_receive(&mut received).await;
    let before = conn.lock().await.clone();
    assert_eq!(*state.borrow_and_update(), ScConnectionState::Connecting);
    for case in invalid_connects(7, [0x02; 6]) {
        let mut wire = case.wire;
        wire[2..4].copy_from_slice(&request.message_id.to_be_bytes());
        peer.send(&wire).await.unwrap();
        next_receive(&mut received).await;
        assert_unchanged(&*conn.lock().await, &before);
        assert!(
            !state.has_changed().unwrap(),
            "{} published state",
            case.name
        );
        assert!(
            timeout(Duration::from_millis(10), peer.recv())
                .await
                .is_err(),
            "{} elicited a response",
            case.name
        );
    }
    let mut valid = valid_connect_with_options(7, [0x22; 6]);
    valid[2..4].copy_from_slice(&request.message_id.to_be_bytes());
    peer.send(&valid).await.unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(*state.borrow(), ScConnectionState::Connected);
    let conn = conn.lock().await;
    assert_eq!(conn.state, ScConnectionState::Connected);
    assert_eq!(conn.pending_connect_message_id, None);
    assert_eq!(conn.hub_vmac, Some([0x22; 6]));
    assert_eq!(conn.hub_device_uuid, Some([0x33; 16]));
    assert_eq!(
        (conn.hub_max_bvlc_length, conn.hub_max_apdu_length),
        (8192, 4096)
    );
}

#[tokio::test(start_paused = true)]
async fn connect_accept_invalid_flood_keeps_absolute_deadline() {
    let (inner, peer) = LoopbackWebSocket::pair();
    let (receiving, mut received) = mpsc::unbounded_channel();
    let ws = ObservedWebSocket { inner, receiving };
    let conn = Arc::new(Mutex::new(sentinel_connection()));
    let task = tokio::spawn({
        let conn = conn.clone();
        async move { perform_handshake(&ws, &conn, None, 1000).await }
    });
    let request = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    next_receive(&mut received).await;
    let started = tokio::time::Instant::now();
    let mut expected = conn.lock().await.clone();
    let mut invalid = valid_connect(7, [0; 6]);
    invalid[2..4].copy_from_slice(&request.message_id.to_be_bytes());
    for _ in 0..9 {
        tokio::time::advance(Duration::from_millis(100)).await;
        peer.send(&invalid).await.unwrap();
        next_receive(&mut received).await;
        assert_unchanged(&*conn.lock().await, &expected);
    }
    tokio::time::advance(Duration::from_millis(100)).await;
    assert!(
        matches!(task.await.unwrap(), Err(Error::Timeout(duration)) if duration == Duration::from_secs(1))
    );
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    expected.state = ScConnectionState::Disconnected;
    expected.pending_connect_message_id = None;
    assert_unchanged(&*conn.lock().await, &expected);
}

#[tokio::test(start_paused = true)]
async fn connect_accept_nil_only_and_flood_keep_absolute_deadline() {
    for frames in [1, 9] {
        let (inner, peer) = LoopbackWebSocket::pair();
        let (receiving, mut received) = mpsc::unbounded_channel();
        let ws = ObservedWebSocket { inner, receiving };
        let conn = Arc::new(Mutex::new(sentinel_connection()));
        let (states, mut state) = watch::channel(ScConnectionState::Disconnected);
        let task = tokio::spawn({
            let conn = conn.clone();
            async move { perform_handshake(&ws, &conn, Some(&states), 1000).await }
        });
        let request = peer.recv().await.unwrap();
        next_receive(&mut received).await;
        let started = tokio::time::Instant::now();
        let mut expected = conn.lock().await.clone();
        assert_eq!(*state.borrow_and_update(), ScConnectionState::Connecting);
        let mut nil = valid_connect(7, [0x22; 6]);
        nil[2..4].copy_from_slice(&request[2..4]);
        nil[10..26].fill(0);
        for _ in 0..frames {
            tokio::time::advance(Duration::from_millis(100)).await;
            peer.send(&nil).await.unwrap();
            next_receive(&mut received).await;
            assert_unchanged(&*conn.lock().await, &expected);
            assert!(!state.has_changed().unwrap());
        }
        tokio::time::advance(Duration::from_millis(1000 - frames * 100)).await;
        assert!(
            matches!(task.await.unwrap(), Err(Error::Timeout(d)) if d == Duration::from_secs(1))
        );
        assert_eq!(started.elapsed(), Duration::from_secs(1));
        assert_eq!(*state.borrow(), ScConnectionState::Disconnected);
        expected.state = ScConnectionState::Disconnected;
        expected.pending_connect_message_id = None;
        assert_unchanged(&*conn.lock().await, &expected);
        // Once the owned socket drops, an empty channel proves no reply was queued.
        assert!(peer.recv().await.is_err());
    }
}

#[tokio::test]
async fn connect_accept_nil_wrong_id_is_discarded_but_valid_wrong_id_is_terminal() {
    let (inner, peer) = LoopbackWebSocket::pair();
    let (receiving, mut received) = mpsc::unbounded_channel();
    let ws = ObservedWebSocket { inner, receiving };
    let conn = Arc::new(Mutex::new(sentinel_connection()));
    let task = tokio::spawn({
        let conn = conn.clone();
        async move { perform_handshake(&ws, &conn, None, 5000).await }
    });
    let request = decode_sc_message(&peer.recv().await.unwrap()).unwrap();
    next_receive(&mut received).await;
    let mut expected = conn.lock().await.clone();
    let mut accept = valid_connect(7, [0x22; 6]);
    accept[2..4].copy_from_slice(&request.message_id.wrapping_add(1).to_be_bytes());
    accept[10..26].fill(0);
    peer.send(&accept).await.unwrap();
    next_receive(&mut received).await;
    assert_unchanged(&*conn.lock().await, &expected);
    assert!(timeout(Duration::from_millis(10), peer.recv())
        .await
        .is_err());
    accept[10..26].fill(0xff);
    peer.send(&accept).await.unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(
        ScConnectError::from_error(&error),
        Some(&ScConnectError::ConnectAcceptMismatch)
    );
    expected.state = ScConnectionState::Disconnected;
    expected.pending_connect_message_id = None;
    assert_unchanged(&*conn.lock().await, &expected);
    assert!(peer.recv().await.is_err());
}

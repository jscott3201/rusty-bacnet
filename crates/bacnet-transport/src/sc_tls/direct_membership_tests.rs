//! Real TLS peer identity and accepted capacity regressions (#851).
use super::*;
use crate::port_ownership::connect_loopback;
use crate::sc_tls::TlsWebSocket;

async fn connect_claim(
    listener: &DirectListener,
    ca: &TestCa,
    vmac: [u8; 6],
    uuid: [u8; 16],
) -> (TlsWebSocket, ScMessage) {
    let ws = tokio::time::timeout(
        Duration::from_secs(3),
        TlsWebSocket::connect_direct(
            &direct_url(&listener.local_addr()),
            ca.node_config(vec!["node".into()]),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let mut conn = ScConnection::new(vmac, uuid);
    let req = conn.build_connect_request();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &req);
    ws.send(&buf).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(3), ws.recv())
        .await
        .unwrap()
        .unwrap();
    (ws, decode_sc_message(&reply).unwrap())
}

fn assert_nak(msg: &ScMessage, class: ErrorClass, code: ErrorCode) {
    assert_eq!(msg.function, ScFunction::Result);
    assert_eq!(msg.message_id, 1);
    assert_eq!(msg.originating_vmac, None);
    assert_eq!(msg.destination_vmac, None);
    assert!(msg.dest_options.is_empty() && msg.data_options.is_empty());
    let mut expected = vec![ScFunction::ConnectRequest.to_raw(), 1, 0];
    expected.extend_from_slice(&class.to_raw().to_be_bytes());
    expected.extend_from_slice(&code.to_raw().to_be_bytes());
    assert_eq!(msg.payload.as_ref(), expected);
}

async fn assert_routable(
    ws: &TlsWebSocket,
    rx: &mut tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>,
    vmac: [u8; 6],
) {
    let mut conn = ScConnection::new(vmac, [7; 16]);
    let msg = conn.build_direct_encapsulated_npdu(NPDU, &[]).unwrap();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws.send(&buf).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.source_mac.as_ref(), vmac);
    assert_eq!(received.npdu.as_ref(), NPDU);
}

#[tokio::test]
async fn distinct_uuid_duplicate_vmac_rejected_incumbent_routable() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let (old, accepted) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    assert_eq!(accepted.function, ScFunction::ConnectAccept);
    let (_new, denied) = connect_claim(&listener, &ca, DIAL_VMAC, [8; 16]).await;
    assert_nak(
        &denied,
        ErrorClass::COMMUNICATION,
        ErrorCode::NODE_DUPLICATE_VMAC,
    );
    assert_routable(&old, &mut rx, DIAL_VMAC).await;
    listener.stop().await;
}

#[tokio::test]
async fn known_uuid_replaces_at_capacity_one() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c.with_max_established_peers(1)).await;
    let (old, accepted) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    assert_eq!(accepted.function, ScFunction::ConnectAccept);
    let (new, accepted) = connect_claim(&listener, &ca, [0x23; 6], DIAL_UUID).await;
    assert_eq!(accepted.function, ScFunction::ConnectAccept);
    let disconnect = tokio::time::timeout(Duration::from_secs(3), old.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decode_sc_message(&disconnect).unwrap().function,
        ScFunction::DisconnectRequest
    );
    assert_routable(&new, &mut rx, [0x23; 6]).await;
    listener.stop().await;
}

#[tokio::test]
async fn accepted_capacity_nak_preserves_incumbent() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c.with_max_established_peers(1)).await;
    let (old, _) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    let (_new, denied) = connect_claim(&listener, &ca, [0x23; 6], [8; 16]).await;
    assert_nak(&denied, ErrorClass::RESOURCES, ErrorCode::OTHER);
    assert_routable(&old, &mut rx, DIAL_VMAC).await;
    listener.stop().await;
    assert_eq!(listener.active_connections(), 0);
    assert_eq!(listener.membership.counts(), (0, 0));
}

#[tokio::test]
async fn third_peer_collision_preserves_both_incumbents() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c.with_max_established_peers(2)).await;
    let (a, _) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    let (b, _) = connect_claim(&listener, &ca, [0x23; 6], [8; 16]).await;
    let (_new, denied) = connect_claim(&listener, &ca, [0x23; 6], DIAL_UUID).await;
    assert_nak(
        &denied,
        ErrorClass::COMMUNICATION,
        ErrorCode::NODE_DUPLICATE_VMAC,
    );
    assert_routable(&a, &mut rx, DIAL_VMAC).await;
    assert_routable(&b, &mut rx, [0x23; 6]).await;
    assert_eq!(listener.membership.counts(), (2, 0));
    listener.stop().await;
}

#[path = "direct_reject_tests.rs"]
mod direct_reject_tests;

async fn wait_counts(listener: &DirectListener, active: usize, pending: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if listener.active_connections() == active
                && listener.pending.load(std::sync::atomic::Ordering::Relaxed) == pending
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn repeated_replacement_bounds_fence_old_npdu_and_preserve_queued_work() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c.with_max_established_peers(1)).await;
    let (mut old, _) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    for index in 0..8 {
        wait_counts(&listener, 1, 0).await;
        // Prove one complete NPDU is admitted before replacing its socket.
        let mut conn = ScConnection::new(DIAL_VMAC, DIAL_UUID);
        let msg = conn
            .build_direct_encapsulated_npdu(&[1, 0, index], &[])
            .unwrap();
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        old.send(&buf).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while rx.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (new, accepted) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
        assert_eq!(accepted.function, ScFunction::ConnectAccept);
        assert!(listener.active_connections() <= 2);
        assert_eq!(listener.membership.counts(), (1, 0));
        let disconnect = tokio::time::timeout(Duration::from_secs(3), old.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            decode_sc_message(&disconnect).unwrap().function,
            ScFunction::DisconnectRequest
        );
        let _ = old.send(&buf).await; // Either TCP failure or a stale frame; never fresh admission.
        wait_counts(&listener, 1, 0).await;
        assert_eq!(rx.try_recv().unwrap().npdu.as_ref(), &[1, 0, index]);
        assert!(rx.try_recv().is_err());
        assert_routable(&new, &mut rx, DIAL_VMAC).await;
        old = new;
    }
    listener.stop().await;
    wait_counts(&listener, 0, 0).await;
}

#[tokio::test]
async fn pending_saturation_timeout_and_stop_release_both_bounds() {
    use tokio::io::AsyncReadExt;
    let ca = TestCa::generate();
    let (mut listener, _rx) = start_listener(&ca, |c| {
        c.with_max_established_peers(1)
            .with_connect_timeout(Duration::from_millis(150))
    })
    .await;
    let first = connect_loopback(listener.local_addr()).await;
    wait_counts(&listener, 1, 1).await;
    let mut refused = connect_loopback(listener.local_addr()).await;
    let mut byte = [0];
    let result = tokio::time::timeout(Duration::from_secs(3), refused.read(&mut byte))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0) | Err(_)));
    wait_counts(&listener, 0, 0).await;
    drop(first);
    let (_peer, accepted) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    assert_eq!(accepted.function, ScFunction::ConnectAccept);
    listener.stop().await;
    wait_counts(&listener, 0, 0).await;
    assert_eq!(listener.membership.counts(), (0, 0));
}

#[tokio::test]
async fn positive_limit_overflow_rejected_before_bind_and_zero_normalizes() {
    let ca = TestCa::generate();
    let config = DirectAcceptConfig::new(
        loopback_addr(),
        LISTENER_VMAC,
        LISTENER_UUID,
        ca.node_config(vec!["localhost".into()]),
    );
    assert_eq!(
        config
            .clone()
            .with_max_established_peers(0)
            .max_established_peers,
        1
    );
    // An occupied address would report a bind failure if overflow validation ran late.
    let occupied = tokio::net::TcpListener::bind(loopback_addr())
        .await
        .unwrap();
    let mut config = config.with_max_established_peers(usize::MAX / 2 + 1);
    config.bind_addr = occupied.local_addr().unwrap();
    let error = DirectListener::start(config).await.err().unwrap();
    assert!(error.to_string().contains("overflows"));
}

#[path = "direct_reservation_tests.rs"]
mod direct_reservation_tests;

#[tokio::test]
async fn incumbent_plus_pending_reaches_physical_cap_and_connect_timeout_recovers() {
    let ca = TestCa::generate();
    // The pending peer holds its slot for the connect timeout, which must
    // outlast the refused dial to `localhost`. On Windows that dial takes at
    // least the 250 ms attempt delay: `::1` is tried first, and the IPv4-only
    // listener's refusal there is slow (#950).
    let (mut listener, mut rx) = start_listener(&ca, |c| {
        c.with_max_established_peers(1)
            .with_connect_timeout(Duration::from_secs(1))
    })
    .await;
    let (old, _) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    let pending = TlsWebSocket::connect_direct(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["node".into()]),
    )
    .await
    .unwrap();
    wait_counts(&listener, 2, 1).await;
    let refused = TlsWebSocket::connect_direct(
        &direct_url(&listener.local_addr()),
        ca.node_config(vec!["node".into()]),
    )
    .await;
    assert!(refused.is_err());
    assert!(
        tokio::time::timeout(Duration::from_secs(3), pending.recv())
            .await
            .unwrap()
            .is_err()
    );
    wait_counts(&listener, 1, 0).await;
    assert_routable(&old, &mut rx, DIAL_VMAC).await;
    listener.stop().await;
}

#[tokio::test]
async fn simultaneous_accepted_vmac_collision_has_one_winner_in_both_orders() {
    for reverse in [false, true] {
        let ca = TestCa::generate();
        let (mut listener, mut rx) = start_listener(&ca, |c| c.with_max_established_peers(2)).await;
        let a = TlsWebSocket::connect_direct(
            &direct_url(&listener.local_addr()),
            ca.node_config(vec!["a".into()]),
        )
        .await
        .unwrap();
        let b = TlsWebSocket::connect_direct(
            &direct_url(&listener.local_addr()),
            ca.node_config(vec!["b".into()]),
        )
        .await
        .unwrap();
        wait_counts(&listener, 2, 2).await;
        let barrier = tokio::sync::Barrier::new(2);
        let attempt = |ws: TlsWebSocket, uuid| {
            let barrier = &barrier;
            async move {
                let mut conn = ScConnection::new(DIAL_VMAC, uuid);
                let mut bytes = BytesMut::new();
                encode_sc_message(&mut bytes, &conn.build_connect_request());
                barrier.wait().await;
                ws.send(&bytes).await.unwrap();
                let reply = tokio::time::timeout(Duration::from_secs(3), ws.recv())
                    .await
                    .unwrap()
                    .unwrap();
                (ws, decode_sc_message(&reply).unwrap())
            }
        };
        let ((a, ar), (b, br)) = if reverse {
            tokio::join!(attempt(b, [8; 16]), attempt(a, DIAL_UUID))
        } else {
            tokio::join!(attempt(a, DIAL_UUID), attempt(b, [8; 16]))
        };
        let winner = if ar.function == ScFunction::ConnectAccept {
            assert_nak(
                &br,
                ErrorClass::COMMUNICATION,
                ErrorCode::NODE_DUPLICATE_VMAC,
            );
            a
        } else {
            assert_nak(
                &ar,
                ErrorClass::COMMUNICATION,
                ErrorCode::NODE_DUPLICATE_VMAC,
            );
            assert_eq!(br.function, ScFunction::ConnectAccept);
            b
        };
        assert_routable(&winner, &mut rx, DIAL_VMAC).await;
        assert_eq!(listener.membership.counts(), (1, 0));
        listener.stop().await;
    }
}

#[tokio::test]
async fn registered_listener_membership_survives_discovery_toggles() {
    let ca = TestCa::generate();
    let config = DirectAcceptConfig::new(
        loopback_addr(),
        LISTENER_VMAC,
        LISTENER_UUID,
        ca.node_config(vec!["localhost".into()]),
    );
    let (client, _hub) = crate::sc::LoopbackWebSocket::pair();
    let (transport, mut listener) = ScTransport::new(client, LISTENER_VMAC)
        .with_device_uuid(LISTENER_UUID)
        .with_direct_listener(config)
        .await
        .unwrap();
    let (old, _) = connect_claim(&listener, &ca, DIAL_VMAC, DIAL_UUID).await;
    let transport = transport
        .with_direct_discovery(true)
        .with_direct_discovery(false)
        .with_direct_discovery(true);
    let (_new, refusal) = connect_claim(&listener, &ca, DIAL_VMAC, [8; 16]).await;
    assert_nak(
        &refusal,
        ErrorClass::COMMUNICATION,
        ErrorCode::NODE_DUPLICATE_VMAC,
    );
    assert_eq!(listener.membership.counts(), (1, 0));
    drop(transport);
    // Discovery toggles preserve membership; dropping the registered transport
    // now seals its listener too. The retained handle joins that shared teardown.
    listener.stop().await;
    assert_eq!(listener.membership.counts(), (0, 0));
    assert_eq!(listener.active_connections(), 0);
    assert!(old.recv().await.is_err());
}

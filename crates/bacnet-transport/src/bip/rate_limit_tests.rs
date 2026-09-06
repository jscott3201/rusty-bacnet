use super::rate_limit::{
    is_covered_management_request, ManagementRateLimiter, MANAGEMENT_RATE_GLOBAL,
    MANAGEMENT_RATE_MAX_SOURCES, MANAGEMENT_RATE_PER_SOURCE_IP, MANAGEMENT_RATE_WINDOW,
};
use super::*;
use bytes::{Bytes, BytesMut};
use std::time::Instant;
use tokio::time::{timeout, Duration};

async fn send_register(socket: &UdpSocket, dest: SocketAddrV4, ttl: u16) -> Option<BvllMessage> {
    let mut buf = BytesMut::new();
    encode_bvll(
        &mut buf,
        BvlcFunction::REGISTER_FOREIGN_DEVICE,
        &ttl.to_be_bytes(),
    )
    .unwrap();
    socket.send_to(&buf, dest).await.unwrap();
    let mut recv_buf = [0u8; 2048];
    match timeout(Duration::from_millis(300), socket.recv_from(&mut recv_buf)).await {
        Ok(Ok((len, _))) => Some(decode_bvll(&recv_buf[..len]).unwrap()),
        _ => None,
    }
}

async fn send_raw(
    socket: &UdpSocket,
    dest: SocketAddrV4,
    function: BvlcFunction,
    payload: &[u8],
) -> Option<BvllMessage> {
    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, function, payload).unwrap();
    socket.send_to(&buf, dest).await.unwrap();
    let mut recv_buf = [0u8; 2048];
    match timeout(Duration::from_millis(300), socket.recv_from(&mut recv_buf)).await {
        Ok(Ok((len, _))) => Some(decode_bvll(&recv_buf[..len]).unwrap()),
        _ => None,
    }
}

fn test_ctx(
    socket: Arc<UdpSocket>,
    bbmd: Option<BbmdState>,
    npdu_tx: mpsc::Sender<ReceivedNpdu>,
) -> RecvContext {
    let local_port = socket.local_addr().unwrap().port();
    RecvContext {
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket,
        npdu_tx,
        bbmd: bbmd.map(|s| Arc::new(Mutex::new(s))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
    }
}

#[test]
fn management_rate_limit_classification() {
    for f in [
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
        BvlcFunction::REGISTER_FOREIGN_DEVICE,
        BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
    ] {
        assert!(
            is_covered_management_request(f),
            "{f:?} must share the management quota"
        );
    }

    for f in [
        BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
        BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        BvlcFunction::FORWARDED_NPDU,
        BvlcFunction::BVLC_RESULT,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
        BvlcFunction::from_raw(0x0D),
    ] {
        assert!(
            !is_covered_management_request(f),
            "{f:?} must stay outside the management quota"
        );
    }
}

#[test]
fn management_limiter_allows_16_per_ip_then_rejects() {
    let t0 = Instant::now();
    let mut limiter = ManagementRateLimiter::new();
    let ip = [192, 0, 2, 10];
    for _ in 0..MANAGEMENT_RATE_PER_SOURCE_IP {
        assert!(limiter.check(ip, t0));
    }
    assert!(!limiter.check(ip, t0));
    assert_eq!(limiter.tracked_source_count(), 1);
    assert_eq!(limiter.admitted_in_window(), MANAGEMENT_RATE_PER_SOURCE_IP);
}

#[test]
fn management_limiter_shares_combined_quota_per_source() {
    // The quota is combined across covered functions: the limiter counts
    // covered requests without distinguishing them, so any mix of 16
    // exhausts the same per-source budget.
    let t0 = Instant::now();
    let mut limiter = ManagementRateLimiter::new();
    let ip = [192, 0, 2, 11];
    for _ in 0..8 {
        assert!(limiter.check(ip, t0));
    }
    for _ in 0..8 {
        assert!(limiter.check(ip, t0));
    }
    assert!(!limiter.check(ip, t0));
}

#[test]
fn management_limiter_enforces_global_256_and_bounds_tracking() {
    let t0 = Instant::now();
    let mut limiter = ManagementRateLimiter::new();
    for i in 0..MANAGEMENT_RATE_GLOBAL {
        let ip = [10, (i >> 8) as u8, i as u8, 1];
        assert!(
            limiter.check(ip, t0),
            "distinct source {i} must be admitted"
        );
    }
    assert_eq!(limiter.admitted_in_window(), MANAGEMENT_RATE_GLOBAL);
    assert!(limiter.tracked_source_count() <= MANAGEMENT_RATE_MAX_SOURCES);

    // Both a new distinct source and an existing source are rejected once
    // the global budget is spent, and tracking never grows past the bound.
    assert!(!limiter.check([10, 9, 9, 9], t0));
    assert!(!limiter.check([10, 0, 0, 1], t0));
    assert!(limiter.tracked_source_count() <= MANAGEMENT_RATE_MAX_SOURCES);

    // Rejected requests must not create unbounded tracking state even under
    // a flood of distinct sources.
    for i in 0..512 {
        let ip = [192, 0, (i >> 8) as u8, i as u8];
        let _ = limiter.check(ip, t0);
    }
    assert!(limiter.tracked_source_count() <= MANAGEMENT_RATE_MAX_SOURCES);
    assert_eq!(limiter.admitted_in_window(), MANAGEMENT_RATE_GLOBAL);
}

#[test]
fn management_limiter_new_window_restores_admission() {
    let t0 = Instant::now();
    let mut limiter = ManagementRateLimiter::new();
    let ip = [192, 0, 2, 20];
    for _ in 0..MANAGEMENT_RATE_PER_SOURCE_IP {
        assert!(limiter.check(ip, t0));
    }
    assert!(!limiter.check(ip, t0));

    let later = t0 + MANAGEMENT_RATE_WINDOW;
    assert!(
        limiter.check(ip, later),
        "a new window must restore admission"
    );
    assert_eq!(limiter.tracked_source_count(), 1);
    assert_eq!(limiter.admitted_in_window(), 1);

    // Prior per-source accounting is cleared: a different address starts
    // fresh in the new window as well.
    assert!(limiter.check([192, 0, 2, 21], later));
    assert_eq!(limiter.tracked_source_count(), 2);
}

#[tokio::test]
async fn rate_limit_silences_17th_register_from_same_ip() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
    let dest = SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port);

    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    for i in 0..16 {
        let resp = send_register(&socket, dest, 60).await;
        assert!(
            resp.is_some(),
            "covered request {} must be admitted below the per-source quota",
            i + 1
        );
    }

    let over = send_register(&socket, dest, 60).await;
    assert!(
        over.is_none(),
        "17th covered request from the same source IP must be silently discarded"
    );

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn rate_limit_source_quota_ignores_udp_port() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    bbmd_transport.enable_foreign_device_registration(ForeignDevicePolicy {
        registration_rate_per_source: 32,
        registration_rate_global: 256,
        ..Default::default()
    });
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
    let dest = SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port);

    let socket_a = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let socket_b = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    for _ in 0..16 {
        let resp = send_register(&socket_a, dest, 60).await;
        assert!(resp.is_some(), "first 16 from socket A must be admitted");
    }

    let evader = send_register(&socket_b, dest, 60).await;
    assert!(
        evader.is_none(),
        "changing only source UDP port must not evade the per-IP quota"
    );

    // The evading port must not have created a second FDT entry.
    {
        let state = bbmd_transport.bbmd_state().unwrap();
        let mut state = state.lock().await;
        assert_eq!(
            state.fdt().len(),
            1,
            "over-limit Register must not mutate FDT state"
        );
    }

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn rate_limit_bounds_non_bbmd_naks() {
    let mut server = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = server.start().await.unwrap();
    let server_mac = server.local_mac().to_vec();
    let (ip, port) = decode_bip_mac(&server_mac).unwrap();
    let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    for i in 0..16 {
        let resp = send_raw(
            &socket,
            dest,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            &[],
        )
        .await
        .unwrap_or_else(|| panic!("non-BBMD NAK {i} must be emitted below quota"));
        assert_eq!(resp.function, BvlcFunction::BVLC_RESULT);
        assert_eq!(
            decode_bvlc_result_code(&resp).unwrap(),
            BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK
        );
    }

    assert!(
        send_raw(
            &socket,
            dest,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            &[],
        )
        .await
        .is_none(),
        "repeated non-BBMD NAKs must also be bounded by the limiter"
    );

    server.stop().await.unwrap();
}

#[tokio::test]
async fn rate_limit_discards_malformed_and_unauthorized_before_normal_handling() {
    let server_socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let local_port = server_socket.local_addr().unwrap().port();
    let peer = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let peer_port = peer.local_addr().unwrap().port();
    let sender = (Ipv4Addr::LOCALHOST.octets(), peer_port);
    let (npdu_tx, _npdu_rx) = mpsc::channel(8);

    // Empty ACL denies every Delete-FDT-Entry sender.
    let state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    let ctx = RecvContext {
        local_mac: encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), local_port),
        socket: Arc::clone(&server_socket),
        npdu_tx,
        bbmd: Some(Arc::new(Mutex::new(state))),
        broadcast_addr: Ipv4Addr::LOCALHOST,
        broadcast_port: local_port,
        pending_bvlc_response: Arc::new(Mutex::new(None)),
        management_limiter: Arc::new(std::sync::Mutex::new(ManagementRateLimiter::new())),
        fanout: None,
        force_dbtn_forward_failure: false,
    };

    async fn recv_one(peer: &UdpSocket) -> Option<BvllMessage> {
        let mut buf = [0u8; 2048];
        match timeout(Duration::from_millis(300), peer.recv_from(&mut buf)).await {
            Ok(Ok((len, _))) => Some(decode_bvll(&buf[..len]).unwrap()),
            _ => None,
        }
    }

    async fn send_covered(
        server: &RecvContext,
        sender: ([u8; 4], u16),
        function: BvlcFunction,
        payload: &[u8],
    ) {
        let msg = BvllMessage {
            function,
            payload: Bytes::copy_from_slice(payload),
            originating_ip: None,
            originating_port: None,
        };
        handle_bvll_message(&msg, sender, server).await;
    }

    // Fill the combined quota with a mix: 8 reads + 8 deletes. Each delete
    // is unauthorized and NAKs, proving normal handling below the limit.
    for _ in 0..8 {
        send_covered(
            &ctx,
            sender,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            &[],
        )
        .await;
        assert!(recv_one(&peer).await.is_some());
    }
    for _ in 0..8 {
        send_covered(
            &ctx,
            sender,
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            &[127, 0, 0, 9, 0xBA, 0xC0],
        )
        .await;
        let resp = recv_one(&peer).await.expect("delete NAK below quota");
        assert_eq!(
            decode_bvlc_result_code(&resp).unwrap(),
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
        );
    }

    // Quota is now spent: a malformed Register (which would normally NAK on
    // payload validation) and an unauthorized Delete (which would normally
    // NAK on ACL) are both silently discarded before that handling.
    send_covered(&ctx, sender, BvlcFunction::REGISTER_FOREIGN_DEVICE, &[0x00]).await;
    assert!(
        recv_one(&peer).await.is_none(),
        "over-limit malformed Register must be silent, not a validation NAK"
    );

    send_covered(
        &ctx,
        sender,
        BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
        &[127, 0, 0, 9, 0xBA, 0xC0],
    )
    .await;
    assert!(
        recv_one(&peer).await.is_none(),
        "over-limit Delete must be silent, not an ACL NAK"
    );

    // A malformed Delete payload is likewise discarded before length checks.
    send_covered(
        &ctx,
        sender,
        BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
        &[0x00],
    )
    .await;
    assert!(
        recv_one(&peer).await.is_none(),
        "over-limit malformed Delete must be silent"
    );

    // Read-FDT from the same source is also covered.
    send_covered(&ctx, sender, BvlcFunction::READ_FOREIGN_DEVICE_TABLE, &[]).await;
    assert!(
        recv_one(&peer).await.is_none(),
        "over-limit Read-FDT must be silent"
    );

    // No FDT mutation could have occurred through the discarded path.
    let bbmd = ctx.bbmd.as_ref().unwrap();
    assert!(bbmd.lock().await.fdt().is_empty());
}

#[tokio::test]
async fn rate_limit_write_bdt_still_naks_after_quota_exhaustion() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (ip, port) = decode_bip_mac(&bbmd_mac).unwrap();
    let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    for _ in 0..16 {
        assert!(send_register(&socket, dest, 60).await.is_some());
    }
    // Quota is exhausted for covered requests.
    assert!(send_register(&socket, dest, 60).await.is_none());

    // Write-BDT is explicitly excluded and must NAK every time.
    for _ in 0..2 {
        let resp = send_raw(
            &socket,
            dest,
            BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
            &[],
        )
        .await
        .expect("Write-BDT must still NAK after quota exhaustion");
        assert_eq!(
            decode_bvlc_result_code(&resp).unwrap(),
            BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
        );
    }

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn rate_limit_leaves_dbtn_and_npdu_outside_limiter() {
    let server_socket = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let local_port = server_socket.local_addr().unwrap().port();
    let peer = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let peer_port = peer.local_addr().unwrap().port();
    let sender = (Ipv4Addr::LOCALHOST.octets(), peer_port);
    let (npdu_tx, mut npdu_rx) = mpsc::channel(8);
    let state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local_port);
    let ctx = test_ctx(Arc::clone(&server_socket), Some(state), npdu_tx);

    // Exhaust the management quota from this source IP.
    for _ in 0..MANAGEMENT_RATE_PER_SOURCE_IP {
        let msg = BvllMessage {
            function: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            payload: Bytes::new(),
            originating_ip: None,
            originating_port: None,
        };
        handle_bvll_message(&msg, sender, &ctx).await;
        // Drain each admitted Read-BDT-ACK from the peer.
        let mut buf = [0u8; 2048];
        timeout(Duration::from_millis(300), peer.recv_from(&mut buf))
            .await
            .expect("admitted read below quota must answer")
            .unwrap();
    }
    // Confirm exhaustion: one more covered request is silent.
    let over = BvllMessage {
        function: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
        payload: Bytes::new(),
        originating_ip: None,
        originating_port: None,
    };
    handle_bvll_message(&over, sender, &ctx).await;
    {
        let mut buf = [0u8; 2048];
        assert!(
            timeout(Duration::from_millis(100), peer.recv_from(&mut buf))
                .await
                .is_err(),
            "quota must be exhausted before the data-plane checks"
        );
    }

    // Ordinary NPDU traffic still flows.
    let npdu = BvllMessage {
        function: BvlcFunction::ORIGINAL_UNICAST_NPDU,
        payload: Bytes::from_static(&[0x01, 0x04, 0xAA]),
        originating_ip: None,
        originating_port: None,
    };
    handle_bvll_message(&npdu, sender, &ctx).await;
    let received = timeout(Duration::from_secs(2), npdu_rx.recv())
        .await
        .expect("NPDU must bypass the management limiter")
        .expect("channel open");
    assert_eq!(received.npdu.as_ref(), &[0x01, 0x04, 0xAA]);

    // Distribute-Broadcast-To-Network is data plane: an unregistered sender
    // still gets its NAK even after management quota exhaustion.
    let dbtn = BvllMessage {
        function: BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        payload: Bytes::from_static(&[0x01, 0x20, 0xAA]),
        originating_ip: None,
        originating_port: None,
    };
    handle_bvll_message(&dbtn, sender, &ctx).await;
    {
        let mut buf = [0u8; 2048];
        let (len, _) = timeout(Duration::from_secs(2), peer.recv_from(&mut buf))
            .await
            .expect("DBTN NAK must bypass the management limiter")
            .unwrap();
        let resp = decode_bvll(&buf[..len]).unwrap();
        assert_eq!(
            decode_bvlc_result_code(&resp).unwrap(),
            BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
        );
    }
}

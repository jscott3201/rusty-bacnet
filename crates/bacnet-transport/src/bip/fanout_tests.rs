//! Tests for broadcast forwarding fanout budgeting, deduplication, and non-starvation (Issue #531).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::fanout::{FanoutPolicy, FanoutRateLimiter};
use super::*;
use crate::bbmd::{BdtEntry, ForeignDevicePolicy};
use crate::bvll::{decode_bip_mac, decode_bvll, encode_bvll, BvllMessage};
use crate::port::TransportPort;
use bacnet_types::enums::BvlcFunction;

async fn recv_bvll(socket: &UdpSocket) -> BvllMessage {
    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLL frame")
        .expect("recv_from error");
    decode_bvll(&recv_buf[..len]).expect("valid BVLL message")
}

async fn assert_no_bvll(socket: &UdpSocket, label: &str) {
    let mut recv_buf = [0u8; 2048];
    assert!(
        timeout(Duration::from_millis(100), socket.recv_from(&mut recv_buf))
            .await
            .is_err(),
        "{label} received an unexpected extra BVLL frame"
    );
}

#[test]
fn fanout_rate_limiter_budgets_and_throttles() {
    let policy = FanoutPolicy {
        max_fanout_per_input: 4,
        max_packets_per_sec_global: 10,
        max_bytes_per_sec_global: 2000,
        max_packets_per_sec_per_origin: 6,
        queue_capacity: 128,
    };
    let mut limiter = FanoutRateLimiter::new(policy);
    let t0 = Instant::now();
    let origin_a = [10, 0, 0, 1];
    let origin_b = [10, 0, 0, 2];
    let packet_bytes = 100;

    // Origin A: input with 8 candidates -> capped to per_input (4)
    let (admitted, throttled) = limiter.check_and_admit(origin_a, 8, packet_bytes, t0);
    assert_eq!(admitted, 4);
    assert_eq!(throttled, 4);

    // Origin A: 2nd input with 4 candidates -> origin budget remaining is 6 - 4 = 2
    let (admitted, throttled) = limiter.check_and_admit(origin_a, 4, packet_bytes, t0);
    assert_eq!(admitted, 2);
    assert_eq!(throttled, 2);

    // Origin A: 3rd input -> origin budget exhausted
    let (admitted, throttled) = limiter.check_and_admit(origin_a, 4, packet_bytes, t0);
    assert_eq!(admitted, 0);
    assert_eq!(throttled, 4);

    // Origin B: input with 6 candidates -> global remaining is 10 - 6 = 4
    let (admitted, throttled) = limiter.check_and_admit(origin_b, 6, packet_bytes, t0);
    assert_eq!(admitted, 4);
    assert_eq!(throttled, 2);

    // Global budget now exhausted (10 admitted)
    let (admitted, throttled) = limiter.check_and_admit(origin_b, 4, packet_bytes, t0);
    assert_eq!(admitted, 0);
    assert_eq!(throttled, 4);

    // After 1 second window roll, budgets restore
    let t1 = t0 + Duration::from_millis(1100);
    let (admitted, throttled) = limiter.check_and_admit(origin_a, 4, packet_bytes, t1);
    assert_eq!(admitted, 4);
    assert_eq!(throttled, 0);
}

#[tokio::test]
async fn duplicate_bdt_and_fdt_entries_yield_exactly_one_send_per_destination() {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let sink_socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let sink_port = sink_socket.local_addr().unwrap().port();

    // BDT entry that points to the sink port (unicast /32)
    bbmd.enable_bbmd(vec![BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port: sink_port,
        broadcast_mask: [255, 255, 255, 255],
    }]);
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let _bbmd_rx = bbmd.start().await.unwrap();
    let bbmd_mac = bbmd.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    // Register foreign device that also points to the same sink IP and port
    let fd_reg = {
        let mut buf = BytesMut::new();
        encode_bvll(
            &mut buf,
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            &60u16.to_be_bytes(),
        )
        .unwrap();
        buf.freeze()
    };
    sink_socket
        .send_to(
            &fd_reg,
            SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port),
        )
        .await
        .unwrap();
    let reg_result = recv_bvll(&sink_socket).await;
    assert_eq!(reg_result.function, BvlcFunction::BVLC_RESULT);

    // Send Original-Broadcast-NPDU to the BBMD from a client
    let client_sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let test_npdu = vec![0x01, 0x20, 0xCA, 0xFE];
    let mut bcast_buf = BytesMut::new();
    encode_bvll(
        &mut bcast_buf,
        BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        &test_npdu,
    )
    .unwrap();
    client_sock
        .send_to(
            &bcast_buf,
            SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port),
        )
        .await
        .unwrap();

    // The sink must receive exactly ONE forwarded NPDU (not 2 copies)
    let frame = recv_bvll(&sink_socket).await;
    assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(frame.payload.as_ref(), test_npdu.as_slice());
    assert_no_bvll(&sink_socket, "sink deduplication check").await;

    // Verify fanout counters
    let counters = bbmd.fanout_counters();
    assert_eq!(counters.packets_forwarded, 1);
    assert_eq!(counters.destinations_deduplicated, 1);

    bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn sustained_broadcast_input_does_not_starve_concurrent_unicast() {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut bdt = Vec::new();
    for i in 1..=8 {
        bdt.push(BdtEntry {
            ip: [192, 168, 1, i],
            port: 47808,
            broadcast_mask: [255, 255, 255, 255],
        });
    }
    bbmd.enable_bbmd(bdt);
    // Tight queue and rate limit
    bbmd.set_fanout_policy(FanoutPolicy {
        max_fanout_per_input: 8,
        max_packets_per_sec_global: 16,
        max_bytes_per_sec_global: 32_000,
        max_packets_per_sec_per_origin: 16,
        queue_capacity: 2,
    });
    let mut bbmd_rx = bbmd.start().await.unwrap();
    let bbmd_mac = bbmd.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
    let bbmd_dest = SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port);

    // Flood BBMD with continuous broadcast traffic in background
    let flood_sock = Arc::new(
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap(),
    );
    let stop_flood = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_flood_clone = Arc::clone(&stop_flood);
    let flood_handle = tokio::spawn(async move {
        let mut buf = BytesMut::new();
        encode_bvll(
            &mut buf,
            BvlcFunction::ORIGINAL_BROADCAST_NPDU,
            &[0x01, 0x20, 0x11, 0x22],
        )
        .unwrap();
        let frame = buf.freeze();
        while !stop_flood_clone.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = flood_sock.send_to(&frame, bbmd_dest).await;
            tokio::task::yield_now().await;
        }
    });

    // Let flood start sending frames
    tokio::time::sleep(Duration::from_millis(15)).await;

    // Send concurrent unicast NPDU and ensure it is received promptly
    let unicast_sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let unicast_npdu = vec![0x01, 0x00, 0x99, 0x88, 0x77];
    let mut unicast_buf = BytesMut::new();
    encode_bvll(
        &mut unicast_buf,
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        &unicast_npdu,
    )
    .unwrap();

    let start_time = Instant::now();
    unicast_sock.send_to(&unicast_buf, bbmd_dest).await.unwrap();

    // Loop until we find our unicast NPDU (broadcasts also deliver to npdu_rx)
    let mut found_unicast = false;
    while start_time.elapsed() < Duration::from_secs(2) {
        if let Ok(Some(received)) = timeout(Duration::from_millis(100), bbmd_rx.recv()).await {
            if received.npdu == unicast_npdu {
                found_unicast = true;
                break;
            }
        }
    }

    stop_flood.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = flood_handle.await;

    assert!(
        found_unicast,
        "concurrent unicast NPDU was starved or lost during sustained broadcast flood"
    );
    assert!(
        start_time.elapsed() < Duration::from_millis(500),
        "concurrent unicast NPDU took too long: {:?}",
        start_time.elapsed()
    );

    // Queue overflow drops occurred during flood
    let counters = bbmd.fanout_counters();
    assert!(
        counters.queue_overflow_drops > 0 || counters.packets_throttled > 0,
        "flood must trigger fanout queue overflow or throttling: {counters:?}"
    );

    bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn fanout_semantics_for_original_forwarded_and_dbtn() {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let peer_bdt = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let peer_bdt_port = peer_bdt.local_addr().unwrap().port();
    let peer_fd = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let _peer_fd_port = peer_fd.local_addr().unwrap().port();

    bbmd.enable_bbmd(vec![BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port: peer_bdt_port,
        broadcast_mask: [255, 255, 255, 255],
    }]);
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());

    let _bbmd_rx = bbmd.start().await.unwrap();
    let bbmd_mac = bbmd.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
    let bbmd_dest = SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port);

    // 1. Register foreign device
    let mut reg_buf = BytesMut::new();
    encode_bvll(
        &mut reg_buf,
        BvlcFunction::REGISTER_FOREIGN_DEVICE,
        &60u16.to_be_bytes(),
    )
    .unwrap();
    peer_fd.send_to(&reg_buf, bbmd_dest).await.unwrap();
    let reg_ack = recv_bvll(&peer_fd).await;
    assert_eq!(reg_ack.function, BvlcFunction::BVLC_RESULT);

    // 2. Test DBTN: from registered foreign device -> forwards to BDT peer
    let dbtn_npdu = vec![0x01, 0x20, 0xAA, 0x11];
    let mut dbtn_buf = BytesMut::new();
    encode_bvll(
        &mut dbtn_buf,
        BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        &dbtn_npdu,
    )
    .unwrap();
    peer_fd.send_to(&dbtn_buf, bbmd_dest).await.unwrap();

    let dbtn_forwarded = recv_bvll(&peer_bdt).await;
    assert_eq!(dbtn_forwarded.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(dbtn_forwarded.payload.as_ref(), dbtn_npdu.as_slice());
    // DBTN does not echo back to sender
    assert_no_bvll(&peer_fd, "DBTN sender").await;

    // 3. Test Forwarded-NPDU from BDT peer -> forwards to foreign device
    let fwd_npdu = vec![0x01, 0x20, 0xBB, 0x22];
    let mut fwd_buf = BytesMut::new();
    crate::bvll::encode_bvll_forwarded(&mut fwd_buf, [10, 1, 2, 3], 47808, &fwd_npdu).unwrap();
    peer_bdt.send_to(&fwd_buf, bbmd_dest).await.unwrap();

    let fwd_to_fd = recv_bvll(&peer_fd).await;
    assert_eq!(fwd_to_fd.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(fwd_to_fd.payload.as_ref(), fwd_npdu.as_slice());
    assert_eq!(fwd_to_fd.originating_ip, Some([10, 1, 2, 3]));

    // 4. Test DBTN from unregistered sender -> returns NAK
    let unreg_sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    unreg_sock.send_to(&dbtn_buf, bbmd_dest).await.unwrap();
    let nak_res = recv_bvll(&unreg_sock).await;
    assert_eq!(nak_res.function, BvlcFunction::BVLC_RESULT);
    assert_eq!(
        crate::bip::decode_bvlc_result_code(&nak_res).unwrap(),
        bacnet_types::enums::BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
    );

    bbmd.stop().await.unwrap();
}

#[tokio::test]
async fn fanout_counters_accurately_track_all_metrics() {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let sink_a = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port_a = sink_a.local_addr().unwrap().port();

    bbmd.enable_bbmd(vec![BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port: port_a,
        broadcast_mask: [255, 255, 255, 255],
    }]);
    bbmd.set_fanout_policy(FanoutPolicy {
        max_fanout_per_input: 1,
        max_packets_per_sec_global: 10,
        max_bytes_per_sec_global: 5000,
        max_packets_per_sec_per_origin: 2,
        queue_capacity: 16,
    });
    let _rx = bbmd.start().await.unwrap();
    let bbmd_mac = bbmd.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
    let bbmd_dest = SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port);

    let client = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let npdu = vec![0x01, 0x20, 0x55, 0x66];
    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, &npdu).unwrap();
    let frame = buf.freeze();

    // Send 1st broadcast: forwarded
    client.send_to(&frame, bbmd_dest).await.unwrap();
    let _ = recv_bvll(&sink_a).await;

    // Send 2nd broadcast: forwarded (origin quota = 2)
    client.send_to(&frame, bbmd_dest).await.unwrap();
    let _ = recv_bvll(&sink_a).await;

    // Send 3rd broadcast: throttled (origin quota exceeded)
    client.send_to(&frame, bbmd_dest).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let counters = bbmd.fanout_counters();
    assert_eq!(counters.packets_forwarded, 2);
    assert!(counters.bytes_forwarded > 0);
    assert_eq!(counters.packets_throttled, 1);

    bbmd.stop().await.unwrap();
}

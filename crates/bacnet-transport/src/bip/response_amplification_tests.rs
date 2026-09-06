use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::rate_limit::{ManagementRateLimiter, MANAGEMENT_RESPONSE_BYTES_GLOBAL};
use super::*;
use crate::bbmd::{self, BbmdState, BdtEntry, BDT_ENTRY_SIZE, FDT_ENTRY_SIZE};
use crate::bvll::{decode_bip_mac, decode_bvll, encode_bip_mac, encode_bvll};
use bacnet_types::enums::{BvlcFunction, BvlcResultCode};

#[test]
fn test_bdt_empty_and_max_response_sizes() {
    // Empty BDT payload -> 4-byte BVLC header only
    let mut empty_payload = BytesMut::new();
    bbmd::encode_bdt_entries(&[], &mut empty_payload);
    assert_eq!(empty_payload.len(), 0);
    let mut empty_buf = BytesMut::new();
    encode_bvll(
        &mut empty_buf,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
        &empty_payload,
    )
    .unwrap();
    assert_eq!(empty_buf.len(), 4, "Empty Read-BDT-ACK must be 4 bytes");

    // Maximum BDT (128 entries): 128 * 10 = 1280 payload bytes -> 1284 total
    let entries: Vec<BdtEntry> = (0..BbmdState::MAX_BDT_ENTRIES)
        .map(|i| BdtEntry {
            ip: [10, 0, (i / 256) as u8, (i % 256) as u8],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        })
        .collect();
    assert_eq!(entries.len(), 128);
    let mut max_payload = BytesMut::new();
    bbmd::encode_bdt_entries(&entries, &mut max_payload);
    assert_eq!(max_payload.len(), 128 * BDT_ENTRY_SIZE);
    let mut max_buf = BytesMut::new();
    encode_bvll(
        &mut max_buf,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
        &max_payload,
    )
    .unwrap();
    assert_eq!(
        max_buf.len(),
        1284,
        "Maximum Read-BDT-ACK (128 entries) must be 1,284 bytes"
    );

    let decoded = BbmdState::decode_bdt(&max_payload).unwrap();
    assert_eq!(decoded.len(), 128);
}

#[test]
fn test_fdt_empty_and_max_response_sizes() {
    let mut state = BbmdState::new([127, 0, 0, 1], 0xBAC0);
    state.enable_foreign_device_registration(ForeignDevicePolicy {
        registration_rate_global: 256,
        max_entries_per_source: 128,
        ..Default::default()
    });

    // Empty FDT payload -> 4-byte BVLC header only
    let mut empty_payload = BytesMut::new();
    state.encode_fdt(&mut empty_payload);
    assert_eq!(empty_payload.len(), 0);
    let mut empty_buf = BytesMut::new();
    encode_bvll(
        &mut empty_buf,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
        &empty_payload,
    )
    .unwrap();
    assert_eq!(empty_buf.len(), 4, "Empty Read-FDT-ACK must be 4 bytes");

    // Maximum FDT (128 entries): 128 * 10 = 1280 payload bytes -> 1284 total
    for i in 0..BbmdState::MAX_FDT_ENTRIES {
        let ip = [10, 0, (i / 256) as u8, (i % 256) as u8];
        let port = 0xBAC0 + (i as u16);
        assert_eq!(
            state.register_foreign_device(ip, port, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    // Attempting 129th entry must be rejected
    assert_eq!(
        state.register_foreign_device([10, 1, 0, 1], 0xBAC0, 60),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    assert_eq!(state.fdt().len(), 128);

    let mut max_payload = BytesMut::new();
    state.encode_fdt(&mut max_payload);
    assert_eq!(max_payload.len(), 128 * FDT_ENTRY_SIZE);
    let mut max_buf = BytesMut::new();
    encode_bvll(
        &mut max_buf,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
        &max_payload,
    )
    .unwrap();
    assert_eq!(
        max_buf.len(),
        1284,
        "Maximum Read-FDT-ACK (128 entries) must be 1,284 bytes"
    );

    let decoded = bbmd::decode_fdt(&max_payload).unwrap();
    assert_eq!(decoded.len(), 128);
}

#[test]
fn test_decode_fdt_rejects_exceeding_max_entries() {
    let wire_bytes_129 = vec![0u8; 129 * FDT_ENTRY_SIZE];
    let err = bbmd::decode_fdt(&wire_bytes_129).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("exceeds maximum of 128"),
        "error must cite maximum of 128: {msg}"
    );

    // 128 entries is accepted
    let wire_bytes_128 = vec![0u8; 128 * FDT_ENTRY_SIZE];
    assert!(bbmd::decode_fdt(&wire_bytes_128).is_ok());
}

#[tokio::test]
async fn test_max_fdt_response_accepted_by_bip_client() {
    let server = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let server_port = server.local_addr().unwrap().port();
    let server_mac = encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), server_port);

    // Responder sends a 128-entry (1,284 bytes) Read-FDT-ACK
    let responder = tokio::spawn(async move {
        let mut recv_buf = [0u8; 2048];
        let (_len, client_addr) = timeout(Duration::from_secs(2), server.recv_from(&mut recv_buf))
            .await
            .expect("timed out waiting for management request")
            .unwrap();

        let mut payload = BytesMut::with_capacity(128 * FDT_ENTRY_SIZE);
        for i in 0..128u16 {
            payload.extend_from_slice(&[10, 0, (i / 256) as u8, (i % 256) as u8]);
            payload.extend_from_slice(&(0xBAC0 + i).to_be_bytes());
            payload.extend_from_slice(&60u16.to_be_bytes());
            payload.extend_from_slice(&60u16.to_be_bytes());
        }
        let mut response = BytesMut::with_capacity(4 + payload.len());
        encode_bvll(
            &mut response,
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
            &payload,
        )
        .unwrap();
        assert_eq!(response.len(), 1284);
        server.send_to(&response, client_addr).await.unwrap();
    });

    let mut client = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client.start().await.unwrap();

    let entries = client.read_fdt(&server_mac).await.unwrap();
    assert_eq!(entries.len(), 128);

    client.stop().await.unwrap();
    responder.await.unwrap();
}

#[test]
fn test_response_budget_per_source_throttling() {
    let t0 = Instant::now();
    let mut limiter = ManagementRateLimiter::new();
    let ip = [192, 0, 2, 50];

    // Response of 1,284 bytes:
    // 1st: 1,284 <= 4,096 -> ok
    assert!(limiter.check_response(ip, 1284, t0));
    limiter.record_read_bdt_response(1284);
    assert_eq!(limiter.response_bytes_in_window(), 1284);

    // 2nd: 2,568 <= 4,096 -> ok
    assert!(limiter.check_response(ip, 1284, t0));
    limiter.record_read_bdt_response(1284);
    assert_eq!(limiter.response_bytes_in_window(), 2568);

    // 3rd: 3,852 <= 4,096 -> ok
    assert!(limiter.check_response(ip, 1284, t0));
    limiter.record_read_bdt_response(1284);
    assert_eq!(limiter.response_bytes_in_window(), 3852);

    // 4th: 3,852 + 1,284 = 5,136 > 4,096 -> throttled!
    assert!(!limiter.check_response(ip, 1284, t0));
    limiter.record_response_throttled();
    assert_eq!(limiter.response_bytes_in_window(), 3852);

    let counters = limiter.counters();
    assert_eq!(counters.read_bdt_responses, 3);
    assert_eq!(counters.response_bytes_sent, 3852);
    assert_eq!(counters.response_throttled, 1);

    // Window rollover restores budget
    let later = t0 + Duration::from_millis(1001);
    assert!(limiter.check_response(ip, 1284, later));
    assert_eq!(limiter.response_bytes_in_window(), 1284);
}

#[test]
fn test_response_budget_global_throttling() {
    let t0 = Instant::now();
    let mut limiter = ManagementRateLimiter::new();

    // 8 distinct sources each take 4,096 bytes: 8 * 4096 = 32,768 (MANAGEMENT_RESPONSE_BYTES_GLOBAL)
    for i in 0..8u8 {
        let ip = [10, 0, 0, i];
        assert!(limiter.check_response(ip, 4096, t0));
    }
    assert_eq!(
        limiter.response_bytes_in_window(),
        MANAGEMENT_RESPONSE_BYTES_GLOBAL
    );

    // 9th source is rejected globally even for a small 4-byte response
    let new_ip = [10, 0, 0, 99];
    assert!(!limiter.check_response(new_ip, 4, t0));
}

#[tokio::test]
async fn test_bbmd_wire_read_bdt_throttling_and_npdu_traffic() {
    // Set up BBMD with 127 peer BDT entries; self-entry auto-insertion reaches 128 entries (1,284 bytes)
    let entries: Vec<BdtEntry> = (0..(BbmdState::MAX_BDT_ENTRIES - 1))
        .map(|i| BdtEntry {
            ip: [10, 0, (i / 256) as u8, (i % 256) as u8],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        })
        .collect();

    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(entries);
    let mut bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
    let bbmd_dest = SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port);

    let client_socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    let mut req_buf = BytesMut::new();
    encode_bvll(
        &mut req_buf,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
        &[],
    )
    .unwrap();

    // Requests 1, 2, 3 should succeed (3 * 1,284 = 3,852 <= 4,096)
    for _ in 0..3 {
        client_socket.send_to(&req_buf, bbmd_dest).await.unwrap();
        let mut recv_buf = [0u8; 2048];
        let (len, _) = timeout(
            Duration::from_millis(500),
            client_socket.recv_from(&mut recv_buf),
        )
        .await
        .expect("expected Read-BDT-ACK response")
        .unwrap();
        let msg = decode_bvll(&recv_buf[..len]).unwrap();
        assert_eq!(
            msg.function,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
        );
        assert_eq!(len, 1284);
    }

    // Request 4 should be throttled (3,852 + 1,284 = 5,136 > 4,096) -> no response sent
    client_socket.send_to(&req_buf, bbmd_dest).await.unwrap();
    let mut recv_buf = [0u8; 2048];
    let timeout_result = timeout(
        Duration::from_millis(300),
        client_socket.recv_from(&mut recv_buf),
    )
    .await;
    assert!(
        timeout_result.is_err(),
        "4th Read-BDT request must be throttled and yield no response"
    );

    // Verify operational counters on BBMD
    let counters = bbmd_transport.management_counters();
    assert_eq!(counters.read_bdt_responses, 3);
    assert_eq!(counters.response_bytes_sent, 3852);
    assert_eq!(counters.response_throttled, 1);

    // Verify NPDU traffic from the same client is NOT starved or affected
    let mut npdu_buf = BytesMut::new();
    let npdu_payload = [0x01, 0x04, 0xDE, 0xAD, 0xBE, 0xEF];
    encode_bvll(
        &mut npdu_buf,
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        &npdu_payload,
    )
    .unwrap();
    client_socket.send_to(&npdu_buf, bbmd_dest).await.unwrap();

    let received = timeout(Duration::from_millis(500), bbmd_rx.recv())
        .await
        .expect("NPDU must not be blocked by management rate limiting")
        .expect("NPDU channel must be open");
    assert_eq!(received.npdu.as_ref(), &npdu_payload);

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn test_bbmd_wire_read_fdt_throttling() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    // Register 128 foreign devices directly in BBMD state
    {
        let mut state = BbmdState::new([127, 0, 0, 1], 0xBAC0);
        state.enable_foreign_device_registration(ForeignDevicePolicy {
            registration_rate_global: 256,
            max_entries_per_source: 128,
            ..Default::default()
        });
        for i in 0..BbmdState::MAX_FDT_ENTRIES {
            let ip = [10, 0, (i / 256) as u8, (i % 256) as u8];
            state.register_foreign_device(ip, 0xBAC0 + (i as u16), 300);
        }
        bbmd_transport.enable_bbmd(state.bdt().to_vec());
        bbmd_transport.enable_foreign_device_registration(ForeignDevicePolicy {
            registration_rate_global: 256,
            max_entries_per_source: 128,
            ..Default::default()
        });
    }

    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();
    let bbmd_dest = SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port);

    // Populate the running BBMD's FDT with 128 entries
    if let Some(bbmd_arc) = bbmd_transport.bbmd_state() {
        let mut state = bbmd_arc.lock().await;
        for i in 0..BbmdState::MAX_FDT_ENTRIES {
            let ip = [10, 0, (i / 256) as u8, (i % 256) as u8];
            state.register_foreign_device(ip, 0xBAC0 + (i as u16), 300);
        }
    }

    let client_socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();

    let mut req_buf = BytesMut::new();
    encode_bvll(&mut req_buf, BvlcFunction::READ_FOREIGN_DEVICE_TABLE, &[]).unwrap();

    // Requests 1, 2, 3 should succeed
    for _ in 0..3 {
        client_socket.send_to(&req_buf, bbmd_dest).await.unwrap();
        let mut recv_buf = [0u8; 2048];
        let (len, _) = timeout(
            Duration::from_millis(500),
            client_socket.recv_from(&mut recv_buf),
        )
        .await
        .expect("expected Read-FDT-ACK response")
        .unwrap();
        let msg = decode_bvll(&recv_buf[..len]).unwrap();
        assert_eq!(msg.function, BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK);
        assert_eq!(len, 1284);
    }

    // Request 4 should be throttled
    client_socket.send_to(&req_buf, bbmd_dest).await.unwrap();
    let mut recv_buf = [0u8; 2048];
    let timeout_result = timeout(
        Duration::from_millis(300),
        client_socket.recv_from(&mut recv_buf),
    )
    .await;
    assert!(
        timeout_result.is_err(),
        "4th Read-FDT request must be throttled"
    );

    let counters = bbmd_transport.management_counters();
    assert_eq!(counters.read_fdt_responses, 3);
    assert_eq!(counters.response_bytes_sent, 3852);
    assert_eq!(counters.response_throttled, 1);

    bbmd_transport.stop().await.unwrap();
}

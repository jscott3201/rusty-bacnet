use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;

use super::ingress::original_destination_matches;
use super::vmac_table::derive_vmac_from_device_instance;
use super::*;
use crate::port::TransportPort;

#[test]
fn original_function_requires_matching_ip_destination_and_vmac() {
    let local_ip = Ipv6Addr::LOCALHOST;
    let local_vmac = [1, 2, 3];

    assert!(original_destination_matches(
        Bvlc6Function::OriginalUnicast,
        local_ip.into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::OriginalUnicast,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::OriginalUnicast,
        local_ip.into(),
        Some([9, 9, 9]),
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        None,
    ));
    assert!(original_destination_matches(
        Bvlc6Function::OriginalBroadcast,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        None,
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::AddressResolutionAck,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        Some(true),
    ));

    assert!(original_destination_matches(
        Bvlc6Function::OriginalUnicast,
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9).into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip, Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9),],
        true,
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::OriginalUnicast,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip],
        true,
        Some(true),
    ));
    assert!(original_destination_matches(
        Bvlc6Function::AddressResolution,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        Some(true),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::AddressResolution,
        local_ip.into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        Some(false),
    ));
    assert!(original_destination_matches(
        Bvlc6Function::VirtualAddressResolution,
        local_ip.into(),
        None,
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        Some(false),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::VirtualAddressResolution,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        None,
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        Some(true),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ForwardedAddressResolution,
        local_ip.into(),
        Some(local_vmac),
        local_ip,
        local_vmac,
        &[local_ip],
        false,
        Some(false),
    ));
}

#[tokio::test]
async fn vmac_table_preserves_scope_and_replaces_address_owner() {
    let table = super::vmac_table::VmacTable::new();
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
    let first = SocketAddrV6::new(ip, 47808, 0, 4);
    let replacement = SocketAddrV6::new(ip, 47808, 0, 4);

    table.learn([1, 2, 3], first).await;
    assert_eq!(
        table.resolve_by_addr(ip, 47808).await,
        Some(([1, 2, 3], first))
    );

    table.learn([4, 5, 6], replacement).await;
    assert_eq!(table.lookup(&[1, 2, 3]).await, None);
    assert_eq!(
        table.resolve_by_addr(ip, 47808).await,
        Some(([4, 5, 6], replacement))
    );
}

#[tokio::test]
async fn vmac_table_fails_closed_for_ambiguous_scope() {
    let table = super::vmac_table::VmacTable::new();
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);

    table
        .learn([1, 2, 3], SocketAddrV6::new(ip, 47808, 0, 4))
        .await;
    table
        .learn([4, 5, 6], SocketAddrV6::new(ip, 47808, 0, 7))
        .await;

    assert_eq!(table.resolve_by_addr(ip, 47808).await, None);
}

#[tokio::test]
async fn vmac_table_is_bounded() {
    let table = super::vmac_table::VmacTable::new();
    for index in 0..super::vmac_table::MAX_VMAC_TABLE_ENTRIES {
        let vmac = [
            ((index >> 16) & 0xFF) as u8,
            ((index >> 8) & 0xFF) as u8,
            (index & 0xFF) as u8,
        ];
        table
            .learn(
                vmac,
                SocketAddrV6::new(Ipv6Addr::LOCALHOST, 10_000 + index as u16, 0, 0),
            )
            .await;
    }
    table
        .learn(
            [0xFE, 0xFE, 0xFE],
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, 9999, 0, 0),
        )
        .await;

    assert_eq!(table.len().await, super::vmac_table::MAX_VMAC_TABLE_ENTRIES);
    assert_eq!(table.lookup(&[0xFE, 0xFE, 0xFE]).await, None);

    table
        .learn(
            [0xFD, 0xFD, 0xFD],
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, 10_000, 0, 0),
        )
        .await;
    assert_eq!(table.len().await, super::vmac_table::MAX_VMAC_TABLE_ENTRIES);
    assert_eq!(table.lookup(&[0, 0, 0]).await, None);
    assert!(table.lookup(&[0xFD, 0xFD, 0xFD]).await.is_some());
}

#[tokio::test]
async fn vmac_table_does_not_retain_ambiguous_reverse_entries() {
    let table = super::vmac_table::VmacTable::new();
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);

    for port in 10_000..10_100 {
        table
            .learn([1, 2, 3], SocketAddrV6::new(ip, port, 0, 4))
            .await;
        table
            .learn([4, 5, 6], SocketAddrV6::new(ip, port, 0, 7))
            .await;
        assert_eq!(table.resolve_by_addr(ip, port).await, None);
    }

    assert_eq!(table.len().await, 2);
    assert_eq!(table.resolve_by_addr(ip, 10_000).await, None);
}

#[tokio::test]
async fn wrong_destination_vmac_ack_is_not_learned() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
    let _rx = transport.start().await.unwrap();
    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let destination = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    let sender = UdpSocket::bind("[::1]:0").await.unwrap();
    let sender_addr = match sender.local_addr().unwrap() {
        std::net::SocketAddr::V6(address) => address,
        _ => unreachable!(),
    };
    let foreign_destination = [9, 9, 9];

    let ar_source = [1, 2, 3];
    sender
        .send_to(
            &encode_address_resolution_ack(&ar_source, &foreign_destination),
            destination,
        )
        .await
        .unwrap();
    let var_source = [4, 5, 6];
    sender
        .send_to(
            &encode_virtual_address_resolution_ack(&var_source, &foreign_destination),
            destination,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(transport.vmac_table.lookup(&ar_source).await, None);
    assert_eq!(transport.vmac_table.lookup(&var_source).await, None);
    assert_eq!(
        transport
            .vmac_table
            .resolve_by_addr(*sender_addr.ip(), sender_addr.port())
            .await,
        None
    );

    transport.stop().await.unwrap();
}

#[test]
fn encode_original_unicast() {
    let src_vmac: Bip6Vmac = [0x01, 0x02, 0x03];
    let dst_vmac: Bip6Vmac = [0x0A, 0x0B, 0x0C];
    let npdu = vec![0x01, 0x00, 0xAA];
    let mut buf = BytesMut::new();
    encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu)
        .expect("valid BVLC6 unicast encoding");
    assert_eq!(buf[0], BVLC6_TYPE);
    assert_eq!(buf[1], Bvlc6Function::OriginalUnicast.to_byte());
    let len = u16::from_be_bytes([buf[2], buf[3]]);
    assert_eq!(len as usize, BVLC6_UNICAST_HEADER_LENGTH + npdu.len());
    assert_eq!(&buf[4..7], &src_vmac);
    assert_eq!(&buf[7..10], &dst_vmac);
    assert_eq!(&buf[10..], &npdu[..]);
}

#[test]
fn encode_original_broadcast() {
    let vmac: Bip6Vmac = [0x01; 3];
    let npdu = vec![0xBB];
    let mut buf = BytesMut::new();
    encode_bvlc6_original_broadcast(&mut buf, &vmac, &npdu)
        .expect("valid BVLC6 broadcast encoding");
    assert_eq!(buf[1], Bvlc6Function::OriginalBroadcast.to_byte());
}

#[test]
fn encode_bvlc6_oversized_payload_errors() {
    let vmac: Bip6Vmac = [0x01; 3];
    let npdu = vec![0; u16::MAX as usize - BVLC6_HEADER_LENGTH + 1];
    let mut buf = BytesMut::new();
    assert!(encode_bvlc6(&mut buf, Bvlc6Function::OriginalBroadcast, &vmac, &npdu).is_err());
}

#[test]
fn encode_bvlc6_unicast_oversized_payload_errors() {
    let src_vmac: Bip6Vmac = [0x01, 0x02, 0x03];
    let dst_vmac: Bip6Vmac = [0x0A, 0x0B, 0x0C];
    let npdu = vec![0; u16::MAX as usize - BVLC6_UNICAST_HEADER_LENGTH + 1];
    let mut buf = BytesMut::new();
    assert!(encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu).is_err());
}

#[test]
fn decode_round_trip_unicast() {
    let src_vmac: Bip6Vmac = [0x01, 0x02, 0x03];
    let dst_vmac: Bip6Vmac = [0x0A, 0x0B, 0x0C];
    let npdu = vec![0x01, 0x00, 0xAA, 0xBB];
    let mut buf = BytesMut::new();
    encode_bvlc6_original_unicast(&mut buf, &src_vmac, &dst_vmac, &npdu)
        .expect("valid BVLC6 unicast encoding");
    let decoded = decode_bvlc6(&buf).unwrap();
    assert_eq!(decoded.function, Bvlc6Function::OriginalUnicast);
    assert_eq!(decoded.source_vmac, src_vmac);
    assert_eq!(decoded.destination_vmac, Some(dst_vmac));
    assert_eq!(decoded.payload, npdu);
}

#[test]
fn decode_rejects_short_frame() {
    assert!(decode_bvlc6(&[0x82, 0x01]).is_err());
}

#[test]
fn decode_rejects_wrong_type() {
    assert!(decode_bvlc6(&[0x81, 0x01, 0x00, 0x07, 0, 0, 0]).is_err());
}

#[test]
fn function_round_trip() {
    for byte in 0x00..=0x0Cu8 {
        let f = Bvlc6Function::from_byte(byte);
        assert_eq!(f.to_byte(), byte);
    }
}

#[test]
fn bip6_mac_round_trip() {
    let ip = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
    let port = 47808u16;
    let mac = encode_bip6_mac(ip, port);
    assert_eq!(mac.len(), 18);
    let (decoded_ip, decoded_port) = decode_bip6_mac(&mac).unwrap();
    assert_eq!(decoded_ip, ip);
    assert_eq!(decoded_port, port);
}

#[test]
fn bip6_mac_rejects_wrong_length() {
    assert!(decode_bip6_mac(&[0; 6]).is_err());
    assert!(decode_bip6_mac(&[0; 20]).is_err());
}

#[test]
fn bip6_max_apdu_length() {
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    assert_eq!(transport.max_apdu_length(), 1476);
}

#[tokio::test]
async fn bip6_start_stop() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1));
    let _rx = transport.start().await.unwrap();
    assert!(transport.socket.is_some());
    assert_eq!(transport.local_mac().len(), 18);
    transport.stop().await.unwrap();
    assert!(transport.socket.is_none());
}

#[tokio::test]
async fn bip6_unicast_loopback() {
    let mut transport_a = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(2));
    let mut transport_b = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(3));

    let _rx_a = transport_a.start().await.unwrap();
    let mut rx_b = transport_b.start().await.unwrap();

    let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD];

    let error = transport_a
        .send_unicast(&test_npdu, transport_b.local_mac())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("destination VMAC is unknown"));

    let (peer_ip, peer_port) = decode_bip6_mac(transport_b.local_mac()).unwrap();
    transport_a
        .vmac_table
        .learn(
            transport_b.source_vmac,
            SocketAddrV6::new(peer_ip, peer_port, 0, 0),
        )
        .await;

    transport_a
        .send_unicast(&test_npdu, transport_b.local_mac())
        .await
        .unwrap();

    let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx_b.recv())
        .await
        .expect("timeout")
        .expect("channel closed");

    assert_eq!(received.npdu, test_npdu);
    assert_eq!(received.source_mac.as_slice(), transport_a.local_mac());

    transport_a.stop().await.unwrap();
    transport_b.stop().await.unwrap();
}

// --- Virtual Address Resolution tests ---

#[test]
fn encode_decode_virtual_address_resolution() {
    let vmac: Bip6Vmac = [0xAA, 0xBB, 0xCC];
    let buf = encode_virtual_address_resolution(&vmac);

    // VAR is 7 bytes: type(1) + function(1) + length(2) + source_vmac(3)
    assert_eq!(buf.len(), BVLC6_HEADER_LENGTH);
    assert_eq!(buf[0], BVLC6_TYPE);
    assert_eq!(buf[1], Bvlc6Function::VirtualAddressResolution.to_byte());
    let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    assert_eq!(total_len, BVLC6_HEADER_LENGTH);
    assert_eq!(&buf[4..7], &vmac);

    let frame = decode_bvlc6(&buf).unwrap();
    assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolution);
    assert_eq!(frame.source_vmac, vmac);
    assert!(frame.payload.is_empty());
}

#[test]
fn encode_decode_virtual_address_resolution_ack() {
    let source: Bip6Vmac = [0x11, 0x22, 0x33];
    let dest: Bip6Vmac = [0x44, 0x55, 0x66];
    let buf = encode_virtual_address_resolution_ack(&source, &dest);

    // VAR-ACK is 10 bytes: type(1)+function(1)+length(2)+source(3)+dest(3)
    assert_eq!(buf.len(), BVLC6_UNICAST_HEADER_LENGTH);
    assert_eq!(buf[0], BVLC6_TYPE);
    assert_eq!(buf[1], Bvlc6Function::VirtualAddressResolutionAck.to_byte());
    let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    assert_eq!(total_len, BVLC6_UNICAST_HEADER_LENGTH);
    assert_eq!(&buf[4..7], &source);
    assert_eq!(&buf[7..10], &dest);

    let frame = decode_bvlc6(&buf).unwrap();
    assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolutionAck);
    assert_eq!(frame.source_vmac, source);
    assert_eq!(frame.destination_vmac, Some(dest));
    assert!(frame.payload.is_empty());
}

#[test]
fn encode_decode_address_resolution() {
    let source: Bip6Vmac = [0x01, 0x02, 0x03];
    let target: Bip6Vmac = [0x04, 0x05, 0x06];
    let buf = encode_address_resolution(&source, &target);

    assert_eq!(buf.len(), BVLC6_UNICAST_HEADER_LENGTH);
    let frame = decode_bvlc6(&buf).unwrap();
    assert_eq!(frame.function, Bvlc6Function::AddressResolution);
    assert_eq!(frame.source_vmac, source);
    assert_eq!(frame.destination_vmac, Some(target));
}

#[test]
fn encode_decode_address_resolution_ack() {
    let source: Bip6Vmac = [0x0A, 0x0B, 0x0C];
    let dest: Bip6Vmac = [0x0D, 0x0E, 0x0F];
    let buf = encode_address_resolution_ack(&source, &dest);

    assert_eq!(buf.len(), BVLC6_UNICAST_HEADER_LENGTH);
    let frame = decode_bvlc6(&buf).unwrap();
    assert_eq!(frame.function, Bvlc6Function::AddressResolutionAck);
    assert_eq!(frame.source_vmac, source);
    assert_eq!(frame.destination_vmac, Some(dest));
}

#[test]
fn fixed_length_bvlc6_frames_reject_surplus_bytes() {
    let mut var = encode_virtual_address_resolution(&[1, 2, 3]);
    var.extend_from_slice(&[0]);
    assert!(decode_bvlc6(&var).is_err());

    let mut ar = encode_address_resolution(&[1, 2, 3], &[4, 5, 6]);
    ar.extend_from_slice(&[0]);
    let ar_len = ar.len() as u16;
    ar[2..4].copy_from_slice(&ar_len.to_be_bytes());
    assert!(decode_bvlc6(&ar).is_err());

    for (function, payload) in [
        (Bvlc6Function::Result, vec![0; 2]),
        (Bvlc6Function::RegisterForeignDevice, vec![0; 2]),
        (Bvlc6Function::DeleteForeignDeviceEntry, vec![0; 18]),
    ] {
        let mut frame = BytesMut::new();
        encode_bvlc6(&mut frame, function, &[1, 2, 3], &payload).unwrap();
        assert!(decode_bvlc6(&frame).is_ok());
        frame.extend_from_slice(&[0]);
        let length = frame.len() as u16;
        frame[2..4].copy_from_slice(&length.to_be_bytes());
        assert!(decode_bvlc6(&frame).is_err(), "{function:?}");
    }
}

#[test]
fn bvlc6_declared_length_must_match_datagram() {
    let mut frame = encode_virtual_address_resolution(&[1, 2, 3]);
    frame[2..4].copy_from_slice(&6u16.to_be_bytes());
    assert!(decode_bvlc6(&frame).is_err());
}

#[test]
fn vmac_from_valid_device_instance_uses_all_22_bits() {
    let vmac = derive_vmac_from_device_instance(0x123456);
    assert_eq!(vmac, [0x12, 0x34, 0x56]);
    assert_eq!(
        derive_vmac_from_device_instance(0x3F_FFFF),
        [0x3F, 0xFF, 0xFF]
    );
}

// --- ForwardedNpdu tests ---

#[test]
fn decode_forwarded_npdu_extracts_npdu() {
    let source_ip = Ipv6Addr::LOCALHOST;
    let source_port: u16 = 47808;
    let npdu_data = vec![0x01, 0x00, 0xFF, 0xEE];
    let mut payload = source_ip.octets().to_vec();
    payload.extend_from_slice(&source_port.to_be_bytes());
    payload.extend_from_slice(&npdu_data);

    let (addr, npdu) = decode_forwarded_npdu_payload(&payload).unwrap();
    assert_eq!(*addr.ip(), source_ip);
    assert_eq!(addr.port(), source_port);
    assert_eq!(npdu, &npdu_data[..]);
}

#[test]
fn decode_forwarded_npdu_rejects_short_payload() {
    assert!(decode_forwarded_npdu_payload(&[0x01; 17]).is_err());
    assert!(decode_forwarded_npdu_payload(&[0x01; 18]).is_err());
    assert!(decode_forwarded_npdu_payload(&[0x01; 19]).is_err());
    assert!(decode_forwarded_npdu_payload(&[]).is_err());
}

#[test]
fn forwarded_npdu_encode_decode_round_trip() {
    // Build a full ForwardedNpdu BVLC6 frame and decode it
    let originating_vmac: Bip6Vmac = [0xAA, 0xBB, 0xCC];
    let source_ip = Ipv6Addr::LOCALHOST;
    let npdu = vec![0x01, 0x00, 0xDE, 0xAD];

    let mut forwarded_payload = source_ip.octets().to_vec();
    forwarded_payload.extend_from_slice(&47808u16.to_be_bytes());
    forwarded_payload.extend_from_slice(&npdu);

    let mut buf = BytesMut::new();
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::ForwardedNpdu,
        &originating_vmac,
        &forwarded_payload,
    )
    .expect("valid BVLC6 encoding");

    let frame = decode_bvlc6(&buf).unwrap();
    assert_eq!(frame.function, Bvlc6Function::ForwardedNpdu);
    assert_eq!(frame.source_vmac, originating_vmac);

    let (addr, extracted_npdu) = decode_forwarded_npdu_payload(&frame.payload).unwrap();
    assert_eq!(*addr.ip(), source_ip);
    assert_eq!(extracted_npdu, &npdu[..]);
}

#[tokio::test]
async fn bip6_forwarded_npdu_delivered() {
    // Verify a foreign device accepts a ForwardedNpdu only from its BBMD.
    let sender = UdpSocket::bind("[::1]:0").await.unwrap();
    let sender_addr = match sender.local_addr().unwrap() {
        std::net::SocketAddr::V6(address) => address,
        _ => unreachable!(),
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(44));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: *sender_addr.ip(),
        bbmd_port: sender_addr.port(),
        ttl: 60,
    });
    let mut rx = transport.start().await.unwrap();

    // Build a ForwardedNpdu frame from a "BBMD"
    let originating_vmac: Bip6Vmac = [0xAA, 0xAA, 0xAA];
    let test_npdu = vec![0x01, 0x00, 0xCA, 0xFE];

    let origin_addr =
        SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9), 47_808, 0, 0);

    let mut forwarded_payload = origin_addr.ip().octets().to_vec();
    forwarded_payload.extend_from_slice(&origin_addr.port().to_be_bytes());
    forwarded_payload.extend_from_slice(&test_npdu);

    let mut buf = BytesMut::new();
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::ForwardedNpdu,
        &originating_vmac,
        &forwarded_payload,
    )
    .expect("valid BVLC6 encoding");

    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);

    let malformed_vmac = [0x41, 0x00, 0x01];
    let mut malformed_payload = origin_addr.ip().octets().to_vec();
    malformed_payload.extend_from_slice(&origin_addr.port().to_be_bytes());
    malformed_payload.push(0);
    let mut malformed = BytesMut::new();
    encode_bvlc6(
        &mut malformed,
        Bvlc6Function::ForwardedNpdu,
        &malformed_vmac,
        &malformed_payload,
    )
    .unwrap();
    sender.send_to(&malformed, dest).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(transport.vmac_table.lookup(&malformed_vmac).await, None);

    let invalid_origins = [
        (
            [0x41, 0x00, 0x02],
            SocketAddrV6::new(
                Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
                origin_addr.port(),
                0,
                4,
            ),
        ),
        (
            [0x41, 0x00, 0x03],
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, origin_addr.port(), 0, 0),
        ),
        (
            [0x41, 0x00, 0x04],
            SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, origin_addr.port(), 0, 0),
        ),
        (
            [0x41, 0x00, 0x05],
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, origin_addr.port(), 0, 0),
        ),
        (
            [0x41, 0x00, 0x06],
            SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 10), 0, 0, 0),
        ),
    ];
    for (vmac, invalid_origin) in invalid_origins {
        let mut payload = invalid_origin.ip().octets().to_vec();
        payload.extend_from_slice(&invalid_origin.port().to_be_bytes());
        payload.extend_from_slice(&test_npdu);
        let mut frame = BytesMut::new();
        encode_bvlc6(&mut frame, Bvlc6Function::ForwardedNpdu, &vmac, &payload).unwrap();
        sender.send_to(&frame, dest).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
    for (vmac, _) in invalid_origins {
        assert_eq!(transport.vmac_table.lookup(&vmac).await, None);
    }

    sender.send_to(&buf, dest).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");

    assert_eq!(received.npdu, test_npdu);
    assert_eq!(
        received.source_mac.as_slice(),
        &encode_bip6_mac(*origin_addr.ip(), origin_addr.port())
    );
    assert!(received.link_layer_group);

    assert_eq!(
        transport.vmac_table.lookup(&originating_vmac).await,
        Some(origin_addr)
    );

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bip6_var_response() {
    // A unicast VAR asks the destination node to disclose its VMAC.
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
    let _rx = transport.start().await.unwrap();
    let our_vmac = transport.source_vmac;
    let requester_vmac = [0x41, 0x22, 0x33];

    let buf = encode_virtual_address_resolution(&requester_vmac);

    // Send VAR to the transport
    let checker = UdpSocket::bind("[::1]:0").await.unwrap();
    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    checker.send_to(&buf, dest).await.unwrap();

    // We should receive a VAR-Ack back (confirming collision)
    let mut resp_buf = vec![0u8; 64];
    let result =
        tokio::time::timeout(Duration::from_secs(2), checker.recv_from(&mut resp_buf)).await;

    match result {
        Ok(Ok((len, _))) => {
            let frame = decode_bvlc6(&resp_buf[..len]).unwrap();
            assert_eq!(frame.function, Bvlc6Function::VirtualAddressResolutionAck);
            assert_eq!(frame.source_vmac, our_vmac);
            assert_eq!(frame.destination_vmac, Some(requester_vmac));
        }
        Ok(Err(e)) => panic!("recv error: {e}"),
        Err(_) => panic!("timeout waiting for VAR-Ack response"),
    }

    transport.stop().await.unwrap();
}

/// Verify that local `Bvlc6Function` byte values match `bacnet_types::Bvlc6Function`.
#[test]
fn bvlc6_function_codes_match_types_crate() {
    use bacnet_types::enums::Bvlc6Function as TypesBvlc6;

    let expected: &[(u8, &str)] = &[
        (0x00, "BVLC_RESULT"),
        (0x01, "ORIGINAL_UNICAST_NPDU"),
        (0x02, "ORIGINAL_BROADCAST_NPDU"),
        (0x03, "ADDRESS_RESOLUTION"),
        (0x04, "FORWARDED_ADDRESS_RESOLUTION"),
        (0x05, "ADDRESS_RESOLUTION_ACK"),
        (0x06, "VIRTUAL_ADDRESS_RESOLUTION"),
        (0x07, "VIRTUAL_ADDRESS_RESOLUTION_ACK"),
        (0x08, "FORWARDED_NPDU"),
        (0x09, "REGISTER_FOREIGN_DEVICE"),
        (0x0A, "DELETE_FOREIGN_DEVICE_TABLE_ENTRY"),
        (0x0C, "DISTRIBUTE_BROADCAST_TO_NETWORK"),
    ];

    for &(byte, _name) in expected {
        let local = Bvlc6Function::from_byte(byte);
        let types_val = TypesBvlc6::from_raw(byte);
        assert_eq!(
            local.to_byte(),
            types_val.to_raw(),
            "Mismatch at 0x{byte:02X}: bip6.rs={}, enums.rs={}",
            local.to_byte(),
            types_val.to_raw(),
        );
    }

    // Verify 0x0C is Distribute-Broadcast-To-Network (not the old SECURE_BVLL)
    assert_eq!(
        Bvlc6Function::DistributeBroadcastToNetwork.to_byte(),
        TypesBvlc6::DISTRIBUTE_BROADCAST_TO_NETWORK.to_raw(),
    );
    // 0x0B should decode as Unknown since it's removed
    assert!(matches!(
        Bvlc6Function::from_byte(0x0B),
        Bvlc6Function::Unknown(0x0B)
    ));
}

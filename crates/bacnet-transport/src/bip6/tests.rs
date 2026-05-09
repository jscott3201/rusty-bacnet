use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;

use super::port::derive_vmac_from_device_instance;
use super::*;
use crate::port::TransportPort;

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
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    let _rx = transport.start().await.unwrap();
    assert!(transport.socket.is_some());
    assert_eq!(transport.local_mac().len(), 18);
    transport.stop().await.unwrap();
    assert!(transport.socket.is_none());
}

#[tokio::test]
async fn bip6_unicast_loopback() {
    let mut transport_a = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    let mut transport_b = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);

    let _rx_a = transport_a.start().await.unwrap();
    let mut rx_b = transport_b.start().await.unwrap();

    let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD];

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
fn vmac_from_device_instance_masks_to_22_bits() {
    let vmac = derive_vmac_from_device_instance(0x123456);
    assert_eq!(vmac, [0x12, 0x34, 0x56]);
    // Value > 22 bits — upper bits should be masked off
    let vmac = derive_vmac_from_device_instance(0xFFFFFFFF);
    assert_eq!(vmac, [0x3F, 0xFF, 0xFF]);
}

// --- ForwardedNpdu tests ---

#[test]
fn decode_forwarded_npdu_extracts_npdu() {
    let originating_vmac: Bip6Vmac = [0xDE, 0xAD, 0x01];
    let source_ip = Ipv6Addr::LOCALHOST;
    let source_port: u16 = 47808;
    let npdu_data = vec![0x01, 0x00, 0xFF, 0xEE];
    let mut payload = originating_vmac.to_vec();
    payload.extend_from_slice(&source_ip.octets());
    payload.extend_from_slice(&source_port.to_be_bytes());
    payload.extend_from_slice(&npdu_data);

    let (vmac, addr, npdu) = decode_forwarded_npdu_payload(&payload).unwrap();
    assert_eq!(vmac, originating_vmac);
    assert_eq!(*addr.ip(), source_ip);
    assert_eq!(addr.port(), source_port);
    assert_eq!(npdu, &npdu_data[..]);
}

#[test]
fn decode_forwarded_npdu_rejects_short_payload() {
    assert!(decode_forwarded_npdu_payload(&[0x01; 20]).is_err());
    assert!(decode_forwarded_npdu_payload(&[]).is_err());
}

#[test]
fn decode_forwarded_npdu_vmac_and_addr_only_is_ok() {
    // Exactly 21 bytes = VMAC + B/IPv6 address with empty NPDU
    let mut payload = vec![0x01, 0x02, 0x03]; // vmac
    payload.extend_from_slice(&Ipv6Addr::LOCALHOST.octets()); // 16 bytes
    payload.extend_from_slice(&47808u16.to_be_bytes()); // 2 bytes
    let (vmac, _addr, npdu) = decode_forwarded_npdu_payload(&payload).unwrap();
    assert_eq!(vmac, [0x01, 0x02, 0x03]);
    assert!(npdu.is_empty());
}

#[test]
fn forwarded_npdu_encode_decode_round_trip() {
    // Build a full ForwardedNpdu BVLC6 frame and decode it
    let sender_vmac: Bip6Vmac = [0x10, 0x20, 0x30];
    let originating_vmac: Bip6Vmac = [0xAA, 0xBB, 0xCC];
    let source_ip = Ipv6Addr::LOCALHOST;
    let npdu = vec![0x01, 0x00, 0xDE, 0xAD];

    let mut forwarded_payload = originating_vmac.to_vec();
    forwarded_payload.extend_from_slice(&source_ip.octets());
    forwarded_payload.extend_from_slice(&47808u16.to_be_bytes());
    forwarded_payload.extend_from_slice(&npdu);

    let mut buf = BytesMut::new();
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::ForwardedNpdu,
        &sender_vmac,
        &forwarded_payload,
    )
    .expect("valid BVLC6 encoding");

    let frame = decode_bvlc6(&buf).unwrap();
    assert_eq!(frame.function, Bvlc6Function::ForwardedNpdu);
    assert_eq!(frame.source_vmac, sender_vmac);

    let (orig_vmac, addr, extracted_npdu) = decode_forwarded_npdu_payload(&frame.payload).unwrap();
    assert_eq!(orig_vmac, originating_vmac);
    assert_eq!(*addr.ip(), source_ip);
    assert_eq!(extracted_npdu, &npdu[..]);
}

#[tokio::test]
async fn bip6_forwarded_npdu_delivered() {
    // Verify that a ForwardedNpdu sent to a transport is delivered as a ReceivedNpdu
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    let mut rx = transport.start().await.unwrap();

    // Build a ForwardedNpdu frame from a "BBMD"
    let bbmd_vmac: Bip6Vmac = [0xBB, 0xBB, 0xBB];
    let originating_vmac: Bip6Vmac = [0xAA, 0xAA, 0xAA];
    let test_npdu = vec![0x01, 0x00, 0xCA, 0xFE];

    let mut forwarded_payload = originating_vmac.to_vec();
    forwarded_payload.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
    forwarded_payload.extend_from_slice(&47808u16.to_be_bytes());
    forwarded_payload.extend_from_slice(&test_npdu);

    let mut buf = BytesMut::new();
    encode_bvlc6(
        &mut buf,
        Bvlc6Function::ForwardedNpdu,
        &bbmd_vmac,
        &forwarded_payload,
    )
    .expect("valid BVLC6 encoding");

    // Send directly to the transport's bound address using a separate socket
    let sender = UdpSocket::bind("[::1]:0").await.unwrap();
    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    sender.send_to(&buf, dest).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");

    assert_eq!(received.npdu, test_npdu);
    // source_mac must be the originating VMAC (3 bytes), not the UDP sender address
    assert_eq!(received.source_mac.as_slice(), &originating_vmac[..]);

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bip6_var_response() {
    // Verify that receiving a VAR from a node with the same VMAC
    // causes a VAR-Ack response (collision detection per Clause U.5).
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
    let _rx = transport.start().await.unwrap();
    let our_vmac = transport.source_vmac;

    // Build a VAR frame from a node claiming our VMAC (collision scenario)
    let buf = encode_virtual_address_resolution(&our_vmac);

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
            // destination_vmac should be the querier's VMAC (same as ours)
            assert_eq!(frame.destination_vmac, Some(our_vmac));
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

#[test]
fn generate_random_vmac_produces_3_bytes() {
    let vmac = generate_random_vmac();
    assert_eq!(vmac.len(), 3);
}

#[test]
fn generate_random_vmac_is_nondeterministic() {
    // Generate several VMACs — at least two should differ.
    let vmacs: Vec<Bip6Vmac> = (0..10).map(|_| generate_random_vmac()).collect();
    let all_same = vmacs.windows(2).all(|w| w[0] == w[1]);
    assert!(!all_same, "10 random VMACs should not all be identical");
}

#[test]
fn max_vmac_retries_constant() {
    const { assert!(MAX_VMAC_RETRIES >= 1, "must allow at least one retry") };
}

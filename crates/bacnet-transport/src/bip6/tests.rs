use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;

use super::frame::encoded;
use super::ingress::{destination_vmac_matches, original_destination_matches, LocalBinding};
use super::*;
use crate::port::TransportPort;

fn vmac(octets: [u8; 3]) -> Bip6Vmac {
    Bip6Vmac::new(octets)
}

/// `message` on the wire.
fn wire(message: Bvll6<'_>) -> BytesMut {
    encoded(&message).expect("a valid B/IPv6 message")
}

fn binding(
    ip: Ipv6Addr,
    vmac: Bip6Vmac,
    unicast_ips: &[Ipv6Addr],
    wildcard_bind: bool,
) -> LocalBinding<'_> {
    LocalBinding {
        ip,
        vmac,
        unicast_ips,
        wildcard_bind,
    }
}

#[test]
fn original_function_requires_matching_ip_destination_and_vmac() {
    let local_ip = Ipv6Addr::LOCALHOST;
    let local_vmac = vmac([1, 2, 3]);

    assert!(original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        local_ip.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        local_ip.into(),
        Some(vmac([9, 9, 9])),
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(original_destination_matches(
        Bvlc6Function::ORIGINAL_BROADCAST_NPDU,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        None,
        &binding(local_ip, local_vmac, &[local_ip], false),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ADDRESS_RESOLUTION_ACK,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(true),
    ));

    assert!(original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9).into(),
        Some(local_vmac),
        &binding(
            local_ip,
            local_vmac,
            &[local_ip, Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9),],
            true
        ),
        None,
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ORIGINAL_UNICAST_NPDU,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], true),
        Some(true),
    ));
    assert!(original_destination_matches(
        Bvlc6Function::ADDRESS_RESOLUTION,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(true),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::ADDRESS_RESOLUTION,
        local_ip.into(),
        Some(local_vmac),
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(false),
    ));
    assert!(original_destination_matches(
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
        local_ip.into(),
        None,
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(false),
    ));
    assert!(!original_destination_matches(
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        None,
        &binding(local_ip, local_vmac, &[local_ip], false),
        Some(true),
    ));
    // The BBMD requests this node refuses must come to its own address: a
    // group delivery would draw a NAK from every member.
    for function in [
        Bvlc6Function::REGISTER_FOREIGN_DEVICE,
        Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
        Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK,
    ] {
        assert!(
            original_destination_matches(
                function,
                local_ip.into(),
                None,
                &binding(local_ip, local_vmac, &[local_ip], false),
                Some(false),
            ),
            "{function:?}"
        );
        for (destination, os) in [
            (BACNET_IPV6_MULTICAST_SITE_LOCAL, None),
            (local_ip, Some(true)),
        ] {
            assert!(
                !original_destination_matches(
                    function,
                    destination.into(),
                    None,
                    &binding(local_ip, local_vmac, &[local_ip], false),
                    os,
                ),
                "{function:?} {destination} {os:?}"
            );
        }
    }
}

/// U.3: the three messages that name a recipient's VMAC pass only when it
/// is this node's. Address-Resolution's target is not a destination, and a
/// message that names none passes.
#[test]
fn destination_vmac_filter_admits_only_this_nodes_vmac() {
    let local = vmac([1, 2, 3]);
    let other = vmac([9, 9, 9]);
    let unicast = |destination| Bvll6::OriginalUnicastNpdu {
        source: other,
        destination,
        npdu: &[0x01, 0x00],
    };
    let ar_ack = |destination| Bvll6::AddressResolutionAck {
        source: other,
        destination,
    };
    let var_ack = |destination| Bvll6::VirtualAddressResolutionAck {
        source: other,
        destination,
    };
    for (message, admitted) in [
        (unicast(local), true),
        (unicast(other), false),
        (ar_ack(local), true),
        (ar_ack(other), false),
        (var_ack(local), true),
        (var_ack(other), false),
        (
            Bvll6::AddressResolution {
                source: other,
                target: other,
            },
            true,
        ),
        (
            Bvll6::OriginalBroadcastNpdu {
                source: other,
                npdu: &[0x01, 0x00],
            },
            true,
        ),
    ] {
        assert_eq!(
            destination_vmac_matches(&message, local),
            admitted,
            "{message:?}"
        );
    }
}

#[tokio::test]
async fn wrong_destination_vmac_ack_is_not_learned() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
    let _rx = transport.start().await.unwrap();
    let transport_port = transport.local_address().unwrap().port();
    let destination = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    let sender = UdpSocket::bind("[::1]:0").await.unwrap();
    let foreign_destination = vmac([9, 9, 9]);

    let ar_source = vmac([1, 2, 3]);
    sender
        .send_to(
            &wire(Bvll6::AddressResolutionAck {
                source: ar_source,
                destination: foreign_destination,
            }),
            destination,
        )
        .await
        .unwrap();
    let var_source = vmac([4, 5, 6]);
    sender
        .send_to(
            &wire(Bvll6::VirtualAddressResolutionAck {
                source: var_source,
                destination: foreign_destination,
            }),
            destination,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(transport.lookup_vmac(ar_source), None);
    assert_eq!(transport.lookup_vmac(var_source), None);

    transport.stop().await.unwrap();
}

#[test]
fn bip6_max_apdu_length() {
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    assert_eq!(transport.egress_apdu_limit(), 1476);
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
}

#[tokio::test]
async fn bip6_start_stop() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1));
    let _rx = transport.start().await.unwrap();
    assert!(transport.socket.is_some());
    // The MAC is the VMAC device instance 1 gives (U.1.3, H.7.2).
    assert_eq!(transport.local_mac(), [0x00, 0x00, 0x01]);
    assert_eq!(transport.local_vmac(), Some(vmac([0x00, 0x00, 0x01])));
    let address = transport.local_address().unwrap();
    assert_eq!(*address.ip(), Ipv6Addr::LOCALHOST);
    assert_ne!(address.port(), 0);
    transport.stop().await.unwrap();
    assert!(transport.socket.is_none());
    assert_eq!(transport.local_mac(), [0; 3]);
    assert_eq!(
        (transport.local_vmac(), transport.local_address()),
        (None, None)
    );
}

/// The MAC is the 3-octet VMAC (U.1.3): a node that found its peer's VMAC
/// from the peer's address sends to that VMAC, the frame arrives with the
/// sender's VMAC as its source MAC, and the peer answers that VMAC at once,
/// having learned it from the frame (U.5).
#[tokio::test]
async fn bip6_unicast_loopback() {
    let mut transport_a = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(2));
    let mut transport_b = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(3));

    let mut rx_a = transport_a.start().await.unwrap();
    let mut rx_b = transport_b.start().await.unwrap();

    let found = transport_a
        .resolve_virtual_address(transport_b.local_address().unwrap())
        .await
        .unwrap();
    assert_eq!(found.octets(), transport_b.local_mac());

    let test_npdu = vec![0x01, 0x00, 0xDE, 0xAD];
    transport_a
        .send_unicast(&test_npdu, transport_b.local_mac())
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(2), rx_b.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    assert_eq!(received.npdu, test_npdu);
    assert_eq!(received.source_mac.as_slice(), [0x00, 0x00, 0x02]);

    transport_b
        .send_unicast(&test_npdu, &received.source_mac)
        .await
        .unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(2), rx_a.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    assert_eq!(answer.source_mac.as_slice(), transport_b.local_mac());
    assert_eq!(transport_a.resolution_counters().resolutions_started, 0);

    transport_a.stop().await.unwrap();
    transport_b.stop().await.unwrap();
}

#[test]
fn vmac_from_valid_device_instance_uses_all_22_bits() {
    let vmac = vmac_from_device_instance(0x123456).unwrap();
    assert_eq!(vmac.octets(), [0x12, 0x34, 0x56]);
    assert_eq!(
        vmac_from_device_instance(0x3F_FFFF).unwrap().octets(),
        [0x3F, 0xFF, 0xFF]
    );
    assert!(vmac_from_device_instance(0x40_0000).is_err());
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
    let originating_vmac = vmac([0xAA, 0xAA, 0xAA]);
    let test_npdu = vec![0x01, 0x00, 0xCA, 0xFE];

    let origin_addr =
        SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9), 47_808, 0, 0);
    let forwarded = |source: Bip6Vmac, origin: SocketAddrV6, npdu: &[u8]| {
        wire(Bvll6::ForwardedNpdu {
            original_source: source,
            original_address: origin.into(),
            npdu,
        })
    };
    let buf = forwarded(originating_vmac, origin_addr, &test_npdu);

    let transport_port = transport.local_address().unwrap().port();
    let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);

    let malformed_vmac = vmac([0x41, 0x00, 0x01]);
    let malformed = forwarded(malformed_vmac, origin_addr, &[0]);
    sender.send_to(&malformed, dest).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(transport.lookup_vmac(malformed_vmac), None);

    let invalid_origins = [
        (
            vmac([0x41, 0x00, 0x02]),
            SocketAddrV6::new(
                Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
                origin_addr.port(),
                0,
                4,
            ),
        ),
        (
            vmac([0x41, 0x00, 0x03]),
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, origin_addr.port(), 0, 0),
        ),
        (
            vmac([0x41, 0x00, 0x04]),
            SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, origin_addr.port(), 0, 0),
        ),
        (
            vmac([0x41, 0x00, 0x05]),
            SocketAddrV6::new(Ipv6Addr::LOCALHOST, origin_addr.port(), 0, 0),
        ),
        (
            vmac([0x41, 0x00, 0x06]),
            SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 10), 0, 0, 0),
        ),
    ];
    for (vmac, invalid_origin) in invalid_origins {
        let frame = forwarded(vmac, invalid_origin, &test_npdu);
        sender.send_to(&frame, dest).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
    for (vmac, _) in invalid_origins {
        assert_eq!(transport.lookup_vmac(vmac), None);
    }

    sender.send_to(&buf, dest).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("channel closed");

    assert_eq!(received.npdu, test_npdu);
    // The origin's VMAC is the NPDU's source MAC.
    assert_eq!(received.source_mac.as_slice(), originating_vmac.octets());
    assert!(received.link_layer_group);

    assert_eq!(transport.lookup_vmac(originating_vmac), Some(origin_addr));

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn bip6_var_response() {
    // A unicast VAR asks the destination node to disclose its VMAC.
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(42));
    let _rx = transport.start().await.unwrap();
    let our_vmac = transport.source_vmac;
    let requester_vmac = vmac([0x41, 0x22, 0x33]);

    let buf = wire(Bvll6::VirtualAddressResolution {
        source: requester_vmac,
    });

    // Send VAR to the transport
    let checker = UdpSocket::bind("[::1]:0").await.unwrap();
    let transport_port = transport.local_address().unwrap().port();
    let dest = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    checker.send_to(&buf, dest).await.unwrap();

    // We should receive a VAR-Ack back (confirming collision)
    let mut resp_buf = vec![0u8; 64];
    let result =
        tokio::time::timeout(Duration::from_secs(2), checker.recv_from(&mut resp_buf)).await;

    match result {
        Ok(Ok((len, _))) => {
            let frame = decode_bvll6(&resp_buf[..len]).unwrap();
            assert_eq!(
                frame.function(),
                Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK
            );
            assert_eq!(frame.source_vmac(), our_vmac);
            assert_eq!(frame.destination_vmac(), Some(requester_vmac));
        }
        Ok(Err(e)) => panic!("recv error: {e}"),
        Err(_) => panic!("timeout waiting for VAR-Ack response"),
    }

    transport.stop().await.unwrap();
}

use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::UdpSocket;

use super::ingress::{forwarded_npdu_is_trusted, forwarded_source_is_usable};
use super::*;
use crate::port::TransportPort;

#[test]
fn forwarded_npdu_requires_multicast_or_configured_bbmd() {
    let local_ip = Ipv6Addr::LOCALHOST;
    let peer = SocketAddrV6::new(local_ip, 47_808, 0, 0);

    assert!(forwarded_npdu_is_trusted(
        peer.into(),
        BACNET_IPV6_MULTICAST_LINK_LOCAL.into(),
        Some(true),
        local_ip,
        &[local_ip],
        false,
        None,
    ));
    assert!(!forwarded_npdu_is_trusted(
        peer.into(),
        local_ip.into(),
        Some(false),
        local_ip,
        &[local_ip],
        false,
        None,
    ));
    assert!(forwarded_npdu_is_trusted(
        peer.into(),
        local_ip.into(),
        Some(false),
        local_ip,
        &[local_ip],
        false,
        Some((local_ip, 47_808)),
    ));
    assert!(!forwarded_npdu_is_trusted(
        SocketAddrV6::new(local_ip, 47_809, 0, 0).into(),
        local_ip.into(),
        Some(false),
        local_ip,
        &[local_ip],
        false,
        Some((local_ip, 47_808)),
    ));
    let link_local = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
    assert!(!forwarded_npdu_is_trusted(
        SocketAddrV6::new(link_local, 47_808, 0, 4).into(),
        local_ip.into(),
        Some(false),
        local_ip,
        &[local_ip],
        false,
        Some((link_local, 47_808)),
    ));
}

#[test]
fn forwarded_npdu_origin_must_be_a_usable_unicast_endpoint() {
    assert!(forwarded_source_is_usable(SocketAddrV6::new(
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9),
        47_808,
        0,
        0,
    )));
    for source in [
        SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 47_808, 0, 0),
        SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, 47_808, 0, 0),
        SocketAddrV6::new(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1), 47_808, 0, 4),
        SocketAddrV6::new(Ipv6Addr::LOCALHOST, 47_808, 0, 0),
        SocketAddrV6::new(
            Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xc000, 0x0201),
            47_808,
            0,
            0,
        ),
        SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9), 0, 0, 0),
    ] {
        assert!(!forwarded_source_is_usable(source), "{source}");
    }
}

#[tokio::test]
async fn device_zero_vmac_is_learned_and_used_for_reply() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(43));
    let mut rx = transport.start().await.unwrap();
    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let destination = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);
    let peer = UdpSocket::bind("[::1]:0").await.unwrap();
    let npdu = [0x01, 0x00, 0x10, 0x08];
    let mut frame = BytesMut::new();
    encode_bvlc6_original_unicast(&mut frame, &[0, 0, 0], &transport.source_vmac, &npdu).unwrap();
    peer.send_to(&frame, destination).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.as_ref(), npdu);
    transport
        .send_unicast(&[0x01, 0x00, 0x10, 0x09], received.source_mac.as_slice())
        .await
        .unwrap();

    let mut response = [0u8; 64];
    let (len, source) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.port(), transport_port);
    let response = decode_bvlc6(&response[..len]).unwrap();
    assert_eq!(response.destination_vmac, Some([0, 0, 0]));

    transport.stop().await.unwrap();
}

#[tokio::test]
async fn untrusted_forwarded_npdu_cannot_replace_learned_peer() {
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let bbmd_addr = match bbmd.local_addr().unwrap() {
        std::net::SocketAddr::V6(address) => address,
        _ => unreachable!(),
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(45));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: *bbmd_addr.ip(),
        bbmd_port: bbmd_addr.port(),
        ttl: 60,
    });
    let mut rx = transport.start().await.unwrap();
    let (_, transport_port) = decode_bip6_mac(transport.local_mac()).unwrap();
    let destination = SocketAddrV6::new(Ipv6Addr::LOCALHOST, transport_port, 0, 0);

    let origin = UdpSocket::bind("[::1]:0").await.unwrap();
    let origin_addr = match origin.local_addr().unwrap() {
        std::net::SocketAddr::V6(address) => address,
        _ => unreachable!(),
    };
    let legitimate_vmac = [0x41, 0x10, 0x01];
    let mut original = BytesMut::new();
    encode_bvlc6_original_unicast(
        &mut original,
        &legitimate_vmac,
        &transport.source_vmac,
        &[0x01, 0x00, 0x10, 0x08],
    )
    .unwrap();
    origin.send_to(&original, destination).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();

    let attacker = UdpSocket::bind("[::1]:0").await.unwrap();
    let forged_vmac = [0x41, 0x10, 0x02];
    let mut forwarded_payload = origin_addr.ip().octets().to_vec();
    forwarded_payload.extend_from_slice(&origin_addr.port().to_be_bytes());
    forwarded_payload.extend_from_slice(&[0x01, 0x00, 0x10, 0x09]);
    let mut forged = BytesMut::new();
    encode_bvlc6(
        &mut forged,
        Bvlc6Function::ForwardedNpdu,
        &forged_vmac,
        &forwarded_payload,
    )
    .unwrap();
    attacker.send_to(&forged, destination).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(transport.vmac_table.lookup(&forged_vmac).await, None);

    transport
        .send_unicast(&[0x01, 0x00, 0x10, 0x0A], received.source_mac.as_slice())
        .await
        .unwrap();
    let mut reply = [0u8; 64];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), origin.recv_from(&mut reply))
        .await
        .unwrap()
        .unwrap();
    let reply = decode_bvlc6(&reply[..len]).unwrap();
    assert_eq!(reply.destination_vmac, Some(legitimate_vmac));

    transport.stop().await.unwrap();
}

//! Explicit two-link external qualification; never runs against an inferred LAN.
#![cfg(all(feature = "ipv6", unix))]

#[path = "ipv6_selected_link/support.rs"]
mod support;
#[path = "ipv6_selected_link/wire.rs"]
mod wire;
use support::*;

use bacnet_transport::{
    bip6::{Bip6Transport, Bip6Vmac},
    port::TransportPort,
};
use std::net::{Ipv6Addr, SocketAddrV6};

#[tokio::test]
#[ignore = "requires two explicitly supplied task-owned internal IPv6 links"]
async fn two_links_explicit_selection_rejects_other_interface_and_local_destination() {
    let (selected, index) = fixture();
    let other: Ipv6Addr = std::env::var("RB_IPV6_OTHER_ADDRESS")
        .expect("second isolated ULA required")
        .parse()
        .unwrap();
    let other_index: u32 = std::env::var("RB_IPV6_OTHER_INDEX")
        .expect("second isolated index required")
        .parse()
        .unwrap();
    assert!(other.is_unique_local());
    assert_ne!(other, selected);
    assert_ne!(other_index, index);
    assert_ne!(other_index, 0);
    let mut ambiguous = Bip6Transport::new(Ipv6Addr::UNSPECIFIED, 0, None);
    let error = ambiguous.start().await.unwrap_err();
    assert!(
        matches!(error, bacnet_types::error::Error::Transport(e) if e.kind() == std::io::ErrorKind::AddrNotAvailable)
    );
    assert_eq!(ambiguous.local_mac(), &[0; 3]);

    let (observer, port) = observer(index, true);
    let mut transport = Bip6Transport::new(selected, port, None);
    let mut incoming = transport.start().await.unwrap();
    let local = transport.local_address().unwrap();
    let (announced, published) = (*local.ip(), local.port());
    assert_eq!((announced, published), (selected, port));
    observer.join_multicast_v6(&GROUP, other_index).unwrap();
    let peer = udp(selected, 0, index, false);
    let outsider = udp(other, 0, other_index, false);
    transport.send_broadcast(NPDU).await.unwrap();
    let sent = wire_broadcast(&observer, NPDU).await.unwrap();
    assert_eq!((*sent.source.ip(), sent.source.port()), (selected, port));
    assert_eq!((sent.destination, sent.index), (GROUP, index));
    let local_vmac = &sent.bytes[4..7];

    let bad_npdu = [1, 0, 0x10, 8, 0x09, 77, 0x19, 77];
    let mut bad_group = vec![0x82, 2, 0, 15, 0x40, 0x77, 1];
    bad_group.extend_from_slice(&bad_npdu);
    outsider
        .send_to(&bad_group, SocketAddrV6::new(GROUP, port, 0, other_index))
        .await
        .unwrap();
    let observed = wire_broadcast(&observer, &bad_npdu).await.unwrap();
    assert_eq!(observed.index, other_index);
    assert_eq!(*observed.source.ip(), other);
    // Remove the same-port observer before the unicast negative, so it cannot
    // consume the datagram instead of the production wildcard receiver.
    drop(observer);
    let mut bad_unicast = vec![0x82, 1, 0, 18, 0x40, 0x77, 1];
    bad_unicast.extend_from_slice(local_vmac);
    bad_unicast.extend_from_slice(&bad_npdu);
    outsider
        .send_to(&bad_unicast, SocketAddrV6::new(other, port, 0, 0))
        .await
        .unwrap();
    let mut good = vec![0x82, 2, 0, 11, 0x40, 0x88, 1];
    good.extend_from_slice(NPDU);
    peer.send_to(&good, SocketAddrV6::new(GROUP, port, 0, index))
        .await
        .unwrap();
    let admitted = tokio::time::timeout(DEADLINE, incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admitted.npdu.as_ref(), NPDU);
    assert_eq!(admitted.source_mac.as_slice(), [0x40, 0x88, 0x01]);
    assert_eq!(
        transport
            .lookup_vmac(Bip6Vmac::new([0x40, 0x88, 0x01]))
            .map(|peer| (*peer.ip(), peer.port())),
        Some((selected, peer.local_addr().unwrap().port()))
    );
    assert!(
        incoming.try_recv().is_err(),
        "off-link traffic reached NPDU admission"
    );
    // Nothing from the other link taught the table the outsider's VMAC.
    assert_eq!(
        transport.lookup_vmac(Bip6Vmac::new([0x40, 0x77, 0x01])),
        None
    );
    transport
        .send_unicast(NPDU, &admitted.source_mac)
        .await
        .unwrap();
    let reply = tokio::time::timeout(DEADLINE, wire::receive(&peer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((*reply.source.ip(), reply.source.port()), (selected, port));
    transport.stop().await.unwrap();
}

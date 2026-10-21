//! Opt-in actual-transport checks for an isolated, multicast-capable IPv6 link.
//!
//! Supply `RB_IPV6_TEST_ADDRESS` and `RB_IPV6_TEST_INDEX` from a task-owned
//! internal Docker bridge. These tests must not discover or use a host LAN.
// The packet-info oracle (wire.rs) needs nix's decoded IPV6_PKTINFO, which
// it gives only on these targets.
#![cfg(all(
    feature = "ipv6",
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_vendor = "apple"
    )
))]
#![forbid(unsafe_code)]
#![allow(clippy::print_stderr)] // opt-in checks report skipped setups outside a test function

#[path = "ipv6_selected_link/controls.rs"]
mod controls;
#[path = "ipv6_selected_link/foreign.rs"]
mod foreign;
#[path = "ipv6_selected_link/lifecycle.rs"]
mod lifecycle;
// A closing rebind can lose the freed port to another process (#1419).
#[path = "../../bacnet-endpoint/tests/support/port_retry.rs"]
mod port_retry;
#[path = "ipv6_selected_link/support.rs"]
mod support;
use support::*;
#[path = "ipv6_selected_link/wire.rs"]
mod wire;

use std::net::{Ipv6Addr, SocketAddrV6};

use bacnet_transport::bip6::{Bip6Transport, Bip6Vmac};
use bacnet_transport::port::TransportPort;
use tokio::net::UdpSocket;

async fn actual_transport_selected_link(automatic: bool) {
    let (selected, index) = fixture();
    let requested = if automatic {
        Ipv6Addr::UNSPECIFIED
    } else {
        selected
    };
    // Random startup exercises the production collision probe; no configured
    // Device instance or foreign-device mode is used to bypass that exchange.
    let (observer, port) = observer(index, true);
    let mut transport = Bip6Transport::new(requested, port, None);
    let mut incoming = tokio::time::timeout(DEADLINE, transport.start())
        .await
        .expect("actual transport startup exceeded its bounded probe")
        .expect("actual transport random-VMAC startup must succeed");
    let local = transport.local_address().unwrap();
    let (announced, published) = (*local.ip(), local.port());
    assert_eq!(
        published, port,
        "the transport must publish its actual bound port"
    );
    let peer = udp(selected, 0, index, false);
    let peer_address = peer.local_addr().unwrap();
    let mut failures = Vec::new();
    if announced != selected {
        failures.push(format!(
            "announced {announced}, expected selected {selected}"
        ));
    }

    let send = transport.send_broadcast(NPDU).await;
    let outgoing = if send.is_ok() {
        wire_broadcast(&observer, NPDU).await
    } else {
        Err(format!("actual transport broadcast send failed: {send:?}"))
    };
    if let Ok(frame) = &outgoing {
        let bytes = &frame.bytes;
        let source = &frame.source;
        assert_eq!(frame.destination, GROUP);
        assert_eq!(frame.index, index);
        assert_eq!(
            u16::from_be_bytes([bytes[2], bytes[3]]) as usize,
            bytes.len()
        );
        assert_eq!(bytes[4] & 0xc0, 0x40, "expected random Device VMAC");
        if *source.ip() != selected || source.port() != port || *source.ip() != announced {
            failures.push(format!(
                "wire source {source} disagrees with selected/announced identity"
            ));
        }
    } else {
        failures.push(outgoing.as_ref().unwrap_err().clone());
    }

    // Independent Annex U Original-Broadcast bytes; do not use the production
    // encoder as the oracle for this transport regression.
    let wire = [
        0x82, 0x02, 0, 15, 0x40, 0x88, 0x70, 1, 0, 0x10, 0x08, 0x09, 1, 0x19, 1,
    ];
    peer.send_to(&wire, SocketAddrV6::new(GROUP, port, 0, index))
        .await
        .unwrap();
    let observed_peer = wire_broadcast(&observer, &wire[7..]).await.unwrap();
    assert_eq!(observed_peer.bytes, wire);
    assert_eq!(observed_peer.destination, GROUP);
    assert_eq!(observed_peer.index, index);
    assert_eq!(std::net::SocketAddr::V6(observed_peer.source), peer_address);
    let admitted = tokio::time::timeout(DEADLINE, async {
        while let Some(npdu) = incoming.recv().await {
            if npdu.npdu.as_ref() == &wire[7..] {
                return Some(npdu);
            }
        }
        None
    })
    .await;
    if let Ok(Some(npdu)) = &admitted {
        assert_eq!(npdu.npdu.as_ref(), &wire[7..]);
        assert!(npdu.link_layer_group);
        transport
            .send_unicast(NPDU, &npdu.source_mac)
            .await
            .unwrap();
        let reply = tokio::time::timeout(DEADLINE, wire::receive(&peer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&reply.bytes[..2], &[0x82, 0x01]);
        assert_eq!(&reply.bytes[7..10], &[0x40, 0x88, 0x70]);
        assert_eq!(&reply.bytes[10..], NPDU);
        assert_eq!((*reply.source.ip(), reply.source.port()), (selected, port));
        assert_eq!(reply.destination, selected);

        // The source MAC is the peer's VMAC (U.1.3), learned at its address.
        assert_eq!(npdu.source_mac.as_slice(), [0x40, 0x88, 0x70]);
        assert_eq!(
            transport
                .lookup_vmac(Bip6Vmac::new([0x40, 0x88, 0x70]))
                .map(|peer| (*peer.ip(), peer.port())),
            Some((selected, peer_address.port()))
        );
    } else {
        failures.push("raw peer multicast reached observer but not actual transport intake".into());
    }
    eprintln!(
        "requested={requested} selected={selected}%{index} announced=[{announced}]:{port} outgoing={outgoing:?} admitted={admitted:?} failures={failures:?}"
    );
    transport.stop().await.unwrap();
    assert!(
        failures.is_empty(),
        "selected-link contract failures: {failures:?}"
    );
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 multicast link"]
async fn automatic_selected_link_identity_broadcast_and_receive() {
    actual_transport_selected_link(true).await;
}

#[tokio::test]
#[ignore = "requires an explicitly supplied isolated IPv6 multicast link"]
async fn explicit_selected_link_identity_broadcast_and_receive() {
    actual_transport_selected_link(false).await;
}

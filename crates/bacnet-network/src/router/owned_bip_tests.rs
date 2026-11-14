//! One-shot exclusive rebind supplements the causal child-owner regressions.

use super::{BACnetRouter, RouterOptions, RouterPort};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::test_transport::{SendMode, TestTransport};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Mutex;
use tokio::sync::oneshot;

/// A B/IP port that reports its bound address once started, and takes the
/// startup announcements without sending datagrams (broadcasts stop here).
fn observed(inner: BipTransport, bound: oneshot::Sender<SocketAddrV4>) -> TestTransport {
    let bound = Mutex::new(Some(bound));
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Ignore)
        .on_started(move |started| {
            let mac = started.local_mac;
            let address = SocketAddrV4::new(
                Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]),
                u16::from_be_bytes([mac[4], mac[5]]),
            );
            bound.lock().unwrap().take().unwrap().send(address).unwrap();
        })
        .build()
}

async fn rebind_after_stop(bbmd: bool) {
    let mut inner = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    if bbmd {
        inner.enable_bbmd(Vec::new());
    }
    let (bound, address) = oneshot::channel();
    let port = RouterPort {
        network_number: 1,
        transport: observed(inner, bound),
    };
    let mut started = BACnetRouter::start(vec![port], RouterOptions::new())
        .await
        .unwrap();
    let address = address.await.unwrap();
    assert_ne!(address.port(), 0);
    let snapshots = started.router.ports().to_vec();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        (snapshots[0].index(), snapshots[0].network_number()),
        (0, 1)
    );
    let mut expected_mac = address.ip().octets().to_vec();
    expected_mac.extend_from_slice(&address.port().to_be_bytes());
    assert_eq!(snapshots[0].local_mac(), expected_mac);
    assert!(
        UdpSocket::bind(address).is_err(),
        "the running ephemeral port is exclusive"
    );
    started.router.stop().await.unwrap();
    assert_eq!(started.router.ports(), snapshots);
    // No retries or scheduling waits: an awaited successful stop is the barrier.
    let rebound = UdpSocket::bind(address)
        .expect("awaited router stop must release the exclusive B/IP endpoint");
    assert_eq!(rebound.local_addr().unwrap(), address.into());
}

#[tokio::test]
async fn normal_bip_endpoint_is_reusable_immediately_after_stop() {
    rebind_after_stop(false).await;
}

#[tokio::test]
async fn bbmd_bip_endpoint_is_reusable_immediately_after_stop() {
    rebind_after_stop(true).await;
}

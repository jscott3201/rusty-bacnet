//! One-shot exclusive rebind supplements the causal child-owner regressions.

use super::{BACnetRouter, RouterOptions, RouterPort};
use bacnet_npdu::NpduLimits;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::error::Error;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use tokio::sync::{mpsc, oneshot};

struct ObservedBip {
    inner: BipTransport,
    bound: Option<oneshot::Sender<SocketAddrV4>>,
}

impl TransportPort for ObservedBip {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let rx = self.inner.start().await?;
        let mac = self.inner.local_mac();
        let address = SocketAddrV4::new(
            Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]),
            u16::from_be_bytes([mac[4], mac[5]]),
        );
        self.bound.take().unwrap().send(address).unwrap();
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }
    fn abort(&mut self) {
        self.inner.abort();
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }
    // Startup announcements need no external datagrams for this lifecycle test.
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }
    fn max_npdu(&self) -> NpduLimits {
        self.inner.max_npdu()
    }
}

async fn rebind_after_stop(bbmd: bool) {
    let mut inner = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    if bbmd {
        inner.enable_bbmd(Vec::new());
    }
    let (bound, address) = oneshot::channel();
    let port = RouterPort {
        network_number: 1,
        transport: ObservedBip {
            inner,
            bound: Some(bound),
        },
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

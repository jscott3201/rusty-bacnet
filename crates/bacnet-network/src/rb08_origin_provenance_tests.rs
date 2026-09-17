//! RB-08 authenticated-origin deferred: legacy transport stays unknown-origin end-to-end.
//!
//! Pins the (a) gap: Loopback bytes (unverified legacy origin at the
//! transport) arrive at [`ReceivedApdu`](super::layer::ReceivedApdu) still
//! unverified — `is_unverified`, never `is_verified`/`direct`/`relayed`.
//! Verified-vs-unknown distinguishability on the `ReceivedNpdu` side is
//! pinned in `bacnet-transport` (`sc_hub_relayed_origin_is_verified_and_unknown_origin_dropped`);
//! this file pins the transport-to-network threading for legacy traffic.
//! In-memory Loopback only, no sleeps, no site scans.

use super::layer::NetworkLayer;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::NetworkPriority;
use bytes::{Bytes, BytesMut};
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn legacy_loopback_traffic_reaches_apdu_as_unverified() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut rx = network.start().await.unwrap();

    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    peer.send_unicast(&buffer.freeze(), &[0x01]).await.unwrap();

    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("loopback APDU timed out")
        .expect("network closed");
    assert_eq!(received.apdu.as_ref(), &[0x10, 0x08]);
    assert_eq!(received.source_mac.as_slice(), &[0x02]);
    // Unknown origin preserved: distinguishable from either verified variant.
    assert!(received.provenance.is_unverified());
    assert!(!received.provenance.is_verified());
    assert!(!received.provenance.is_direct_peer());
    assert!(!received.provenance.is_relayed_origin());
    assert!(received.provenance.scope().contains("unverified"));

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}

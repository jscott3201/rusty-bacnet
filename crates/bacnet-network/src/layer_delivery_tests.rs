use super::*;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::loopback::LoopbackTransport;
use std::net::Ipv4Addr;
use tokio::time::{timeout, Duration};

#[test]
fn effective_group_delivery_respects_npdu_destination_precedence() {
    let remote_unicast = NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0x11]),
    };
    let remote_broadcast = NpduAddress {
        network: 200,
        mac_address: MacAddr::new(),
    };
    let global_broadcast = NpduAddress {
        network: 0xFFFF,
        mac_address: MacAddr::new(),
    };

    assert!(!is_group_delivery(false, None));
    assert!(is_group_delivery(true, None));
    assert!(!is_group_delivery(false, Some(&remote_unicast)));
    assert!(!is_group_delivery(true, Some(&remote_unicast)));
    assert!(is_group_delivery(false, Some(&remote_broadcast)));
    assert!(is_group_delivery(false, Some(&global_broadcast)));
}

#[tokio::test]
async fn send_receive_apdu_unicast_is_not_marked_as_group_delivery() {
    let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let mut net_a = NetworkLayer::new(transport_a);
    let mut net_b = NetworkLayer::new(transport_b);

    let _rx_a = net_a.start().await.unwrap();
    let mut rx_b = net_b.start().await.unwrap();
    let test_apdu = vec![0x10, 0x08];

    net_a
        .send_apdu(
            &test_apdu,
            net_b.local_mac(),
            false,
            NetworkPriority::NORMAL,
        )
        .await
        .unwrap();

    let received = timeout(Duration::from_secs(2), rx_b.recv())
        .await
        .expect("Timed out waiting for APDU")
        .expect("Channel closed");

    assert_eq!(received.apdu, test_apdu);
    assert_eq!(received.source_mac.as_slice(), net_a.local_mac());
    assert!(received.source_network.is_none());
    assert!(!received.link_layer_group);
    assert!(!received.is_group);

    net_a.stop().await.unwrap();
    net_b.stop().await.unwrap();
}

fn encoded_npdu(destination: Option<NpduAddress>) -> Bytes {
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination,
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    buffer.freeze()
}

#[tokio::test]
async fn received_apdu_preserves_raw_and_effective_group_matrix() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut received = network.start().await.unwrap();

    peer.send_unicast(&encoded_npdu(None), &[0x01])
        .await
        .unwrap();
    let direct = timeout(Duration::from_secs(1), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((direct.link_layer_group, direct.is_group), (false, false));

    peer.send_broadcast(&encoded_npdu(None)).await.unwrap();
    let local_broadcast = timeout(Duration::from_secs(1), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (local_broadcast.link_layer_group, local_broadcast.is_group),
        (true, true)
    );

    peer.send_unicast(
        &encoded_npdu(Some(NpduAddress {
            network: 0xffff,
            mac_address: MacAddr::new(),
        })),
        &[0x01],
    )
    .await
    .unwrap();
    let global_over_unicast = timeout(Duration::from_secs(1), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            global_over_unicast.link_layer_group,
            global_over_unicast.is_group
        ),
        (false, true)
    );

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn router_preserves_link_group_for_ultimate_network_unicast() {
    use crate::router::{BACnetRouter, RouterPort};

    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let (mut router, mut local) = BACnetRouter::start(vec![RouterPort {
        transport,
        network_number: 200,
    }])
    .await
    .unwrap();
    peer.send_broadcast(&encoded_npdu(Some(NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0x01]),
    })))
    .await
    .unwrap();

    let received = timeout(Duration::from_secs(1), local.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(received.link_layer_group);
    assert!(!received.is_group);

    router.stop().await;
    peer.stop().await.unwrap();
}

#[test]
fn broadcast_to_network_rejects_dnet_ffff() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let net = NetworkLayer::new(transport);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        net.broadcast_to_network(&[0xAA], 0xFFFF, false, NetworkPriority::NORMAL)
            .await
    });
    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("0xFFFF"),
        "Error should mention 0xFFFF: {message}"
    );
}

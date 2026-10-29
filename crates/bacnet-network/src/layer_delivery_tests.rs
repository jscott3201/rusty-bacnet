use super::*;
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::loopback::LoopbackTransport;
use std::net::Ipv4Addr;
use tokio::time::{Duration, timeout};

/// Stopping the layer ends its raw receiver's stream even though the
/// loopback peer keeps its sender (#1713): what was queued before the stop
/// is drained, a later frame isn't taken, and then the stream ends.
#[tokio::test]
async fn stop_ends_the_raw_stream_after_what_was_queued() {
    use crate::loopback_fixture::{APDU, wire};
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut apdus = network.start().await.unwrap();
    let _peer_rx = peer.start().await.unwrap();
    peer.send_unicast(&wire(None, None, None), &[0x01])
        .await
        .unwrap();
    network.stop().await.unwrap();
    // The peer's sender is still alive; its frame is refused or dropped.
    let _ = peer.send_unicast(&wire(None, None, None), &[0x01]).await;
    let queued = timeout(Duration::from_secs(1), apdus.recv())
        .await
        .unwrap()
        .expect("the frame queued before the stop");
    assert_eq!(queued.apdu, APDU[..]);
    assert!(
        timeout(Duration::from_secs(1), apdus.recv())
            .await
            .expect("the stream ends")
            .is_none()
    );
    peer.stop().await.unwrap();
}

/// Dropping the layer ends the stream too.
#[tokio::test]
async fn dropping_the_layer_ends_the_raw_stream() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut apdus = network.start().await.unwrap();
    let _peer_rx = peer.start().await.unwrap();
    drop(network);
    assert!(
        timeout(Duration::from_secs(1), apdus.recv())
            .await
            .expect("the stream ends")
            .is_none()
    );
    assert!(apdus.try_recv().is_err());
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
            NetworkPriority::Normal,
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
        expecting_reply: false,
        priority: NetworkPriority::Normal,
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

fn encoded_reject_message_to_network(reason: RejectMessageReason, dnet: u16) -> Bytes {
    let npdu = Npdu {
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK),
        payload: Bytes::from(vec![reason.to_raw(), (dnet >> 8) as u8, dnet as u8]),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    buffer.freeze()
}

#[tokio::test]
async fn opted_in_network_control_stream_preserves_start_apdu_receiver() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut controls = network.enable_network_control_receiver().unwrap();
    assert!(network.enable_network_control_receiver().is_err());
    let mut apdus = network.start().await.unwrap();

    peer.send_unicast(
        &encoded_reject_message_to_network(RejectMessageReason::MESSAGE_TOO_LONG, 100),
        &[0x01],
    )
    .await
    .unwrap();

    // Polling the APDU receiver decodes the message, which goes to the
    // control receiver and yields no APDU (#1713).
    assert!(
        timeout(Duration::from_millis(25), apdus.recv())
            .await
            .is_err()
    );
    let received = timeout(Duration::from_secs(1), controls.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.source_mac.as_slice(), &[0x02]);
    assert_eq!(received.ingress_sequence, 1);
    assert_eq!(network.network_control_ingress_sequence(), 1);
    assert_eq!(
        received.npdu.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
    );
    assert!(network.enable_network_control_receiver().is_err());

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn network_controls_without_opt_in_are_discarded_without_stopping_apdu_ingress() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut apdus = network.start().await.unwrap();

    peer.send_unicast(
        &encoded_reject_message_to_network(RejectMessageReason::MESSAGE_TOO_LONG, 100),
        &[0x01],
    )
    .await
    .unwrap();
    assert!(
        timeout(Duration::from_millis(25), apdus.recv())
            .await
            .is_err()
    );

    peer.send_unicast(&encoded_npdu(None), &[0x01])
        .await
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), apdus.recv())
            .await
            .unwrap()
            .unwrap()
            .apdu,
        Bytes::from_static(&[0x10, 0x08])
    );

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
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
    use crate::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};

    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let port = RouterPort {
        transport,
        network_number: 200,
    };
    let StartedRouter {
        mut router,
        apdus: mut local,
        ..
    } = BACnetRouter::start(vec![port], RouterOptions::new())
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

    router.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[test]
fn global_broadcast_npdu_has_dnet_ffff() {
    use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
    use bacnet_npdu::NetworkPriority;

    let npdu = Npdu {
        expecting_reply: false,
        priority: NetworkPriority::Normal,
        destination: Some(NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::new(),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0xAA]),
        ..Npdu::default()
    };

    let mut buf = bytes::BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let decoded = decode_npdu(Bytes::from(buf)).unwrap();
    let dest = decoded.destination.unwrap();
    assert_eq!(dest.network, 0xFFFF);
    assert!(dest.mac_address.is_empty());
    assert_eq!(decoded.hop_count, 255);
}

#[test]
fn routed_send_encodes_dnet_dadr() {
    use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
    use bacnet_npdu::NetworkPriority;

    let npdu = Npdu {
        expecting_reply: true,
        priority: NetworkPriority::Normal,
        destination: Some(NpduAddress {
            network: 100,
            mac_address: MacAddr::from_slice(&[1, 2, 3, 4, 5, 6]),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0xAA, 0xBB]),
        ..Npdu::default()
    };

    let mut buf = bytes::BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let decoded = decode_npdu(Bytes::from(buf)).unwrap();
    let dest = decoded.destination.unwrap();
    assert_eq!(dest.network, 100);
    assert_eq!(dest.mac_address.as_slice(), &[1, 2, 3, 4, 5, 6]);
    assert_eq!(decoded.hop_count, 255);
    assert!(decoded.expecting_reply);
}

#[test]
fn broadcast_to_network_encodes_specific_dnet() {
    use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
    use bacnet_npdu::NetworkPriority;

    let npdu = Npdu {
        expecting_reply: false,
        priority: NetworkPriority::Normal,
        destination: Some(NpduAddress {
            network: 42,
            mac_address: MacAddr::new(),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::from_static(&[0xCC]),
        ..Npdu::default()
    };

    let mut buf = bytes::BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let decoded = decode_npdu(Bytes::from(buf)).unwrap();
    let dest = decoded.destination.unwrap();
    assert_eq!(dest.network, 42);
    assert!(dest.mac_address.is_empty());
    assert_eq!(decoded.hop_count, 255);
    assert!(!decoded.expecting_reply);
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
        net.broadcast_to_network(&[0xAA], 0xFFFF, false, NetworkPriority::Normal)
            .await
    });
    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("0xFFFF"),
        "Error should mention 0xFFFF: {message}"
    );
}

/// Network 0 is not a network number, so every send that names a DNET
/// refuses it by name and puts no frame on the link (#1314).
#[tokio::test]
async fn sends_naming_network_zero_are_refused_before_any_frame() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut frames = peer.start().await.unwrap();
    let net = NetworkLayer::new(transport);
    let apdu = [0x10, 0x08];
    let priority = NetworkPriority::Normal;
    let network_zero = NpduAddress {
        network: 0,
        mac_address: MacAddr::from_slice(&[7]),
    };
    // Each refusal names the value and the local form of that send.
    let refusals = [
        (
            net.broadcast_to_network(&apdu, 0, false, priority).await,
            "use broadcast_apdu",
        ),
        (
            net.send_apdu_routed(&apdu, 0, &[7], &[2], false, priority)
                .await,
            "use send_apdu",
        ),
        (
            net.send_apdu_routed_via_local_broadcast(&apdu, 0, &[7], false, priority)
                .await,
            "use send_apdu",
        ),
        (
            net.send_apdu_on_issuance(&apdu, &[2], Some(&network_zero), false, priority, || {
                panic!("a refused response is never issued")
            })
            .await,
            "pass no destination",
        ),
    ];
    for (refusal, local_form) in refusals {
        let message = refusal.unwrap_err().to_string();
        assert!(
            message.contains("dest_network 0 is not a network number")
                && message.contains(local_form),
            "{message}"
        );
    }
    assert!(frames.try_recv().is_err());

    // The same broadcast to a real network number does reach the link.
    net.broadcast_to_network(&apdu, 5, false, priority)
        .await
        .unwrap();
    assert!(frames.try_recv().is_ok());
}

/// DNET 0xFFFF reaches every device on every network. Its sender puts it on
/// its own network with the broadcast MAC so each router there can pass it
/// on (Clause 6.3.2), which only `broadcast_global_apdu` does. Every routed
/// or on-issuance send refuses that DNET by name and puts no frame on the
/// link: with a device's MAC beside it (#1340), and with none (#1380), where
/// a unicast would reach a single router. The global broadcast itself and a
/// routed send to a real network number with the same MAC still go out.
#[tokio::test]
async fn sends_naming_the_global_network_are_refused_before_any_frame() {
    use bacnet_encoding::npdu::decode_npdu;

    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut frames = peer.start().await.unwrap();
    let net = NetworkLayer::new(transport);
    let apdu = [0x10, 0x08];
    let priority = NetworkPriority::Normal;
    fn target(network: u16, mac: &[u8]) -> RoutedTarget<'_> {
        RoutedTarget {
            network,
            mac,
            router_mac: &[2],
        }
    }
    for dadr in [&[7][..], &[]] {
        let global = NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::from_slice(dadr),
        };
        let refusals = [
            net.send_apdu_routed(&apdu, 0xFFFF, dadr, &[2], false, priority)
                .await,
            net.send_apdu_routed_with_data_attributes(
                &apdu,
                target(0xFFFF, dadr),
                false,
                priority,
                &[],
            )
            .await,
            net.send_apdu_routed_via_local_broadcast(&apdu, 0xFFFF, dadr, false, priority)
                .await,
            net.send_apdu_routed_via_local_broadcast_with_data_attributes(
                &apdu,
                0xFFFF,
                dadr,
                false,
                priority,
                &[],
            )
            .await,
            net.send_apdu_on_issuance(&apdu, &[2], Some(&global), false, priority, || {
                panic!("a refused send is never issued")
            })
            .await,
        ];
        for refusal in refusals {
            let message = refusal.unwrap_err().to_string();
            assert!(
                message.contains("dest_network 0xFFFF is the global broadcast")
                    && message.contains("broadcast_global_apdu"),
                "DADR {dadr:?}: {message}"
            );
        }
    }
    assert!(frames.try_recv().is_err());

    let sent = |frame: bacnet_transport::port::ReceivedNpdu| {
        let destination = decode_npdu(frame.npdu).unwrap().destination.unwrap();
        (frame.link_layer_group, destination)
    };
    net.broadcast_global_apdu(&apdu, false, priority)
        .await
        .unwrap();
    let (broadcast, global) = sent(frames.try_recv().unwrap());
    assert!(
        broadcast,
        "a global broadcast goes out with the broadcast MAC"
    );
    assert_eq!((global.network, global.mac_address.len()), (0xFFFF, 0));

    net.send_apdu_routed_with_data_attributes(&apdu, target(5, &[7]), false, priority, &[])
        .await
        .unwrap();
    let (_, routed) = sent(frames.try_recv().unwrap());
    assert_eq!((routed.network, &routed.mac_address[..]), (5, &[7][..]));
}

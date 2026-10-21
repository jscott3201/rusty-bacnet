use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::TransportProvenance;
use bytes::Bytes;

fn parse(bytes: &[u8], group: bool) -> Option<NumberControl> {
    NumberControl::parse(&ReceivedNetworkControl {
        npdu: decode_npdu(Bytes::copy_from_slice(bytes)).unwrap(),
        source_mac: [2].as_slice().into(),
        link_layer_group: group,
        data_attributes: vec![],
        provenance: TransportProvenance::unverified(),
        ingress_sequence: 19,
    })
}
#[test]
fn number_control_independent_wire_vectors_and_reply_flags() {
    for group in [false, true] {
        assert_eq!(parse(&[1, 0x80, 0x12], group), Some(NumberControl::WhatIs));
    }
    assert_eq!(
        parse(&[1, 0x80, 0x13, 0, 77, 1], true),
        Some(NumberControl::NumberIs {
            number: 77,
            flag: 1
        })
    );
    for bytes in [
        vec![1, 0x80, 0x12, 0],
        vec![1, 0x80, 0x13, 0, 77],
        vec![1, 0x80, 0x13, 0, 77, 1, 0],
        vec![1, 0x80, 0x13, 0, 0, 1],
        vec![1, 0x80, 0x13, 255, 255, 1],
        vec![1, 0x80, 0x13, 0, 77, 2],
        vec![1, 0x88, 0, 4, 1, 9, 0x12],
        vec![1, 0xa0, 255, 255, 0, 255, 0x12],
        vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 77, 1],
        vec![1, 0xa0, 255, 255, 0, 255, 0x13, 0, 77, 1],
        vec![1, 0x80, 0x03, 4, 0, 100],
        vec![1, 0, 0x12],
    ] {
        assert_eq!(parse(&bytes, true), None, "{bytes:?}");
    }
    assert_eq!(parse(&[1, 0x80, 0x13, 0, 77, 1], false), None);
    let mut state = NetworkNumber::default();
    assert_eq!(number_is_reply(state), None);
    state.observe(77, 1);
    assert_eq!(number_is_reply(state), Some([1, 0x80, 0x13, 0, 77, 0]));
    assert_eq!(
        number_is_reply(NetworkNumber::configured(77).unwrap()),
        Some([1, 0x80, 0x13, 0, 77, 1])
    );
}

/// A Network-Number-Is whose data isn't three octets is refused, through
/// the router core's view (#1818): cut short, or padded past the flag.
#[test]
fn number_is_of_the_wrong_length_is_refused() {
    let good = [1, 0x80, 0x13, 0x0B, 0xB8, 0];
    assert_eq!(
        parse(&good, true),
        Some(NumberControl::NumberIs {
            number: 3000,
            flag: 0
        })
    );
    for len in 3..good.len() {
        assert_eq!(parse(&good[..len], true), None, "short: {len} octets");
    }
    for padding in [&[0u8][..], &[0, 0], &[1, 2, 3]] {
        let padded = [&good[..], padding].concat();
        assert_eq!(parse(&padded, true), None, "padded: {padded:?}");
    }
}

#[test]
fn local_network_number_shares_the_last_published_state() {
    let slot = LocalNetworkNumber::default();
    let reader = slot.clone();
    assert_eq!(reader.get(), None);
    let mut state = NetworkNumber::default();
    slot.publish(state);
    assert_eq!(reader.get(), None);
    state.observe(77, 0);
    slot.publish(state);
    assert_eq!(reader.get(), Some(77));
    state.observe(78, 1);
    slot.publish(state);
    assert_eq!(reader.get(), Some(78));
    slot.publish(NetworkNumber::configured(65534).unwrap());
    assert_eq!(reader.get(), Some(65534));
    // A new state that is unknown again (a fresh owner) reads as unknown.
    slot.publish(NetworkNumber::configured(0).unwrap());
    assert_eq!(reader.get(), None);
}

/// The layer frames the destination its caller names, a published number
/// notwithstanding (#1358): choosing local or routed traffic is the caller's,
/// since an answer goes back by the route its request arrived on and a
/// confirmed request is keyed to the peer it was sent to.
#[tokio::test]
async fn a_published_number_leaves_routed_sends_as_named() {
    use crate::layer::NetworkLayer;
    use bacnet_encoding::npdu::NpduAddress;
    use bacnet_npdu::NetworkPriority;
    use bacnet_transport::loopback::LoopbackTransport;
    use bacnet_transport::port::TransportPort;

    let (mut link, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut destinations = link.record_unicast_destinations();
    let mut network = NetworkLayer::new(link);
    let _apdus = network.start().await.unwrap();
    let mut frames = peer.start().await.unwrap();
    network
        .local_network_number()
        .publish(NetworkNumber::configured(7).unwrap());
    let apdu = [0x10, 0x08];
    let priority = NetworkPriority::Normal;

    network
        .send_apdu_routed(&apdu, 7, &[3], &[9], false, priority)
        .await
        .unwrap();
    let frame = frames.recv().await.unwrap();
    assert_eq!(destinations.recv().await.unwrap().as_slice(), [9]);
    assert_eq!(
        decode_npdu(frame.npdu).unwrap().destination,
        Some(NpduAddress {
            network: 7,
            mac_address: [3].as_slice().into(),
        })
    );

    network
        .broadcast_to_network(&apdu, 7, false, priority)
        .await
        .unwrap();
    let frame = frames.recv().await.unwrap();
    assert!(frame.link_layer_group);
    let destination = decode_npdu(frame.npdu).unwrap().destination.unwrap();
    assert_eq!((destination.network, destination.mac_address.len()), (7, 0));
    network.stop().await.unwrap();
}

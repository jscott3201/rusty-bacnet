//! Network reports read from received controls, and the local
//! network-message send (#960).

use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_npdu::{NetworkMessageType, RejectMessageReason, RejectMessageToNetwork};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use std::time::Duration;

fn control(bytes: &[u8], group: bool) -> ReceivedNetworkControl {
    ReceivedNetworkControl {
        npdu: decode_npdu(Bytes::copy_from_slice(bytes)).unwrap(),
        source_mac: [7].as_slice().into(),
        link_layer_group: group,
        data_attributes: vec![],
        provenance: TransportProvenance::unverified(),
        ingress_sequence: 1,
    }
}

fn report(bytes: &[u8], group: bool) -> Option<NetworkReport> {
    let control = control(bytes, group);
    let active = ReceivedNetworkReport::parse(&control).map(|received| {
        assert_eq!(received.source_mac.as_slice(), [7]);
        received.report
    });
    assert_eq!(NetworkReport::parse_npdu(&control.npdu, group), active);
    active
}

#[test]
fn i_am_router_lists_its_networks_in_order() {
    assert_eq!(
        report(&[1, 0x80, 0x01, 0, 2, 0x0B, 0xB8], true),
        Some(NetworkReport::IAmRouterToNetwork(vec![2, 3000]))
    );
    // Entries 0 and 0xFFFF are dropped and the rest kept.
    assert_eq!(
        report(&[1, 0x80, 0x01, 0, 0, 0, 2, 0xFF, 0xFF, 0, 5], true),
        Some(NetworkReport::IAmRouterToNetwork(vec![2, 5]))
    );
    // Odd length, an empty list, and a list of nothing but 0 and 0xFFFF
    // are refused.
    for bytes in [
        &[1, 0x80, 0x01, 0, 2, 0][..],
        &[1, 0x80, 0x01],
        &[1, 0x80, 0x01, 0, 0],
        &[1, 0x80, 0x01, 0xFF, 0xFF, 0, 0],
    ] {
        assert_eq!(report(bytes, true), None, "{bytes:?}");
    }
}

#[test]
fn network_number_is_follows_the_number_control_rules() {
    assert_eq!(
        report(&[1, 0x80, 0x13, 0, 77, 1], true),
        Some(NetworkReport::NetworkNumberIs {
            network: 77,
            configured: true
        })
    );
    assert_eq!(
        report(&[1, 0x80, 0x13, 0, 77, 0], true),
        Some(NetworkReport::NetworkNumberIs {
            network: 77,
            configured: false
        })
    );
    // A unicast one is ignored (Clause 6.4.20), as is a routed one.
    assert_eq!(report(&[1, 0x80, 0x13, 0, 77, 1], false), None);
    assert_eq!(report(&[1, 0x88, 0, 4, 1, 9, 0x13, 0, 77, 1], true), None);
    // What-Is-Network-Number is a question, not a report.
    assert_eq!(report(&[1, 0x80, 0x12], true), None);
}

#[test]
fn a_reject_names_its_reason_and_network_and_keeps_its_source() {
    let received =
        ReceivedNetworkReport::parse(&control(&[1, 0x88, 0, 9, 1, 0x44, 0x03, 1, 0, 99], false))
            .unwrap();
    assert_eq!(
        received.report,
        NetworkReport::RejectMessageToNetwork(RejectMessageToNetwork {
            reason: RejectMessageReason::NOT_DIRECTLY_CONNECTED,
            network: 99,
        })
    );
    assert_eq!(
        received.source_network,
        Some(NpduAddress {
            network: 9,
            mac_address: [0x44].as_slice().into(),
        })
    );
    for bytes in [&[1, 0x80, 0x03, 1, 0][..], &[1, 0x80, 0x03, 1, 0, 99, 0]] {
        assert_eq!(report(bytes, false), None, "{bytes:?}");
    }
}

#[test]
fn other_messages_and_apdus_are_not_reports() {
    for bytes in [
        &[1, 0x80, 0x00][..],
        &[1, 0x80, 0x04, 0, 2],
        &[1, 0x80, 0x02, 0, 2, 1],
    ] {
        assert_eq!(report(bytes, true), None, "{bytes:?}");
    }
}

async fn sent(
    message_type: NetworkMessageType,
    payload: &[u8],
    destination: Option<&[u8]>,
) -> (
    Result<(), Error>,
    Option<bacnet_transport::port::ReceivedNpdu>,
) {
    let (a, mut b) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut rx = b.start().await.unwrap();
    let mut layer = NetworkLayer::new(a);
    let _apdus = layer.start().await.unwrap();
    let result = layer
        .send_network_message(message_type, payload, destination)
        .await;
    let frame = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .ok()
        .flatten();
    (result, frame)
}

#[tokio::test]
async fn a_local_network_message_has_no_dnet_and_goes_where_asked() {
    let (result, frame) = sent(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0, 2], None).await;
    result.unwrap();
    let frame = frame.expect("broadcast reaches the peer");
    assert_eq!(&frame.npdu[..], [1, 0x80, 0x00, 0, 2]);

    let (result, frame) = sent(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[], Some(&[2])).await;
    result.unwrap();
    assert_eq!(
        &frame.expect("unicast reaches the peer").npdu[..],
        [1, 0x80, 0x12]
    );
}

#[tokio::test]
async fn only_the_two_questions_are_sent() {
    for refused in [
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        NetworkMessageType::ROUTER_BUSY_TO_NETWORK,
        NetworkMessageType::NETWORK_NUMBER_IS,
        NetworkMessageType::from_raw(0x14),
        NetworkMessageType::from_raw(0x80),
    ] {
        let (result, frame) = sent(refused, &[0, 2], None).await;
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("not one a non-routing node sends"),
            "{refused}: {error}"
        );
        assert!(frame.is_none(), "{refused}");
    }
}

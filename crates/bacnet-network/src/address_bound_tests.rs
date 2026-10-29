//! Network-layer handling of an over-long DADR or SADR (#1141), on the wire.
//!
//! Raw NPDU bytes go in through a [`LoopbackTransport`] peer, or through the
//! shared [`RouterFixture`] for the router. The non-router [`NetworkLayer`]
//! drops and counts an NPDU whose DLEN or SLEN is past
//! [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN); the router also refuses to forward or deliver
//! it, and rejects it with reason 6 when it names a specific DNET. Every case
//! runs once per address field, and an NPDU sent right after the refused one
//! is the first thing to come out, which shows the refused one went nowhere.

use bacnet_encoding::npdu::{Npdu, NpduAddress};
use bacnet_npdu::AddressField;
use bacnet_npdu::RejectMessageToNetwork;
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;

use crate::layer::NetworkLayer;
use crate::loopback_fixture::{
    APDU, LONGEST, ORIGIN, REMOTE, RouterFixture, TOO_LONG, next_from_router, pump, recv,
    reject_of, wire,
};

const FIELDS: [AddressField; 2] = [AddressField::Destination, AddressField::Source];

fn address_of(npdu: &Npdu, field: AddressField) -> &NpduAddress {
    match field {
        AddressField::Destination => npdu.destination.as_ref(),
        AddressField::Source => npdu.source.as_ref(),
    }
    .expect("NPDU carries the address under test")
}

/// A non-router frame: a global broadcast DNET with a `length`-octet DADR, or
/// a local NPDU with a `length`-octet SADR.
fn local_frame(field: AddressField, length: u8, control: Option<NetworkMessageType>) -> Vec<u8> {
    match field {
        AddressField::Destination => wire(Some((0xFFFF, length)), None, control),
        AddressField::Source => wire(None, Some((ORIGIN, length)), control),
    }
}

#[tokio::test]
async fn non_router_drops_and_counts_an_over_long_address_in_either_field() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut network = NetworkLayer::new(transport);
    let mut controls = network.enable_network_control_receiver().unwrap();
    let mut apdus = network.start().await.unwrap();
    let control = Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);

    for field in FIELDS {
        for length in [TOO_LONG, 255] {
            peer.send_unicast(&local_frame(field, length, None), &[0x01])
                .await
                .unwrap();
            peer.send_unicast(&local_frame(field, length, control), &[0x01])
                .await
                .unwrap();
        }
        peer.send_unicast(&local_frame(field, LONGEST, None), &[0x01])
            .await
            .unwrap();
        peer.send_unicast(&local_frame(field, LONGEST, control), &[0x01])
            .await
            .unwrap();

        // An 18-octet address fits. As a SADR it comes out first. As a DADR
        // it sits beside DNET 0xFFFF, the only DNET a non-router takes, and
        // any DADR there is dropped for that, apart from its length (#1379):
        // the plain local NPDUs sent after it come out first.
        if field == AddressField::Destination {
            peer.send_unicast(&wire(None, None, None), &[0x01])
                .await
                .unwrap();
            peer.send_unicast(&wire(None, None, control), &[0x01])
                .await
                .unwrap();
        }
        let apdu = recv(&mut apdus).await;
        assert_eq!(apdu.apdu, APDU[..], "{field}");
        // The controls behind the last APDU reach their receiver once the
        // APDU receiver is polled again (#1713).
        pump(&mut apdus);
        let received = recv(&mut controls).await;
        match field {
            AddressField::Destination => {
                assert!(apdu.source_network.is_none() && !apdu.global_broadcast);
                assert_eq!(received.npdu.destination, None);
            }
            AddressField::Source => {
                let source = apdu.source_network.expect("routed source survives");
                assert_eq!(source.mac_address.len(), bacnet_npdu::MAX_MAC_LEN);
                assert_eq!(
                    address_of(&received.npdu, field).mac_address.len(),
                    bacnet_npdu::MAX_MAC_LEN
                );
            }
        }
    }
    assert_eq!(network.address_length_drops(), 8);
    assert_eq!(network.global_broadcast_dadr_drops(), 2);

    network.stop().await.unwrap();
    peer.stop().await.unwrap();
}

/// A frame from peer A toward network 3000 whose `field` address is `length`
/// octets long.
fn routed_frame(field: AddressField, length: u8) -> Vec<u8> {
    match field {
        AddressField::Destination => wire(Some((REMOTE, length)), None, None),
        AddressField::Source => wire(Some((REMOTE, 1)), Some((ORIGIN, length)), None),
    }
}

#[tokio::test]
async fn router_rejects_an_over_long_address_toward_a_dnet_with_reason_6() {
    for field in FIELDS {
        let mut fixture = RouterFixture::start().await;

        fixture.send_from_a(&routed_frame(field, TOO_LONG)).await;
        let (reject, to) = next_from_router(&mut fixture.from_router_a).await;
        assert_eq!(
            to.as_deref(),
            Some(&[0x0A][..]),
            "{field}: the reject is a unicast to the sender"
        );
        assert!(reject.destination.is_none() && reject.source.is_none());
        assert_eq!(
            reject_of(&reject),
            RejectMessageToNetwork {
                reason: RejectMessageReason::ADDRESSING_ERROR,
                network: REMOTE,
            },
            "{field}"
        );

        fixture.send_from_a(&routed_frame(field, LONGEST)).await;
        let (forwarded, _) = next_from_router(&mut fixture.from_router_b).await;
        assert_eq!(forwarded.payload, APDU[..], "{field}");
        assert_eq!(forwarded.destination.as_ref().unwrap().network, REMOTE);
        assert_eq!(
            address_of(&forwarded, field).mac_address.len(),
            bacnet_npdu::MAX_MAC_LEN,
            "{field}: the boundary address is forwarded intact"
        );
        assert_eq!(fixture.router.address_length_drops(), 1, "{field}");
        fixture.stop().await;
    }
}

#[tokio::test]
async fn router_drops_an_over_long_broadcast_or_local_npdu_without_a_reject() {
    let mut fixture = RouterFixture::start().await;
    let refused = [
        wire(Some((0xFFFF, TOO_LONG)), None, None),
        wire(Some((0xFFFF, 0)), Some((ORIGIN, TOO_LONG)), None),
        wire(None, Some((ORIGIN, 255)), None),
    ];
    for bytes in &refused {
        fixture.send_from_a(bytes).await;
    }

    // For an unknown DNET the router broadcasts its route query on port B and
    // rejects with reason 1 on port A. Each is the first frame on its port,
    // so none of the refused NPDUs drew a reject or was forwarded.
    fixture
        .send_from_a(&wire(Some((5000, 0)), None, None))
        .await;
    let (reject, _) = next_from_router(&mut fixture.from_router_a).await;
    assert_eq!(
        reject_of(&reject).reason,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED
    );
    let (solicit, _) = next_from_router(&mut fixture.from_router_b).await;
    assert_eq!(
        solicit.message_type,
        Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
    );

    // Nor was any delivered locally: the boundary NPDU is the first to arrive.
    fixture
        .send_from_a(&wire(None, Some((ORIGIN, LONGEST)), None))
        .await;
    let delivered = recv(&mut fixture.local).await;
    let source = delivered.source_network.expect("routed source survives");
    assert_eq!(source.mac_address.len(), bacnet_npdu::MAX_MAC_LEN);
    assert_eq!(fixture.router.address_length_drops(), refused.len() as u64);
    fixture.stop().await;
}

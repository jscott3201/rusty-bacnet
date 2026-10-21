//! What a broadcast may carry (#1479).
//!
//! Clause 6.3 keeps every broadcast network address, local, remote or
//! global, for Unconfirmed-Request PDUs. Each send that puts an APDU on one
//! refuses every other PDU type by name, before any frame reaches the link,
//! and still sends an Unconfirmed-Request. A send that names one device keeps
//! taking any PDU type, even when its link DA is the broadcast MAC.

use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;

/// The peer on the link, also the next-hop router for routed sends.
const PEER: [u8; 1] = [0x02];

/// One APDU of each PDU type but Unconfirmed-Request, with its type's name.
/// Only the first octet's high nibble sets the type (Clause 20.1).
const REFUSED: [(&[u8], &str); 9] = [
    // ReadProperty request, invoke ID 1.
    (&[0x00, 0x05, 0x01, 0x0C], "CONFIRMED_REQUEST"),
    // WriteProperty SimpleACK.
    (&[0x20, 0x01, 0x0F], "SIMPLE_ACK"),
    // ReadProperty ComplexACK, start of.
    (&[0x30, 0x01, 0x0C, 0x0C], "COMPLEX_ACK"),
    (&[0x40, 0x01, 0x00, 0x04], "SEGMENT_ACK"),
    // ReadProperty Error: PROPERTY / UNKNOWN_PROPERTY.
    (&[0x50, 0x01, 0x0C, 0x91, 0x02, 0x91, 0x20], "ERROR"),
    (&[0x60, 0x01, 0x04], "REJECT"),
    (&[0x70, 0x01, 0x04], "ABORT"),
    // A reserved PDU type is named by its number.
    (&[0x80, 0x01], "PDU type 8"),
    (&[], "an empty APDU"),
];

/// A Who-Is, an Unconfirmed-Request.
const WHO_IS: [u8; 2] = [0x10, 0x08];

/// A network layer and the frames its peer receives.
async fn link() -> (
    NetworkLayer<LoopbackTransport>,
    mpsc::Receiver<ReceivedNpdu>,
) {
    let (transport, mut peer) = LoopbackTransport::pair(vec![0x01], PEER.to_vec());
    let frames = peer.start().await.unwrap();
    (NetworkLayer::new(transport), frames)
}

/// Send `apdu` through every form that puts it on a broadcast network
/// address, and return each form's name with its result.
async fn every_broadcast_form(
    net: &NetworkLayer<LoopbackTransport>,
    apdu: &[u8],
) -> Vec<(&'static str, Result<(), Error>)> {
    let p = NetworkPriority::Normal;
    let remote_broadcast = NpduAddress {
        network: 5,
        mac_address: MacAddr::new(),
    };
    let to_router = RoutedTarget {
        network: 5,
        mac: &[],
        router_mac: &PEER,
    };
    let issued = IssuedApdu {
        apdu,
        next_hop: &PEER,
        destination: Some(&remote_broadcast),
        expecting_reply: false,
        priority: p,
    };
    let unverified = crate::response_route::ResponseRoute::unverified();
    vec![
        ("broadcast_apdu", net.broadcast_apdu(apdu, false, p).await),
        (
            "broadcast_apdu_with_data_attributes",
            net.broadcast_apdu_with_data_attributes(apdu, false, p, &[])
                .await,
        ),
        (
            "broadcast_global_apdu",
            net.broadcast_global_apdu(apdu, false, p).await,
        ),
        (
            "broadcast_global_apdu_with_data_attributes",
            net.broadcast_global_apdu_with_data_attributes(apdu, false, p, &[])
                .await,
        ),
        (
            "broadcast_to_network",
            net.broadcast_to_network(apdu, 5, false, p).await,
        ),
        (
            "broadcast_to_network_with_data_attributes",
            net.broadcast_to_network_with_data_attributes(apdu, 5, false, p, &[])
                .await,
        ),
        (
            "send_apdu_routed, empty DADR",
            net.send_apdu_routed(apdu, 5, &[], &PEER, false, p).await,
        ),
        (
            "send_apdu_routed_with_data_attributes, empty DADR",
            net.send_apdu_routed_with_data_attributes(apdu, to_router, false, p, &[])
                .await,
        ),
        (
            "send_apdu_on_issuance, empty DADR",
            net.send_apdu_on_issuance(apdu, &PEER, Some(&remote_broadcast), false, p, || {})
                .await,
        ),
        (
            "send_response_apdu_on_issuance, empty DADR",
            net.send_response_apdu_on_issuance(issued, &unverified, || {})
                .await,
        ),
    ]
}

/// Each broadcast form refuses every PDU type but Unconfirmed-Request,
/// names it, and sends nothing; an issuance callback never runs for a
/// refused send.
#[tokio::test(start_paused = true)]
async fn broadcast_forms_refuse_every_pdu_but_an_unconfirmed_request() {
    let (net, mut frames) = link().await;
    for (apdu, pdu_type) in REFUSED {
        for (form, result) in every_broadcast_form(&net, apdu).await {
            let message = result
                .expect_err(&format!("{form} sent {pdu_type}"))
                .to_string();
            assert!(
                message.contains("carries only an UNCONFIRMED_REQUEST APDU (Clause 6.3)")
                    && message.contains(pdu_type),
                "{form}, {pdu_type}: {message}"
            );
        }
        assert!(frames.try_recv().is_err(), "{pdu_type} reached the link");
    }
    let issued = AtomicBool::new(false);
    let remote_broadcast = NpduAddress {
        network: 5,
        mac_address: MacAddr::new(),
    };
    let refused = net
        .send_apdu_on_issuance(
            REFUSED[0].0,
            &PEER,
            Some(&remote_broadcast),
            false,
            NetworkPriority::Normal,
            || issued.store(true, Ordering::SeqCst),
        )
        .await;
    assert!(refused.is_err() && !issued.load(Ordering::SeqCst));
}

/// An Unconfirmed-Request goes out through every one of those forms, each
/// with the network address that makes it the broadcast it is.
#[tokio::test(start_paused = true)]
async fn broadcast_forms_send_an_unconfirmed_request() {
    use bacnet_encoding::npdu::decode_npdu;

    let (net, mut frames) = link().await;
    let results = every_broadcast_form(&net, &WHO_IS).await;
    for (form, result) in results {
        result.unwrap_or_else(|error| panic!("{form}: {error}"));
        let frame = frames
            .try_recv()
            .unwrap_or_else(|_| panic!("{form} sent nothing"));
        let npdu = decode_npdu(frame.npdu).unwrap();
        assert_eq!(npdu.payload[..], WHO_IS, "{form}");
        let destination = npdu
            .destination
            .map(|destination| (destination.network, destination.mac_address.len()));
        let expected = match form {
            f if f.starts_with("broadcast_apdu") => None,
            f if f.starts_with("broadcast_global") => Some((0xFFFF, 0)),
            _ => Some((5, 0)),
        };
        assert_eq!(destination, expected, "{form}");
    }
    assert!(frames.try_recv().is_err());
}

/// The `_via_local_broadcast` forms name one device; with an empty DADR they
/// would duplicate `broadcast_to_network`, so they refuse it for any PDU
/// type and point there, sending nothing.
#[tokio::test(start_paused = true)]
async fn routed_local_broadcast_forms_refuse_an_empty_dadr() {
    let (net, mut frames) = link().await;
    let p = NetworkPriority::Normal;
    for apdu in [&WHO_IS[..], REFUSED[0].0] {
        for result in [
            net.send_apdu_routed_via_local_broadcast(apdu, 5, &[], false, p)
                .await,
            net.send_apdu_routed_via_local_broadcast_with_data_attributes(
                apdu,
                5,
                &[],
                false,
                p,
                &[],
            )
            .await,
        ] {
            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("broadcast on network 5")
                    && message.contains("use broadcast_to_network"),
                "{message}"
            );
        }
    }
    assert!(frames.try_recv().is_err());
}

/// A send that names one device takes any PDU type: a unicast, a routed
/// send with a DADR, a routed send whose link DA is the broadcast MAC
/// because the router isn't known (Clause 6.3 lets the MAC layer broadcast
/// when the network address names one device), and a response routed back.
#[tokio::test(start_paused = true)]
async fn sends_naming_one_device_take_any_pdu_type() {
    let (net, mut frames) = link().await;
    let p = NetworkPriority::Normal;
    let device = NpduAddress {
        network: 5,
        mac_address: MacAddr::from_slice(&[7]),
    };
    for (apdu, pdu_type) in &REFUSED[..7] {
        for (form, result) in [
            ("send_apdu", net.send_apdu(apdu, &PEER, true, p).await),
            (
                "send_apdu_routed",
                net.send_apdu_routed(apdu, 5, &[7], &PEER, true, p).await,
            ),
            (
                "send_apdu_routed_via_local_broadcast",
                net.send_apdu_routed_via_local_broadcast(apdu, 5, &[7], true, p)
                    .await,
            ),
            (
                "send_apdu_on_issuance",
                net.send_apdu_on_issuance(apdu, &PEER, Some(&device), false, p, || {})
                    .await,
            ),
        ] {
            result.unwrap_or_else(|error| panic!("{form}, {pdu_type}: {error}"));
            assert!(frames.try_recv().is_ok(), "{form}, {pdu_type} sent nothing");
        }
    }
}

use super::{Framing, kebab, summarize_frame};
use crate::decode::testing::*;
use bacnet_encoding::npdu::{NpduAddress, decode_npdu, encode_npdu};
use bacnet_transport::bip6::{Bip6Address, Bip6Vmac, Bvll6, encode_bvll6};
use bacnet_transport::bvll::{Bvll, encode_bvll};
use bytes::{Bytes, BytesMut};

#[test]
fn forwarded_ipv4_preserves_original_ip_and_routed_stations_separately() {
    let data = routed_npdu(&i_am(123), false, Some((7, &[5])));
    let mut data = decode_npdu(Bytes::from(data)).unwrap();
    data.destination = Some(NpduAddress {
        network: 8,
        mac_address: bacnet_types::MacAddr::from_slice(&[6]),
    });
    let mut payload = BytesMut::new();
    encode_npdu(&mut payload, &data).unwrap();
    let mut wire = BytesMut::new();
    encode_bvll(
        &mut wire,
        &Bvll::ForwardedNpdu {
            origin: "192.0.2.3:47808".parse().unwrap(),
            npdu: &payload,
        },
    )
    .unwrap();
    let summary = summarize_frame(Framing::Bip, &wire);
    assert_eq!(summary.original_source.as_deref(), Some("192.0.2.3:47808"));
    assert_eq!(
        summary.link_source, None,
        "physical UDP source is not encoded"
    );
    assert_eq!(summary.snet, Some(7));
    assert_eq!(summary.dnet, Some(8));
    assert_eq!(summary.routed_source.as_deref(), Some("7:0x05"));
    assert_eq!(summary.routed_destination.as_deref(), Some("8:0x06"));
    assert_eq!(summary.device, Some(123));
    assert_eq!(summary.parse_error, None);
}

#[test]
fn forwarded_ipv6_preserves_original_address_and_original_vmac() {
    let payload = npdu(&i_am(123), false);
    let mut wire = BytesMut::new();
    encode_bvll6(
        &mut wire,
        &Bvll6::ForwardedNpdu {
            original_source: Bip6Vmac::new([0, 1, 2]),
            original_address: Bip6Address::new("2001:db8::3".parse().unwrap(), 47808),
            npdu: &payload,
        },
    )
    .unwrap();
    let summary = summarize_frame(Framing::Bip6, &wire);
    assert_eq!(
        summary.original_source.as_deref(),
        Some("[2001:db8::3]:47808")
    );
    assert_eq!(summary.link_source.as_deref(), Some("bip6:0x000102"));
    assert_eq!(summary.device, Some(123));
    assert_eq!(summary.parse_error, None);
}

#[test]
fn each_header_layer_reports_its_parse_failure_without_a_protocol_refusal() {
    let frames = [vec![0x81], bip(&[0], false), bip(&[1, 0, 0xff], false)];
    for frame in frames {
        let summary = summarize_frame(Framing::Bip, &frame);
        assert!(summary.parse_error.is_some());
        assert_eq!(summary.problem, summary.parse_error);
    }
}

#[test]
fn an_invalid_apdu_keeps_decoded_routing_metadata() {
    let data = routed_npdu(&i_am(123), false, Some((7, &[5])));
    let mut data = decode_npdu(Bytes::from(data)).unwrap();
    data.payload = Bytes::from_static(&[0xff]);
    data.destination = Some(NpduAddress {
        network: 8,
        mac_address: bacnet_types::MacAddr::new(),
    });
    let mut wire = BytesMut::new();
    encode_npdu(&mut wire, &data).unwrap();
    let summary = summarize_frame(Framing::Bip, &bip(&wire, false));
    assert!(summary.parse_error.is_some());
    assert_eq!(summary.snet, Some(7));
    assert_eq!(summary.dnet, Some(8));
    assert_eq!(summary.routed_source.as_deref(), Some("7:0x05"));
    assert_eq!(summary.routed_destination.as_deref(), Some("8:*"));
    assert!(summary.broadcast);
}

proptest::proptest! {
    #[test]
    fn arbitrary_frame_octets_do_not_panic(
        octets in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512),
    ) {
        for framing in [Framing::Bip, Framing::Bip6, Framing::Sc, Framing::Mstp] {
            let summary = summarize_frame(framing, &octets);
            if summary.parse_error.is_some() {
                proptest::prop_assert!(summary.problem.is_some());
            }
        }
    }
}

#[test]
fn malformed_sc_result_is_not_mistaken_for_a_valid_acknowledgment() {
    use bacnet_transport::sc_frame::{ScFunction, ScMessage, encode_sc_message};
    let message = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &message);
    let summary = summarize_frame(Framing::Sc, &wire);
    assert!(summary.parse_error.is_some());
    assert_eq!(summary.parse_error, summary.problem);
}

/// An NPDU to wrap: a random control octet and tail, or a local NPDU whose
/// tail is a random APDU, so cases reach both the NPDU and APDU decoders.
fn npdu_tail() -> impl proptest::strategy::Strategy<Value = Vec<u8>> {
    use proptest::prelude::*;
    let octets = || proptest::collection::vec(any::<u8>(), 0..700);
    prop_oneof![
        (any::<u8>(), octets()).prop_map(|(control, tail)| [vec![0x01, control], tail].concat()),
        (any::<bool>(), octets()).prop_map(|(reply, apdu)| [
            vec![0x01, u8::from(reply) << 2],
            apdu
        ]
        .concat()),
    ]
}

proptest::proptest! {
    /// Valid B/IP, B/IPv6, BACnet/SC and MS/TP headers (standard, or
    /// extended past 501 octets) around a random NPDU: no panic, the link
    /// header always reads, and a parse failure comes from the NPDU or APDU.
    #[test]
    fn random_npdus_behind_valid_link_headers_decode_without_panic(
        npdu in npdu_tail(),
        broadcast in proptest::prelude::any::<bool>(),
    ) {
        use bacnet_transport::mstp::codec::FrameType;
        let mstp_type = match (npdu.len() > 501, broadcast) {
            (false, false) => FrameType::BACNET_DATA_EXPECTING_REPLY,
            (false, true) => FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            (true, false) => FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
            (true, true) => FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        };
        let frames = [
            (Framing::Bip, bip(&npdu, broadcast)),
            (Framing::Bip6, bip6_broadcast(&npdu)),
            (Framing::Sc, sc_npdu(&npdu, broadcast.then_some([0xFF; 6]))),
            (Framing::Mstp, mstp(mstp_type, if broadcast { 255 } else { 5 }, 1, &npdu)),
        ];
        // The message each header names: exact, so a header that fails to
        // read fails here (an MS/TP frame's type is named before its CRCs
        // are checked, so `malformed` alone would not catch one there; the
        // parse_error check below does).
        let messages = [
            if broadcast {
                "original-broadcast-npdu"
            } else {
                "original-unicast-npdu"
            }
            .to_string(),
            "original-broadcast-npdu".to_string(),
            "encapsulated-npdu".to_string(),
            kebab(mstp_type.name().unwrap()),
        ];
        for ((framing, frame), message) in frames.into_iter().zip(messages) {
            let summary = summarize_frame(framing, &frame);
            proptest::prop_assert_eq!(&summary.link_message, &message, "{:?}", framing);
            if let Some(error) = &summary.parse_error {
                proptest::prop_assert!(
                    error.contains("NPDU decode error") || error.contains("APDU decode error"),
                    "{:?}: {}",
                    framing,
                    error
                );
                proptest::prop_assert_eq!(&summary.problem, &summary.parse_error);
            } else {
                // The decoders reached the APDU, or there was none to reach.
                let empty = decode_npdu(Bytes::from(npdu.clone()))
                    .is_ok_and(|npdu| npdu.payload.is_empty());
                proptest::prop_assert!(
                    summary.pdu.is_some() || summary.network_message.is_some() || empty,
                    "{:?}: {:?}",
                    framing,
                    summary
                );
            }
        }
    }
}

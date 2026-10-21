//! The NPDU view and its encoder: what they accept, what they refuse, and
//! that one undoes the other.

use bacnet_npdu::{
    Address, AddressField, DecodeError, EncodeError, Malformed, NetworkMessageType,
    NetworkPriority, Npdu, MAX_MAC_LEN,
};
use proptest::prelude::*;
use proptest::sample::Index;

/// A network number, weighted toward the reserved and boundary ones.
fn network() -> impl Strategy<Value = u16> {
    prop_oneof![
        Just(0u16),
        Just(1),
        Just(0xFFFE),
        Just(0xFFFF),
        any::<u16>()
    ]
}

/// A length octet and the octets it announces, usually agreeing; otherwise
/// any length, past the bound or not, with whatever octets follow.
fn address() -> impl Strategy<Value = (u16, u8, Vec<u8>)> {
    (
        network(),
        prop::collection::vec(any::<u8>(), 0..=MAX_MAC_LEN + 2),
        prop_oneof![3 => Just(None), 1 => prop_oneof![0u8..=20, any::<u8>()].prop_map(Some)],
    )
        .prop_map(|(network, mac, length)| {
            let length = length.unwrap_or(mac.len() as u8);
            (network, length, mac)
        })
}

/// Frames laid out field by field from the control octet, with any field's
/// value, often cut short at a random point.
fn frame() -> impl Strategy<Value = Vec<u8>> {
    (
        prop_oneof![9 => Just(1u8), 1 => any::<u8>()],
        any::<u8>(),
        address(),
        address(),
        any::<u8>(),
        prop_oneof![any::<u8>(), 0x80u8..=0xFF],
        prop::collection::vec(any::<u8>(), 0..8),
        prop_oneof![3 => Just(None), 1 => any::<Index>().prop_map(Some)],
    )
        .prop_map(
            |(version, control, destination, source, hops, kind, tail, cut)| {
                let mut frame = vec![version, control];
                for (bit, (network, length, mac)) in [(0x20, destination), (0x08, source)] {
                    if control & bit != 0 {
                        frame.extend_from_slice(&network.to_be_bytes());
                        frame.push(length);
                        frame.extend_from_slice(&mac);
                    }
                }
                if control & 0x20 != 0 {
                    frame.push(hops);
                }
                if control & 0x80 != 0 {
                    frame.push(kind);
                }
                frame.extend_from_slice(&tail);
                if let Some(cut) = cut {
                    frame.truncate(cut.index(frame.len() + 1));
                }
                frame
            },
        )
}

/// What any frame the view accepts must satisfy: the payload is the rest of
/// the frame, the fields obey the rules the decoder enforces, and encoding
/// the view gives the frame back but for its reserved control bits.
fn consistent(frame: &[u8]) -> Result<(), TestCaseError> {
    match Npdu::decode(frame) {
        Ok(npdu) => {
            prop_assert_eq!(npdu.payload_offset + npdu.payload.len(), frame.len());
            prop_assert_eq!(npdu.payload, &frame[npdu.payload_offset..]);
            prop_assert_eq!(npdu.priority.to_raw(), frame[1] & 0x03);
            prop_assert_eq!(npdu.expecting_reply, frame[1] & 0x04 != 0);
            prop_assert_eq!(npdu.message_type.is_some(), frame[1] & 0x80 != 0);
            if let Some(destination) = npdu.destination {
                prop_assert!(destination.network != 0 && destination.mac.len() <= MAX_MAC_LEN);
            } else {
                prop_assert_eq!(npdu.hop_count, 255);
            }
            if let Some(source) = npdu.source {
                prop_assert!(source.network != 0 && source.network != 0xFFFF);
                prop_assert!(!source.mac.is_empty() && source.mac.len() <= MAX_MAC_LEN);
            }
            prop_assert_eq!(
                npdu.vendor_id.is_some(),
                npdu.message_type
                    .is_some_and(NetworkMessageType::is_proprietary)
            );
            let mut out = vec![0u8; frame.len()];
            let len = npdu.encode_into(&mut out).unwrap();
            prop_assert_eq!(len, frame.len());
            prop_assert_eq!(out[0], frame[0]);
            prop_assert_eq!(out[1], frame[1] & !0x50);
            prop_assert_eq!(&out[2..], &frame[2..]);
        }
        Err(DecodeError::AddressTooLong { length, .. }) => {
            prop_assert!(usize::from(length) > MAX_MAC_LEN);
        }
        Err(DecodeError::Malformed(_)) => {}
    }
    Ok(())
}

/// A valid NPDU for the encoder: any fields the decoder accepts.
#[derive(Debug, Clone)]
struct Fields {
    expecting_reply: bool,
    priority: u8,
    destination: Option<(u16, Vec<u8>)>,
    source: Option<(u16, Vec<u8>)>,
    hop_count: u8,
    message: Option<(u8, u16)>,
    payload: Vec<u8>,
}

impl Fields {
    fn view(&self) -> Npdu<'_> {
        fn address((network, mac): &(u16, Vec<u8>)) -> Address<'_> {
            Address {
                network: *network,
                mac: mac.as_slice(),
            }
        }
        let destination = self.destination.as_ref().map(address);
        Npdu {
            expecting_reply: self.expecting_reply,
            priority: NetworkPriority::try_from(self.priority).unwrap(),
            destination,
            source: self.source.as_ref().map(address),
            hop_count: if destination.is_some() {
                self.hop_count
            } else {
                255
            },
            message_type: self
                .message
                .map(|(kind, _)| NetworkMessageType::from_raw(kind)),
            vendor_id: self
                .message
                .and_then(|(kind, vendor)| (kind >= 0x80).then_some(vendor)),
            payload: &self.payload,
            payload_offset: 0,
        }
    }
}

fn fields() -> impl Strategy<Value = Fields> {
    let mac = |least| prop::collection::vec(any::<u8>(), least..=MAX_MAC_LEN);
    (
        any::<bool>(),
        0u8..=3,
        prop::option::of((1u16..=0xFFFF, mac(0))),
        prop::option::of((1u16..=0xFFFE, mac(1))),
        any::<u8>(),
        prop::option::of((any::<u8>(), any::<u16>())),
        prop::collection::vec(any::<u8>(), 0..32),
    )
        .prop_map(
            |(expecting_reply, priority, destination, source, hop_count, message, payload)| {
                Fields {
                    expecting_reply,
                    priority,
                    destination,
                    source,
                    hop_count,
                    message,
                    payload,
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// Laid-out frames, valid and not, get a verdict consistent with their
    /// octets, and every accepted one encodes back to itself.
    #[test]
    fn decoded_frames_are_consistent(frame in frame()) {
        consistent(&frame)?;
    }

    /// So do arbitrary octets.
    #[test]
    fn noise_is_decoded_consistently(
        frame in prop::collection::vec(any::<u8>(), 0..48),
        version_one in any::<bool>(),
    ) {
        let mut frame = frame;
        if version_one && !frame.is_empty() {
            frame[0] = 1;
        }
        consistent(&frame)?;
    }

    /// The encoder writes `encoded_len` octets, which decode to the fields
    /// written.
    #[test]
    fn encode_then_decode_round_trips(fields in fields()) {
        let view = fields.view();
        let mut out = vec![0u8; view.encoded_len() + 3];
        let len = view.encode_into(&mut out).unwrap();
        prop_assert_eq!(len, view.encoded_len());
        let decoded = Npdu::decode(&out[..len]).unwrap();
        prop_assert_eq!(decoded, Npdu { payload_offset: len - view.payload.len(), ..view });
    }

    /// A buffer one octet short gets nothing written.
    #[test]
    fn encoder_refuses_a_short_buffer_whole(fields in fields()) {
        let view = fields.view();
        let needed = view.encoded_len();
        let mut out = vec![0xA5u8; needed - 1];
        prop_assert_eq!(
            view.encode_into(&mut out),
            Err(EncodeError::NoRoom { needed, available: needed - 1 })
        );
        prop_assert!(out.iter().all(|&octet| octet == 0xA5));
    }
}

#[test]
fn every_field_lands_where_the_control_octet_says() {
    // Network message 0x81 (proprietary, vendor 0x0102) from SNET 7 SADR
    // 0x0A, to DNET 0x0300 DADR 01 02, hop count 9, expecting a reply, at
    // life-safety priority.
    let frame = [
        0x01, 0xAF, 0x03, 0x00, 0x02, 0x01, 0x02, 0x00, 0x07, 0x01, 0x0A, 0x09, 0x81, 0x01, 0x02,
        0xEE,
    ];
    let npdu = Npdu::decode(&frame).unwrap();
    assert_eq!(
        npdu,
        Npdu {
            expecting_reply: true,
            priority: NetworkPriority::LifeSafety,
            destination: Some(Address {
                network: 0x0300,
                mac: &[1, 2]
            }),
            source: Some(Address {
                network: 7,
                mac: &[0x0A]
            }),
            hop_count: 9,
            message_type: Some(NetworkMessageType::from_raw(0x81)),
            vendor_id: Some(0x0102),
            payload: &[0xEE],
            payload_offset: 15,
        }
    );
}

#[test]
fn every_priority_decodes_from_its_two_bits() {
    for (bits, priority) in [
        (0, NetworkPriority::Normal),
        (1, NetworkPriority::Urgent),
        (2, NetworkPriority::CriticalEquipment),
        (3, NetworkPriority::LifeSafety),
    ] {
        let frame = [0x01, 0x04 | bits, 0x10, 0x08];
        let npdu = Npdu::decode(&frame).unwrap();
        assert_eq!(npdu.priority, priority);
        let mut out = [0u8; 4];
        assert_eq!(npdu.encode_into(&mut out), Ok(4));
        assert_eq!(out, frame);
    }
    assert_eq!(NetworkPriority::try_from(4).unwrap_err().value, 4);
    assert_eq!(
        NetworkPriority::try_from(4).unwrap_err().to_string(),
        "network priority 4 is not 0 to 3"
    );
}

#[test]
fn over_long_addresses_carry_what_a_reject_needs() {
    // DNET 9, DLEN 19 (and 19 DADR octets), then SNET 1, SLEN 1, SADR 0x0A.
    let mut frame = vec![0x01, 0x28, 0x00, 0x09, 19];
    frame.extend_from_slice(&[0xEE; 19]);
    frame.extend_from_slice(&[0x00, 0x01, 0x01, 0x0A, 0xFF, 0x10, 0x08]);
    assert_eq!(
        Npdu::decode(&frame),
        Err(DecodeError::AddressTooLong {
            field: AddressField::Destination,
            length: 19,
            dnet: Some(9),
            source: Some(Address {
                network: 1,
                mac: &[0x0A],
            }),
        })
    );

    // The same with the frame ending inside that source: no source to name.
    let cut = &frame[..frame.len() - 4];
    assert!(matches!(
        Npdu::decode(cut),
        Err(DecodeError::AddressTooLong { source: None, .. })
    ));

    // An over-long SLEN: the DNET is known, and the source names no node.
    let frame = [0x01, 0x28, 0x00, 0x09, 0x01, 0x05, 0x00, 0x01, 200, 0xFF];
    assert_eq!(
        Npdu::decode(&frame),
        Err(DecodeError::AddressTooLong {
            field: AddressField::Source,
            length: 200,
            dnet: Some(9),
            source: None,
        })
    );
    assert_eq!(
        Npdu::decode(&frame).unwrap_err().to_string(),
        "NPDU SLEN=200 exceeds the 18-octet address limit"
    );

    // A global broadcast with an over-long DADR still reports DNET 0xFFFF,
    // which a router answers with no reject.
    let frame = [0x01, 0x20, 0xFF, 0xFF, 19];
    assert!(matches!(
        Npdu::decode(&frame),
        Err(DecodeError::AddressTooLong {
            dnet: Some(0xFFFF),
            ..
        })
    ));
}

/// DNET 9 with a 19-octet DADR, then SNET `snet` with an SADR of `sadr`
/// octets (SLEN = `sadr`), a hop count and an APDU.
fn behind_a_long_dadr(snet: u16, sadr: u8) -> Vec<u8> {
    let mut frame = vec![0x01, 0x28, 0x00, 0x09, 19];
    frame.extend_from_slice(&[0xEE; 19]);
    frame.extend_from_slice(&snet.to_be_bytes());
    frame.push(sadr);
    frame.extend((0..sadr).map(|octet| octet + 1));
    frame.extend_from_slice(&[0xFF, 0x10, 0x08]);
    frame
}

#[test]
fn only_a_source_the_decoder_takes_is_named_behind_a_long_dadr() {
    let source = |frame: &[u8]| match Npdu::decode(frame) {
        Err(DecodeError::AddressTooLong {
            field: AddressField::Destination,
            source,
            ..
        }) => source.map(|source| (source.network, source.mac.len())),
        other => panic!("{other:?}"),
    };
    // The longest SADR the bound allows is named whole.
    assert_eq!(source(&behind_a_long_dadr(1, 18)), Some((1, 18)));
    // One octet more, an empty SADR, or a reserved SNET names nothing.
    for frame in [
        behind_a_long_dadr(1, 19),
        behind_a_long_dadr(1, 0),
        behind_a_long_dadr(0, 1),
        behind_a_long_dadr(0xFFFF, 1),
    ] {
        assert_eq!(source(&frame), None, "{frame:02x?}");
    }
}

#[test]
fn malformations_are_named() {
    let cases: [(&[u8], Malformed); 10] = [
        (&[], Malformed::Truncated),
        (&[0x01], Malformed::Truncated),
        (&[0x02, 0x00], Malformed::Version),
        (
            &[0x01, 0x20, 0x00, 0x00, 0x00, 0xFF],
            Malformed::ReservedNetwork,
        ),
        (
            &[0x01, 0x08, 0xFF, 0xFF, 0x01, 0x05],
            Malformed::ReservedNetwork,
        ),
        (&[0x01, 0x08, 0x00, 0x01, 0x00], Malformed::EmptySource),
        (&[0x01, 0x20, 0xFF, 0xFF, 0x00], Malformed::Truncated),
        (
            &[0x01, 0x20, 0x03, 0xE8, 0x06, 0x01, 0x02],
            Malformed::Truncated,
        ),
        (&[0x01, 0x80], Malformed::Truncated),
        (&[0x01, 0x80, 0x80, 0x00], Malformed::Truncated),
    ];
    for (frame, fault) in cases {
        assert_eq!(
            Npdu::decode(frame),
            Err(DecodeError::Malformed(fault)),
            "{frame:02x?}"
        );
    }
}

#[test]
fn reserved_control_bits_are_read_past_and_not_written() {
    let frame = [0x01, 0x50 | 0x04 | 0x02, 0x10, 0x08];
    let npdu = Npdu::decode(&frame).unwrap();
    assert!(npdu.expecting_reply);
    assert_eq!(npdu.priority, NetworkPriority::CriticalEquipment);
    assert_eq!(npdu.payload, &[0x10, 0x08]);
    assert_eq!(npdu.payload_offset, 2);
    let mut out = [0u8; 8];
    let len = npdu.encode_into(&mut out).unwrap();
    assert_eq!(&out[..len], &[0x01, 0x06, 0x10, 0x08]);
}

#[test]
fn encoder_refuses_what_no_decoder_accepts() {
    let long = [0u8; MAX_MAC_LEN + 1];
    let at = |network, mac| Some(Address { network, mac });
    let mut out = [0u8; 64];
    let cases = [
        (
            Npdu {
                destination: at(0, &[1][..]),
                ..Npdu::apdu(&[])
            },
            EncodeError::ReservedNetwork,
        ),
        (
            Npdu {
                source: at(0, &[1][..]),
                ..Npdu::apdu(&[])
            },
            EncodeError::ReservedNetwork,
        ),
        (
            Npdu {
                source: at(0xFFFF, &[1][..]),
                ..Npdu::apdu(&[])
            },
            EncodeError::ReservedNetwork,
        ),
        (
            Npdu {
                source: at(5, &[][..]),
                ..Npdu::apdu(&[])
            },
            EncodeError::EmptySource,
        ),
        (
            Npdu {
                destination: at(5, &long[..]),
                ..Npdu::apdu(&[])
            },
            EncodeError::AddressTooLong,
        ),
        (
            Npdu {
                source: at(5, &long[..]),
                ..Npdu::apdu(&[])
            },
            EncodeError::AddressTooLong,
        ),
    ];
    for (npdu, error) in cases {
        assert_eq!(npdu.encode_into(&mut out), Err(error));
        assert!(out.iter().all(|&octet| octet == 0));
    }
}

#[test]
fn constructors_build_local_npdus() {
    let apdu = Npdu::apdu(&[0x10, 0x08]);
    let mut out = [0u8; 8];
    let len = apdu.encode_into(&mut out).unwrap();
    assert_eq!(&out[..len], &[0x01, 0x00, 0x10, 0x08]);
    assert!(!apdu.is_network_message() && !apdu.is_global());

    let message = Npdu::message(NetworkMessageType::from_raw(0x12), &[]);
    let len = message.encode_into(&mut out).unwrap();
    assert_eq!(&out[..len], &[0x01, 0x80, 0x12]);
    assert!(message.is_network_message());

    let global = Npdu::decode(&[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08]).unwrap();
    assert!(global.is_global());
}

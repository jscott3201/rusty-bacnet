//! The NPDU view against `bacnet-encoding`'s decoder and encoder.
//!
//! The router core carries its own allocation-free parser beside
//! `decode_npdu`, so these properties hold the two equal: every frame gets
//! the same verdict and the same fields from both, and everything the
//! writer produces is what `encode_npdu` produces for the same NPDU.

use bacnet_encoding::npdu::{
    decode_npdu, encode_npdu, Npdu as Decoded, NpduAddress, NpduAddressField, NpduDecodeError,
};
use bacnet_router_core::message::MessageType;
use bacnet_router_core::npdu::{Address, AddressField, Malformed, Npdu, ParseError, WriteError};
use bacnet_router_core::MAX_MAC_LEN;
use bacnet_types::enums::NetworkPriority;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use proptest::prelude::*;
use proptest::sample::Index;

// The core refuses exactly the addresses the hosted codec refuses.
const _: () = assert!(MAX_MAC_LEN == NpduAddress::MAX_MAC_LEN);

fn same_address(view: Option<Address<'_>>, decoded: Option<&NpduAddress>) -> bool {
    match (view, decoded) {
        (None, None) => true,
        (Some(view), Some(decoded)) => {
            view.network == decoded.network && view.mac == decoded.mac_address.as_slice()
        }
        _ => false,
    }
}

/// Both parsers' verdicts on `frame`, held equal.
fn agree(frame: &[u8]) -> Result<(), TestCaseError> {
    let view = Npdu::parse(frame);
    let decoded = decode_npdu(Bytes::copy_from_slice(frame));
    match (view, decoded) {
        (Ok(view), Ok(decoded)) => {
            prop_assert_eq!(view.message_type.is_some(), decoded.is_network_message);
            prop_assert_eq!(view.expecting_reply, decoded.expecting_reply);
            prop_assert_eq!(view.priority, decoded.priority.to_raw());
            prop_assert!(same_address(view.destination, decoded.destination.as_ref()));
            prop_assert!(same_address(view.source, decoded.source.as_ref()));
            prop_assert_eq!(view.hop_count, decoded.hop_count);
            prop_assert_eq!(
                view.message_type.map(MessageType::to_raw),
                decoded.message_type
            );
            prop_assert_eq!(view.vendor_id, decoded.vendor_id);
            prop_assert_eq!(view.payload, &decoded.payload[..]);
            prop_assert_eq!(view.payload_offset + view.payload.len(), frame.len());
        }
        (
            Err(ParseError::AddressTooLong {
                field,
                length,
                dnet,
                source,
            }),
            Err(NpduDecodeError::AddressTooLong {
                field: decoded_field,
                length: decoded_length,
                dnet: decoded_dnet,
                source: decoded_source,
            }),
        ) => {
            let field_matches = matches!(
                (field, decoded_field),
                (AddressField::Destination, NpduAddressField::Destination)
                    | (AddressField::Source, NpduAddressField::Source)
            );
            prop_assert!(field_matches);
            prop_assert_eq!(length, decoded_length);
            prop_assert_eq!(dnet, decoded_dnet);
            prop_assert!(same_address(source, decoded_source.as_ref()));
        }
        (Err(ParseError::Malformed(_)), Err(NpduDecodeError::Malformed(_))) => {}
        (view, decoded) => {
            prop_assert!(false, "view {view:?}, decoder {decoded:?}");
        }
    }
    Ok(())
}

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

/// A valid NPDU for the writer: any fields `encode_npdu` accepts.
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
            priority: self.priority,
            destination,
            source: self.source.as_ref().map(address),
            hop_count: if destination.is_some() {
                self.hop_count
            } else {
                255
            },
            message_type: self.message.map(|(kind, _)| MessageType::from_raw(kind)),
            vendor_id: self
                .message
                .and_then(|(kind, vendor)| (kind >= 0x80).then_some(vendor)),
            payload: &self.payload,
            payload_offset: 0,
        }
    }

    fn decoded(&self) -> Decoded {
        let view = self.view();
        let address = |address: Address<'_>| NpduAddress {
            network: address.network,
            mac_address: MacAddr::from_slice(address.mac),
        };
        Decoded {
            is_network_message: view.message_type.is_some(),
            expecting_reply: view.expecting_reply,
            priority: NetworkPriority::from_raw(view.priority),
            destination: view.destination.map(address),
            source: view.source.map(address),
            hop_count: view.hop_count,
            message_type: view.message_type.map(MessageType::to_raw),
            vendor_id: view.vendor_id,
            payload: Bytes::copy_from_slice(view.payload),
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

    /// Laid-out frames, valid and not, get one verdict and one set of fields
    /// from both parsers, including where an over-long address reports its
    /// DNET and source.
    #[test]
    fn view_agrees_with_decode_npdu(frame in frame()) {
        agree(&frame)?;
    }

    /// So do arbitrary octets.
    #[test]
    fn view_agrees_with_decode_npdu_on_noise(
        frame in prop::collection::vec(any::<u8>(), 0..48),
        version_one in any::<bool>(),
    ) {
        let mut frame = frame;
        if version_one && !frame.is_empty() {
            frame[0] = 1;
        }
        agree(&frame)?;
    }

    /// The writer produces `encode_npdu`'s octets, which both parsers read
    /// back as the fields written, and `encoded_len` is their length.
    #[test]
    fn writer_matches_encode_npdu(fields in fields()) {
        let view = fields.view();
        let mut expected = BytesMut::new();
        encode_npdu(&mut expected, &fields.decoded()).unwrap();

        let mut out = vec![0u8; view.encoded_len() + 3];
        let len = view.write(&mut out).unwrap();
        prop_assert_eq!(len, view.encoded_len());
        prop_assert_eq!(&out[..len], &expected[..]);

        let decoded = decode_npdu(Bytes::copy_from_slice(&out[..len])).unwrap();
        prop_assert_eq!(decoded, fields.decoded());
        let parsed = Npdu::parse(&out[..len]).unwrap();
        prop_assert_eq!(parsed, Npdu { payload_offset: len - view.payload.len(), ..view });
    }

    /// A buffer one octet short gets nothing written.
    #[test]
    fn writer_refuses_a_short_buffer_whole(fields in fields()) {
        let view = fields.view();
        let needed = view.encoded_len();
        let mut out = vec![0xA5u8; needed - 1];
        prop_assert_eq!(
            view.write(&mut out),
            Err(WriteError::NoRoom { needed, available: needed - 1 })
        );
        prop_assert!(out.iter().all(|&octet| octet == 0xA5));
    }
}

#[test]
fn over_long_addresses_carry_what_a_reject_needs() {
    // DNET 9, DLEN 19 (and 19 DADR octets), then SNET 1, SLEN 1, SADR 0x0A.
    let mut frame = vec![0x01, 0x28, 0x00, 0x09, 19];
    frame.extend_from_slice(&[0xEE; 19]);
    frame.extend_from_slice(&[0x00, 0x01, 0x01, 0x0A, 0xFF, 0x10, 0x08]);
    assert_eq!(
        Npdu::parse(&frame),
        Err(ParseError::AddressTooLong {
            field: AddressField::Destination,
            length: 19,
            dnet: Some(9),
            source: Some(Address {
                network: 1,
                mac: &[0x0A],
            }),
        })
    );
    agree(&frame).unwrap();

    // The same with the frame ending inside that source: no source to name.
    let cut = &frame[..frame.len() - 4];
    assert!(matches!(
        Npdu::parse(cut),
        Err(ParseError::AddressTooLong { source: None, .. })
    ));
    agree(cut).unwrap();

    // An over-long SLEN: the DNET is known, and the source names no node.
    let frame = [0x01, 0x28, 0x00, 0x09, 0x01, 0x05, 0x00, 0x01, 200, 0xFF];
    assert_eq!(
        Npdu::parse(&frame),
        Err(ParseError::AddressTooLong {
            field: AddressField::Source,
            length: 200,
            dnet: Some(9),
            source: None,
        })
    );
    agree(&frame).unwrap();

    // A global broadcast with an over-long DADR still reports DNET 0xFFFF,
    // which the router answers with no reject.
    let frame = [0x01, 0x20, 0xFF, 0xFF, 19];
    assert!(matches!(
        Npdu::parse(&frame),
        Err(ParseError::AddressTooLong {
            dnet: Some(0xFFFF),
            ..
        })
    ));
    agree(&frame).unwrap();
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
fn only_a_source_the_parser_takes_is_named_behind_a_long_dadr() {
    let source = |frame: &[u8]| match Npdu::parse(frame) {
        Err(ParseError::AddressTooLong {
            field: AddressField::Destination,
            source,
            ..
        }) => source.map(|source| (source.network, source.mac.len())),
        other => panic!("{other:?}"),
    };
    // The longest SADR the bound allows is named whole.
    let frame = behind_a_long_dadr(1, 18);
    assert_eq!(source(&frame), Some((1, 18)));
    agree(&frame).unwrap();
    // One octet more, an empty SADR, or a reserved SNET names nothing.
    for frame in [
        behind_a_long_dadr(1, 19),
        behind_a_long_dadr(1, 0),
        behind_a_long_dadr(0, 1),
        behind_a_long_dadr(0xFFFF, 1),
    ] {
        assert_eq!(source(&frame), None, "{frame:02x?}");
        agree(&frame).unwrap();
    }
}

#[test]
fn malformations_are_named() {
    let cases: [(&[u8], Malformed); 7] = [
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
        (&[0x01, 0x80, 0x80, 0x00], Malformed::Truncated),
    ];
    for (frame, fault) in cases {
        assert_eq!(
            Npdu::parse(frame),
            Err(ParseError::Malformed(fault)),
            "{frame:02x?}"
        );
        agree(frame).unwrap();
    }
}

#[test]
fn reserved_control_bits_are_read_past_and_not_written() {
    let frame = [0x01, 0x50 | 0x04 | 0x02, 0x10, 0x08];
    let npdu = Npdu::parse(&frame).unwrap();
    assert!(npdu.expecting_reply);
    assert_eq!(npdu.priority, 2);
    assert_eq!(npdu.payload, &[0x10, 0x08]);
    assert_eq!(npdu.payload_offset, 2);
    let mut out = [0u8; 8];
    let len = npdu.write(&mut out).unwrap();
    assert_eq!(&out[..len], &[0x01, 0x06, 0x10, 0x08]);
}

#[test]
fn writer_refuses_what_no_parser_accepts() {
    let long = [0u8; MAX_MAC_LEN + 1];
    let at = |network, mac| Some(Address { network, mac });
    let mut out = [0u8; 64];
    let cases = [
        (
            Npdu {
                destination: at(0, &[1][..]),
                ..Npdu::apdu(&[])
            },
            WriteError::ReservedNetwork,
        ),
        (
            Npdu {
                source: at(0, &[1][..]),
                ..Npdu::apdu(&[])
            },
            WriteError::ReservedNetwork,
        ),
        (
            Npdu {
                source: at(0xFFFF, &[1][..]),
                ..Npdu::apdu(&[])
            },
            WriteError::ReservedNetwork,
        ),
        (
            Npdu {
                source: at(5, &[][..]),
                ..Npdu::apdu(&[])
            },
            WriteError::EmptySource,
        ),
        (
            Npdu {
                destination: at(5, &long[..]),
                ..Npdu::apdu(&[])
            },
            WriteError::AddressTooLong,
        ),
        (
            Npdu {
                source: at(5, &long[..]),
                ..Npdu::apdu(&[])
            },
            WriteError::AddressTooLong,
        ),
    ];
    for (npdu, error) in cases {
        assert_eq!(npdu.write(&mut out), Err(error));
        assert!(out.iter().all(|&octet| octet == 0));
        // encode_npdu refuses each one too.
        let decoded = Decoded {
            destination: npdu.destination.map(|a| NpduAddress {
                network: a.network,
                mac_address: MacAddr::from_slice(a.mac),
            }),
            source: npdu.source.map(|a| NpduAddress {
                network: a.network,
                mac_address: MacAddr::from_slice(a.mac),
            }),
            ..Decoded::default()
        };
        assert!(encode_npdu(&mut BytesMut::new(), &decoded).is_err());
    }
}

#[test]
fn constructors_build_local_npdus() {
    let apdu = Npdu::apdu(&[0x10, 0x08]);
    let mut out = [0u8; 8];
    let len = apdu.write(&mut out).unwrap();
    assert_eq!(&out[..len], &[0x01, 0x00, 0x10, 0x08]);
    assert!(!apdu.is_network_message() && !apdu.is_global());

    let message = Npdu::message(MessageType::from_raw(0x12), &[]);
    let len = message.write(&mut out).unwrap();
    assert_eq!(&out[..len], &[0x01, 0x80, 0x12]);
    assert!(message.is_network_message());

    let global = Npdu::parse(&[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08]).unwrap();
    assert!(global.is_global());
}

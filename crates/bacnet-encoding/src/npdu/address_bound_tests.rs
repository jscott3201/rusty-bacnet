//! Wire-level tests for the DADR/SADR length bound (#1141): a DLEN or SLEN
//! past [`MAX_MAC_LEN`] is refused in both directions, the
//! lengths up to it still work, and both addresses behave the same way.

use super::*;
use bacnet_npdu::AddressField;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::error::DecodingKind;

const FIELDS: [AddressField; 2] = [AddressField::Destination, AddressField::Source];
const DNET: u16 = 2000;
const SNET: u16 = 3000;
const APDU: [u8; 2] = [0x10, 0x08];

/// Hand-built NPDU wire bytes (the encoder refuses the lengths under test).
/// The `field` address announces `length` octets but holds only `present` of
/// them; the other address, when the frame has one, is a valid 1-octet MAC.
/// A source-field frame also carries a destination when `routed` is set.
fn frame(field: AddressField, length: u8, present: u8, routed: bool) -> Vec<u8> {
    let address = |first: u8| (0..present).map(move |i| first.wrapping_add(i));
    let mut out = vec![bacnet_npdu::VERSION];
    match field {
        AddressField::Destination => {
            out.push(0x20);
            out.extend_from_slice(&DNET.to_be_bytes());
            out.push(length);
            out.extend(address(0xD0));
        }
        AddressField::Source => {
            if routed {
                out.push(0x28);
                out.extend_from_slice(&DNET.to_be_bytes());
                out.extend_from_slice(&[1, 0xD0]);
            } else {
                out.push(0x08);
            }
            out.extend_from_slice(&SNET.to_be_bytes());
            out.push(length);
            out.extend(address(0x50));
        }
    }
    if field == AddressField::Destination || routed {
        out.push(255); // hop count
    }
    out.extend_from_slice(&APDU);
    out
}

/// A complete frame whose `field` address is `length` octets long.
fn full(field: AddressField, length: u8) -> Vec<u8> {
    frame(field, length, length, false)
}

fn decode(bytes: Vec<u8>) -> Result<Npdu, NpduDecodeError> {
    decode_npdu(Bytes::from(bytes))
}

fn expected_dnet(field: AddressField) -> Option<u16> {
    (field == AddressField::Destination).then_some(DNET)
}

fn address_of(npdu: &Npdu, field: AddressField) -> &NpduAddress {
    match field {
        AddressField::Destination => npdu.destination.as_ref(),
        AddressField::Source => npdu.source.as_ref(),
    }
    .expect("decoded NPDU carries the address under test")
}

#[test]
fn npdu_address_bound_matches_the_recipient_bound() {
    assert_eq!(MAX_MAC_LEN, 18);
    assert_eq!(MAX_MAC_LEN, BACnetAddress::MAX_MAC_LEN);
}

#[test]
fn over_long_address_length_is_refused_for_either_field() {
    for field in FIELDS {
        for length in [19, 64, 255] {
            match decode(full(field, length)) {
                Err(NpduDecodeError::AddressTooLong {
                    field: refused,
                    length: reported,
                    dnet,
                    source,
                }) => {
                    assert_eq!(refused, field);
                    assert_eq!(reported, length);
                    assert_eq!(dnet, expected_dnet(field), "{field} {length}");
                    assert_eq!(source, None, "{field} {length}: no SNET/SADR to report");
                }
                other => panic!("{field} length {length}: expected AddressTooLong, got {other:?}"),
            }
        }
    }
}

#[test]
fn over_long_length_is_refused_before_the_address_is_read() {
    // Only two of the announced octets are present: the length alone decides,
    // so this is AddressTooLong rather than a truncated address.
    for field in FIELDS {
        let refused = decode(frame(field, 255, 2, false)).unwrap_err();
        assert!(
            matches!(refused, NpduDecodeError::AddressTooLong { length: 255, .. }),
            "{field}: {refused:?}"
        );
        // A length within the bound but past the frame is a truncation.
        let truncated = decode(frame(field, 18, 2, false)).unwrap_err();
        assert!(
            matches!(truncated, NpduDecodeError::Malformed(_)),
            "{field}: {truncated:?}"
        );
    }
}

#[test]
fn boundary_address_lengths_decode_and_round_trip_for_either_field() {
    for field in FIELDS {
        for length in [1, 6, 18] {
            let wire = full(field, length);
            let npdu = decode(wire.clone()).unwrap_or_else(|e| panic!("{field} {length}: {e}"));
            assert_eq!(
                address_of(&npdu, field).mac_address.len(),
                usize::from(length)
            );
            assert_eq!(npdu.payload, APDU[..]);
            let mut again = BytesMut::new();
            encode_npdu(&mut again, &npdu).unwrap();
            assert_eq!(
                again.as_ref(),
                wire,
                "{field} {length} re-encodes unchanged"
            );
        }
    }
}

#[test]
fn source_refusal_reports_the_npdu_dnet() {
    let refused = decode(frame(AddressField::Source, 19, 19, true)).unwrap_err();
    assert!(
        matches!(
            refused,
            NpduDecodeError::AddressTooLong {
                field: AddressField::Source,
                length: 19,
                dnet: Some(DNET),
                source: None,
            }
        ),
        "{refused:?}"
    );
    let accepted = decode(frame(AddressField::Source, 18, 18, true)).unwrap();
    assert_eq!(accepted.destination.unwrap().network, DNET);
    assert_eq!(accepted.source.unwrap().mac_address.len(), 18);
}

/// A frame toward DNET whose DADR announces `dlen` octets and holds `held` of
/// them, followed by `source` written as raw (SNET, SLEN, SADR octets held).
fn behind_long_dadr(dlen: u8, held: u8, source: Option<(u16, u8, u8)>) -> Vec<u8> {
    let mut out = vec![bacnet_npdu::VERSION, 0x20];
    out.extend_from_slice(&DNET.to_be_bytes());
    out.push(dlen);
    out.extend((0..held).map(|i| 0xD0u8.wrapping_add(i)));
    if let Some((snet, slen, sadr_held)) = source {
        out[1] |= 0x08;
        out.extend_from_slice(&snet.to_be_bytes());
        out.push(slen);
        out.extend((0..sadr_held).map(|i| 0x50 + i));
    }
    out.push(255);
    out.extend_from_slice(&APDU);
    out
}

fn reported_source(bytes: Vec<u8>) -> Option<NpduAddress> {
    match decode(bytes) {
        Err(NpduDecodeError::AddressTooLong {
            field: AddressField::Destination,
            dnet: Some(DNET),
            source,
            ..
        }) => source,
        other => panic!("expected a DADR refusal, got {other:?}"),
    }
}

#[test]
fn destination_refusal_carries_the_source_behind_it() {
    // #1158: the SNET/SADR after an over-long DADR is where a router sends
    // its reject, so the refusal reports it.
    for dlen in [19, 255] {
        assert_eq!(
            reported_source(behind_long_dadr(dlen, dlen, Some((SNET, 2, 2)))),
            Some(NpduAddress {
                network: SNET,
                mac_address: MacAddr::from_slice(&[0x50, 0x51]),
            }),
            "DLEN {dlen}"
        );
        let longest = reported_source(behind_long_dadr(dlen, dlen, Some((SNET, 18, 18))));
        assert_eq!(longest.unwrap().mac_address.len(), 18, "DLEN {dlen}");
    }

    // Nothing to report: no source, a DADR the frame does not hold, a reserved
    // SNET, an SLEN of 0 or past the bound, or a truncated SADR.
    for (case, bytes) in [
        ("no source", behind_long_dadr(19, 19, None)),
        ("short DADR", behind_long_dadr(19, 4, Some((SNET, 1, 1)))),
        ("SNET 0", behind_long_dadr(19, 19, Some((0, 1, 1)))),
        (
            "SNET 0xFFFF",
            behind_long_dadr(19, 19, Some((0xFFFF, 1, 1))),
        ),
        ("SLEN 0", behind_long_dadr(19, 19, Some((SNET, 0, 0)))),
        ("SLEN 19", behind_long_dadr(19, 19, Some((SNET, 19, 19)))),
        ("short SADR", behind_long_dadr(19, 19, Some((SNET, 6, 2)))),
    ] {
        assert_eq!(reported_source(bytes), None, "{case}");
    }
}

#[test]
fn address_too_long_reads_as_out_of_range() {
    for field in FIELDS {
        let refused = decode(full(field, 19)).unwrap_err();
        let text = refused.to_string();
        assert!(
            text.contains(&format!("{}=19", field.length_octet())),
            "{text}"
        );
        let converted = Error::from(refused);
        assert!(matches!(converted, Error::Range(_)));
        assert_eq!(converted.to_string(), format!("value out of range: {text}"));
    }
    let malformed = decode(vec![bacnet_npdu::VERSION]).unwrap_err();
    assert_eq!(
        malformed,
        NpduDecodeError::Malformed(bacnet_npdu::Malformed::Truncated)
    );
    let converted = Error::from(malformed);
    assert_eq!(
        converted.decode_fault().map(|(_, kind)| kind),
        Some(DecodingKind::Missing)
    );
    assert_eq!(
        converted.to_string(),
        "decoding error at offset 0: NPDU truncated"
    );
}

#[test]
fn encoder_refuses_an_address_past_the_bound_for_either_field() {
    for field in FIELDS {
        for length in [MAX_MAC_LEN, MAX_MAC_LEN + 1] {
            let address = NpduAddress {
                network: if field == AddressField::Destination {
                    DNET
                } else {
                    SNET
                },
                mac_address: MacAddr::from_slice(&vec![0x42; length]),
            };
            let npdu = match field {
                AddressField::Destination => Npdu {
                    destination: Some(address),
                    ..Npdu::default()
                },
                AddressField::Source => Npdu {
                    source: Some(address),
                    ..Npdu::default()
                },
            };
            let result = encode_npdu(&mut BytesMut::new(), &npdu);
            if length > MAX_MAC_LEN {
                let text = result.unwrap_err().to_string();
                assert!(
                    text.contains(&format!("{field} address of 19 octets")),
                    "{text}"
                );
            } else {
                result.unwrap_or_else(|e| panic!("{field} {length}: {e}"));
            }
        }
    }
}

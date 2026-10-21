//! The allocation-free APDU header: hand-written octets from Clauses
//! 20.1.2-20.1.9, refusals, and (with alloc) agreement with the owned codec.

use super::*;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, RejectReason, UnconfirmedServiceChoice,
};

const READ_PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

fn request(segmented_response_accepted: bool, max_segments: u8, max_apdu: u8) -> ApduHeader {
    ApduHeader::ConfirmedRequest(ConfirmedRequestHeader {
        segmented_response_accepted,
        max_segments: MaxSegments::from_bits(max_segments),
        max_apdu: MaxApdu::from_bits(max_apdu).unwrap(),
        invoke_id: 42,
        service_choice: READ_PROPERTY,
    })
}

/// Each header with its octets: the PDU type in the top nibble, then the
/// SEG/MOR/SA flags; for a Confirmed-Request the segments and size codes,
/// then the invoke ID and service choice. An Error, a Reject and an Abort
/// carry the invoke ID, then the service, the reason or the reason; an Abort
/// sent by a server sets the low bit of its first octet.
fn goldens() -> [(ApduHeader, &'static [u8]); 9] {
    [
        (request(true, 0, 5), &[0x02, 0x05, 0x2A, 0x0C]),
        (request(false, 7, 0), &[0x00, 0x70, 0x2A, 0x0C]),
        (request(true, 3, 3), &[0x02, 0x33, 0x2A, 0x0C]),
        (
            ApduHeader::UnconfirmedRequest(UnconfirmedServiceChoice::WHO_IS),
            &[0x10, 0x08],
        ),
        (
            ApduHeader::ComplexAck {
                invoke_id: 42,
                service_choice: READ_PROPERTY,
            },
            &[0x30, 0x2A, 0x0C],
        ),
        (
            ApduHeader::Error {
                invoke_id: 42,
                service_choice: READ_PROPERTY,
            },
            &[0x50, 0x2A, 0x0C],
        ),
        (
            ApduHeader::Reject {
                invoke_id: 42,
                reason: RejectReason::UNRECOGNIZED_SERVICE,
            },
            &[0x60, 0x2A, 0x09],
        ),
        (
            ApduHeader::Abort {
                server: true,
                invoke_id: 42,
                reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            },
            &[0x71, 0x2A, 0x04],
        ),
        (
            ApduHeader::Abort {
                server: false,
                invoke_id: 42,
                reason: AbortReason::OTHER,
            },
            &[0x70, 0x2A, 0x00],
        ),
    ]
}

#[test]
fn headers_encode_and_decode_their_octets() {
    for (header, octets) in goldens() {
        let mut storage = [0xA5; 8];
        let mut tail = &mut storage[..];
        assert_eq!(header.encode_to_sink(&mut tail), Ok(octets.len()));
        assert_eq!(&storage[..octets.len()], octets);
        assert_eq!(header.encoded_len(), octets.len());

        let mut apdu = [0; 10];
        apdu[..octets.len()].copy_from_slice(octets);
        apdu[octets.len()..octets.len() + 2].copy_from_slice(&[0x0C, 0x00]);
        let apdu = &apdu[..octets.len() + 2];
        assert_eq!(
            ApduHeader::decode(apdu).unwrap(),
            (header, &[0x0C, 0x00][..])
        );
    }
}

#[test]
fn a_header_and_its_parameters_are_written_whole_or_not_at_all() {
    let (header, octets) = goldens()[0];
    let parameters = [0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55];
    let total = octets.len() + parameters.len();
    for room in 0..total {
        let mut storage = [0xA5; 16];
        let mut tail = &mut storage[..room];
        assert_eq!(
            header.encode_with_parameters_to_sink(&mut tail, &parameters),
            Err(SinkFull {
                needed: total,
                available: room
            })
        );
        assert_eq!(tail.len(), room);
        assert_eq!(storage, [0xA5; 16]);
    }
    let mut storage = [0; 16];
    let mut tail = &mut storage[..];
    assert_eq!(
        header.encode_with_parameters_to_sink(&mut tail, &parameters),
        Ok(total)
    );
    assert_eq!(storage[..4], *octets);
    assert_eq!(storage[4..total], parameters);
}

#[test]
fn every_field_code_round_trips() {
    for accepted in [false, true] {
        for segments in 0..8 {
            for size in 0..6 {
                let header = request(accepted, segments, size);
                let mut storage = [0; 4];
                header.encode_to_sink(&mut &mut storage[..]).unwrap();
                assert_eq!(ApduHeader::decode(&storage).unwrap(), (header, &[][..]));
            }
        }
    }
}

#[test]
fn an_unsegmented_pdu_ignores_its_more_follows_bit() {
    // SEG clear, MOR set (Clause 20.1.2.2).
    let (header, _) = ApduHeader::decode(&[0x06, 0x05, 0x2A, 0x0C]).unwrap();
    assert_eq!(header, request(true, 0, 5));
    let (header, _) = ApduHeader::decode(&[0x34, 0x2A, 0x0C]).unwrap();
    assert_eq!(header, goldens()[4].0);
}

#[test]
fn what_the_header_does_not_read_is_refused() {
    let refused = |apdu: &[u8]| ApduHeader::decode(apdu).unwrap_err();
    let kind = |apdu: &[u8]| refused(apdu).decode_fault().map(|(_, kind)| kind);

    assert_eq!(
        refused(&[]).to_string(),
        "decoding error at offset 0: APDU data is empty"
    );
    for (apdu, need) in [
        (&[0x02, 0x05, 0x2A][..], 4),
        (&[0x10], 2),
        (&[0x30, 0x2A], 3),
    ] {
        assert!(
            matches!(refused(apdu), Error::BufferTooShort { need: n, have } if n == need && have == apdu.len()),
            "{apdu:02X?}"
        );
    }
    // Segmented PDUs.
    for apdu in [
        &[0x0A, 0x05, 0x2A, 0x00, 0x01, 0x0C][..],
        &[0x38, 0x2A, 0x00, 0x01, 0x0C],
    ] {
        assert_eq!(kind(apdu), Some(DecodingKind::Unsupported), "{apdu:02X?}");
    }
    assert_eq!(
        refused(&[0x08, 0x05, 0x2A, 0x00, 0x01, 0x0C]).to_string(),
        "decoding error at offset 0: APDU header: segmented ConfirmedRequest"
    );
    // Reserved max-APDU codes, at the octet that holds them.
    for code in 6..16 {
        assert_eq!(
            refused(&[0x00, code, 0x2A, 0x0C]).to_string(),
            std::format!("decoding error at offset 1: reserved max-APDU-length field value {code}")
        );
    }
    // A reserved code in a segmented request is malformed first, as the
    // owned decoder finds it.
    assert_eq!(
        refused(&[0x0A, 0x06, 0x2A, 0x00, 0x01, 0x0C]).decode_fault(),
        Some((1, DecodingKind::InvalidEncoding))
    );
    // A SimpleACK and a SegmentACK, then nibbles past 7.
    for first in [0x20, 0x40] {
        assert_eq!(
            kind(&[first, 0x2A, 0x0C, 0x00]),
            Some(DecodingKind::Unsupported)
        );
    }
    assert_eq!(
        refused(&[0x80, 0x00]).to_string(),
        "decoding error at offset 0: unknown PDU type nibble: 0x8"
    );
}

#[test]
fn field_codes_name_their_sizes() {
    assert_eq!(
        MaxSegments::from_capacity(None),
        Some(MaxSegments::UNSPECIFIED)
    );
    for (capacity, bits) in [
        (0, None),
        (1, None),
        (2, Some(1)),
        (3, Some(1)),
        (4, Some(2)),
        (15, Some(3)),
        (16, Some(4)),
        (63, Some(5)),
        (64, Some(6)),
        (65, Some(7)),
        (255, Some(7)),
    ] {
        assert_eq!(
            MaxSegments::from_capacity(Some(capacity)).map(MaxSegments::bits),
            bits,
            "{capacity}"
        );
    }
    let counts: [Option<u8>; 8] = [
        None,
        Some(2),
        Some(4),
        Some(8),
        Some(16),
        Some(32),
        Some(64),
        Some(255),
    ];
    for (bits, count) in counts.into_iter().enumerate() {
        assert_eq!(MaxSegments::from_bits(bits as u8).count(), count);
    }
    for (octets, code) in [(50, 0), (128, 1), (206, 2), (480, 3), (1024, 4), (1476, 5)] {
        let field = MaxApdu::from_octets(octets).unwrap();
        assert_eq!((field.bits(), field.octets()), (code, octets));
        assert_eq!(MaxApdu::at_or_below(u32::from(octets)), Some(field));
        assert_eq!(MaxApdu::at_or_below(u32::from(octets) + 1), Some(field));
        assert_eq!(MaxApdu::from_bits(code), Some(field));
    }
    assert_eq!(MaxApdu::from_octets(100), None);
    assert_eq!(MaxApdu::at_or_below(49), None);
    assert_eq!(MaxApdu::at_or_below(127), Some(MaxApdu::OCTETS_50));
    assert_eq!(MaxApdu::at_or_below(u32::MAX), Some(MaxApdu::OCTETS_1476));
}

/// The header writes what the owned encoder writes, and reads what the
/// owned decoder reads, for every unsegmented header it takes.
#[cfg(feature = "alloc")]
#[test]
fn the_header_agrees_with_the_owned_codec() {
    use crate::apdu::{
        decode_apdu, encode_apdu, AbortPdu, Apdu, ComplexAck, ConfirmedRequest, ErrorPdu,
        RejectPdu, UnconfirmedRequest,
    };
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bytes::{Bytes, BytesMut};

    let parameters = Bytes::from_static(&[0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]);
    let mut owned = std::vec::Vec::new();
    for accepted in [false, true] {
        for segments in [None, Some(2), Some(5), Some(64), Some(65)] {
            for size in [50, 128, 206, 480, 1024, 1476] {
                owned.push(Apdu::ConfirmedRequest(ConfirmedRequest {
                    segmented: false,
                    more_follows: false,
                    segmented_response_accepted: accepted,
                    max_segments: segments,
                    max_apdu_length: size,
                    invoke_id: 200,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::from_raw(31),
                    service_request: parameters.clone(),
                }));
            }
        }
    }
    owned.push(Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: parameters.clone(),
    }));
    owned.push(Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: 7,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: READ_PROPERTY,
        service_ack: parameters.clone(),
    }));
    // The class and code of an Error are its parameters; a Reject and an
    // Abort have none.
    let class_and_code: &[u8] = &[0x91, 0x01, 0x91, 0x1F];
    let mut whole = std::vec::Vec::new();
    whole.push(Apdu::Error(ErrorPdu {
        invoke_id: 7,
        service_choice: READ_PROPERTY,
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::UNKNOWN_OBJECT,
        error_data: Bytes::new(),
    }));
    whole.push(Apdu::Reject(RejectPdu {
        invoke_id: 7,
        reject_reason: RejectReason::MISSING_REQUIRED_PARAMETER,
    }));
    for sent_by_server in [false, true] {
        whole.push(Apdu::Abort(AbortPdu {
            sent_by_server,
            invoke_id: 7,
            abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
        }));
    }
    for apdu in owned.into_iter().chain(whole) {
        let mut wire = BytesMut::new();
        encode_apdu(&mut wire, &apdu).unwrap();
        let (header, rest) = ApduHeader::decode(&wire).unwrap();
        let expected: &[u8] = match apdu {
            Apdu::Error(_) => class_and_code,
            Apdu::Reject(_) | Apdu::Abort(_) => &[],
            _ => &parameters[..],
        };
        assert_eq!(rest, expected, "{apdu:?}");
        let mut written = std::vec::Vec::new();
        header
            .encode_with_parameters_to_sink(&mut written, rest)
            .unwrap();
        assert_eq!(written, wire[..], "{apdu:?}");
        assert_eq!(decode_apdu(Bytes::from(written)).unwrap(), {
            // The owned decoder reports the rounded segment count it reads.
            match apdu {
                Apdu::ConfirmedRequest(pdu) => Apdu::ConfirmedRequest(ConfirmedRequest {
                    max_segments: MaxSegments::from_capacity(pdu.max_segments)
                        .unwrap()
                        .count(),
                    ..pdu
                }),
                other => other,
            }
        });
    }
}

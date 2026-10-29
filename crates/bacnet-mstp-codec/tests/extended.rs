//! Extended frames (Types 32 and 33, #1611): the CRC-32K, COBS encoding,
//! the Length rule and its bounds, against the worked examples of Annexes G
//! and T and boundary cases checked by hand.

use bacnet_mstp_codec::crc::{crc8, crc32k};
use bacnet_mstp_codec::{
    Frame, FrameError, FrameType, MAX_COBS_FRAME_LENGTH, MAX_COBS_LENGTH,
    MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH, MIN_COBS_LENGTH,
    PREAMBLE, decode_frame, decode_frame_in_place,
};
use proptest::prelude::*;

mod support;
use support::CRC32K_RESIDUAL;

const EXPECTING: FrameType = FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY;
const NOT_EXPECTING: FrameType = FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY;

fn encode(frame: Frame<'_>) -> Vec<u8> {
    let mut wire = vec![0u8; MAX_COBS_FRAME_LENGTH];
    let used = frame.encode_into(&mut wire).unwrap();
    wire.truncate(used);
    wire
}

/// Each octet XORed with 0x55, as both encoded fields go on the wire.
fn masked(octets: &[u8]) -> Vec<u8> {
    octets.iter().map(|&octet| octet ^ 0x55).collect()
}

/// The header's Length field.
fn length_field(wire: &[u8]) -> usize {
    usize::from(u16::from_be_bytes([wire[5], wire[6]]))
}

/// A frame built by hand from its two encoded fields as they go on the
/// wire, with a correct header CRC.
fn raw_extended(raw_type: u8, destination: u8, source: u8, fields: &[u8]) -> Vec<u8> {
    let [high, low] = u16::try_from(fields.len() - 2).unwrap().to_be_bytes();
    let header = [raw_type, destination, source, high, low];
    let mut wire = PREAMBLE.to_vec();
    wire.extend_from_slice(&header);
    wire.push(crc8(&header));
    wire.extend_from_slice(fields);
    wire
}

/// An Encoded CRC-32K field for these Encoded Data octets, built by hand:
/// the four CRC octets, low first, as one code block of five, masked.
fn crc_field(encoded: &[u8]) -> Vec<u8> {
    let crc = crc32k(encoded).to_le_bytes();
    assert!(
        crc.iter().all(|&octet| octet != 0),
        "the vectors avoid zeros"
    );
    masked(&[0x05, crc[0], crc[1], crc[2], crc[3]])
}

/// Decode `wire` both ways and check it is `expected`: `decode_frame`
/// refuses an extended frame, `decode_frame_in_place` decodes it.
fn decodes_to(wire: &[u8], expected: Frame<'_>) {
    assert_eq!(
        decode_frame(wire),
        Err(FrameError::ExtendedFrame {
            raw_type: expected.header.frame_type.to_raw()
        })
    );
    let mut copy = wire.to_vec();
    assert_eq!(decode_frame_in_place(&mut copy), Ok((expected, wire.len())));
}

/// Annex G.3.1's worked example: the CRC-32K register after each of the
/// data octets 01 22 30, then after each octet of the complement sent low
/// octet first, ending at the residue.
#[test]
fn annex_g_crc32k_example() {
    let data = [0x01, 0x22, 0x30];
    let registers = [0x5638_1747, 0x1255_7D20, 0x83DD_5A41];
    for (count, register) in registers.into_iter().enumerate() {
        assert_eq!(!crc32k(&data[..=count]), register);
    }
    assert_eq!(crc32k(&data), 0x7C22_A5BE);
    let received = [0x01, 0x22, 0x30, 0xBE, 0xA5, 0x22, 0x7C];
    let registers = [0xC0D1_F128, 0x1C70_8944, 0x7FC2_C8BD, CRC32K_RESIDUAL];
    for (count, register) in registers.into_iter().enumerate() {
        assert_eq!(!crc32k(&received[..4 + count]), register);
    }
}

/// Annex T.1's example: the string "Hello World", a newline and a zero.
/// Its COBS encoding is one block of 13 octets, then the block for the zero,
/// and the Length is the 14 encoded octets plus five less two.
///
/// The CRC-32K the annex prints for it covers only the first 13 of the 14
/// encoded octets, which Clause 9.10.2 and the annex's own code and its T.3
/// frame don't do: an erratum in the informative annex. The codec covers
/// all 14, so its Encoded CRC-32K differs from the annex's, and the frame
/// with the annex's field is refused.
#[test]
fn annex_t_hello_world_encoding() {
    let data = b"Hello World\n\0";
    let wire = encode(Frame::new(NOT_EXPECTING, 0xFF, 0x01, data));
    let mut cobs = vec![0x0D];
    cobs.extend_from_slice(&data[..12]);
    cobs.push(0x01);
    let encoded = masked(&cobs);
    assert_eq!(
        encoded,
        [
            0x58, 0x1D, 0x30, 0x39, 0x39, 0x3A, 0x75, 0x02, 0x3A, 0x27, 0x39, 0x31, 0x5F, 0x54
        ]
    );
    assert_eq!(length_field(&wire), 17);
    assert_eq!(wire.len(), 8 + 17 + 2);
    assert_eq!(wire[8..22], encoded[..]);
    // The field over all 14 octets, worked out apart from the codec (a
    // bit-at-a-time CRC gives the register F2 11 25 A2).
    assert_eq!(wire[22..], [0x50, 0x08, 0x8F, 0xBB, 0x58]);
    decodes_to(&wire, Frame::new(NOT_EXPECTING, 0xFF, 0x01, data));

    // The register the annex prints, B3 8F 28 CA, is that of the first 13.
    assert_eq!(!crc32k(&encoded[..13]), 0xB38F_28CA);
    assert_eq!(!crc32k(&encoded), 0xF211_25A2);
    // The frame with the annex's Encoded CRC-32K fails its check.
    let mut printed = wire[..22].to_vec();
    printed.extend_from_slice(&[0x50, 0x60, 0x82, 0x25, 0x19]);
    assert_eq!(
        decode_frame_in_place(&mut printed),
        Err(FrameError::DataCrcMismatch { length: 17 })
    );
}

/// The Who-Has request of Annex T.3, a broadcast Type 33 frame from station
/// 1: an NPDU of 507 octets whose object name is 19 of each letter A-Z.
#[test]
fn annex_t_who_has_frame() {
    let mut npdu = vec![
        0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x07, 0x3D, 0xFE, 0x01, 0xEF, 0x00,
    ];
    let letters: Vec<u8> = (b'A'..=b'Z').flat_map(|letter| [letter; 19]).collect();
    npdu.extend_from_slice(&letters);
    assert_eq!(npdu.len(), 507);
    let frame = Frame::new(NOT_EXPECTING, 0xFF, 0x01, &npdu);
    let wire = encode(frame);

    // The header, with Length 512 and header CRC 4E.
    assert_eq!(wire[..8], [0x55, 0xFF, 0x21, 0xFF, 0x01, 0x02, 0x00, 0x4E]);
    assert_eq!(wire.len(), 522);
    // Four blocks by hand: up to each of the NPDU's two zeros, then the
    // letters as a full block of 254 and a last one of 240.
    let mut cobs = vec![0x05, 0x01, 0x20, 0xFF, 0xFF, 0x08];
    cobs.extend_from_slice(&npdu[5..12]);
    cobs.push(0xFF);
    cobs.extend_from_slice(&letters[..254]);
    cobs.push(0xF1);
    cobs.extend_from_slice(&letters[254..]);
    let encoded = masked(&cobs);
    assert_eq!(wire[8..517], encoded[..]);
    // Spot checks against the annex's dump: the first line of encoded data,
    // the last block's code at 0x114, and the Encoded CRC-32K, whose CRC
    // octets are AC F4 83 BD.
    assert_eq!(
        wire[8..24],
        [
            0x50, 0x54, 0x75, 0xAA, 0xAA, 0x5D, 0xAA, 0x45, 0x52, 0x68, 0xAB, 0x54, 0xBA, 0xAA,
            0x14, 0x14
        ]
    );
    assert_eq!(wire[0x114], 0xA4);
    assert_eq!(wire[517..], [0x50, 0xF9, 0xA1, 0xD6, 0xE8]);
    assert_eq!(crc32k(&encoded).to_le_bytes(), [0xAC, 0xF4, 0x83, 0xBD]);
    decodes_to(&wire, frame);
}

/// Runs of non-zero octets around COBS's 254-octet block limit, checked by
/// hand. A run of 254 fills a block with code FF, which stands for no zero,
/// so when it ends the data nothing follows it (Clause 9.10.1); one octet
/// more starts a second block, and so does a zero after it.
#[test]
fn runs_at_the_254_octet_block_limit() {
    let run = |count: usize| vec![0x11u8; count];
    let cases: [(Vec<u8>, Vec<u8>); 6] = [
        (run(253), [vec![0xFE], run(253)].concat()),
        (run(254), [vec![0xFF], run(254)].concat()),
        (run(255), [vec![0xFF], run(254), vec![0x02, 0x11]].concat()),
        (
            [run(254), vec![0x00]].concat(),
            [vec![0xFF], run(254), vec![0x01, 0x01]].concat(),
        ),
        (
            run(508),
            [vec![0xFF], run(254), vec![0xFF], run(254)].concat(),
        ),
        (
            [vec![0x00], run(254)].concat(),
            [vec![0x01, 0xFF], run(254)].concat(),
        ),
    ];
    for (data, cobs) in cases {
        let frame = Frame::new(EXPECTING, 0x10, 0x05, &data);
        let wire = encode(frame);
        let encoded = masked(&cobs);
        assert_eq!(length_field(&wire), cobs.len() + 3, "{} octets", data.len());
        assert_eq!(wire[8..8 + encoded.len()], encoded[..]);
        assert_eq!(wire[8 + encoded.len()..], crc_field(&encoded)[..]);
        decodes_to(&wire, frame);
        // And the other way: the frame built by hand decodes, and encodes
        // back to the same octets.
        let hand_built = raw_extended(
            0x20,
            0x10,
            0x05,
            &[encoded.clone(), crc_field(&encoded)].concat(),
        );
        assert_eq!(hand_built, wire);
    }
}

/// The smallest and largest Length a COBS header may declare, 5 and 2043
/// (Clause 9.5.3), at both ends of encode and decode, and one past each.
#[test]
fn smallest_and_largest_lengths() {
    let fits = [
        // One data octet: a block of two, so 2 + 5 - 2.
        (vec![0x11], MIN_COBS_LENGTH),
        (vec![0x00], MIN_COBS_LENGTH),
        // Eight full blocks: 8 x 255 encoded octets, plus 3.
        (vec![0x11; 2032], MAX_COBS_LENGTH),
        // Zeros cost nothing extra: 2039 of them make 2040 blocks of one.
        (vec![0x00; 2039], MAX_COBS_LENGTH),
    ];
    for (data, length) in fits {
        let frame = Frame::new(NOT_EXPECTING, 3, 4, &data);
        let wire = encode(frame);
        assert_eq!(length_field(&wire), length);
        assert_eq!(wire.len(), 8 + length + 2);
        decodes_to(&wire, frame);
    }
    assert_eq!(
        encode(Frame::new(NOT_EXPECTING, 3, 4, &[0x11; 2032])).len(),
        MAX_COBS_FRAME_LENGTH
    );

    let refused = [
        (vec![], MIN_COBS_LENGTH - 1),
        (vec![0x11; 2033], MAX_COBS_LENGTH + 2),
        (vec![0x00; 2040], MAX_COBS_LENGTH + 1),
    ];
    for (data, length) in refused {
        let mut output = vec![0xA5u8; MAX_COBS_FRAME_LENGTH + 16];
        assert_eq!(
            Frame::new(EXPECTING, 3, 4, &data).encode_into(&mut output),
            Err(FrameError::InvalidCobsLength { length })
        );
        assert!(output.iter().all(|&octet| octet == 0xA5));
    }

    // Decode: the same bounds on a header built by hand.
    for (length, ok) in [(4u16, false), (5, true), (2043, true), (2044, false)] {
        let mut fields = vec![0u8; usize::from(length) + 2];
        let [high, low] = length.to_be_bytes();
        let header = [0x21, 3, 4, high, low];
        let mut wire = PREAMBLE.to_vec();
        wire.extend_from_slice(&header);
        wire.push(crc8(&header));
        wire.append(&mut fields);
        let result = decode_frame_in_place(&mut wire.clone()).map(|(_, used)| used);
        if ok {
            // A good header, which is all decode_frame looks at; the zeroed
            // body then fails its check.
            assert_eq!(
                decode_frame(&wire),
                Err(FrameError::ExtendedFrame { raw_type: 0x21 })
            );
            assert_eq!(
                result,
                Err(FrameError::DataCrcMismatch {
                    length: usize::from(length)
                })
            );
        } else {
            let error = FrameError::InvalidCobsLength {
                length: usize::from(length),
            };
            assert_eq!(decode_frame(&wire), Err(error));
            assert_eq!(result, Err(error));
        }
    }
}

/// [`MAX_EXTENDED_FRAME_LENGTH`] is the wire length of the largest NPDU
/// with no zeros, the worst case for COBS's overhead.
#[test]
fn the_largest_npdu_fills_the_extended_frame_length() {
    let npdu = [0x11u8; MAX_EXTENDED_MPDU_DATA];
    let wire = encode(Frame::new(NOT_EXPECTING, 3, 4, &npdu));
    assert_eq!(wire.len(), MAX_EXTENDED_FRAME_LENGTH);
    assert_eq!(MAX_EXTENDED_FRAME_LENGTH, 1516);
    assert_eq!(MAX_COBS_FRAME_LENGTH, 2053);
    assert_eq!(MAX_STANDARD_FRAME_LENGTH, 511);
}

/// A slave (128-254) may send a Type 33 reply, and decode takes any source
/// but 255 on either extended type.
#[test]
fn slave_sources_round_trip() {
    for (frame_type, source) in [(NOT_EXPECTING, 200), (NOT_EXPECTING, 254), (EXPECTING, 128)] {
        let frame = Frame::new(frame_type, 1, source, &[0x01, 0x00, 0x30]);
        decodes_to(&encode(frame), frame);
    }
    assert_eq!(
        Frame::new(NOT_EXPECTING, 1, 0xFF, &[1]).encode_into(&mut [0u8; 64]),
        Err(FrameError::BroadcastSource)
    );
}

/// What decode refuses past a good header, and that a refusal leaves the
/// input as it was. `decode_frame` stops at the good header whatever
/// follows it.
#[test]
fn damaged_extended_frames_are_refused() {
    let frame = Frame::new(EXPECTING, 9, 8, b"0123456789");
    let wire = encode(frame);
    let length = length_field(&wire);
    let refuse = |wire: &[u8], error: FrameError| {
        assert_eq!(
            decode_frame(wire),
            Err(FrameError::ExtendedFrame { raw_type: 0x20 })
        );
        let mut copy = wire.to_vec();
        assert_eq!(decode_frame_in_place(&mut copy), Err(error));
        assert_eq!(copy, wire);
    };
    // Truncated anywhere in the encoded fields.
    for end in 8..wire.len() {
        refuse(
            &wire[..end],
            FrameError::Truncated {
                length,
                available: end - 8,
            },
        );
    }
    // Any octet of either field changed.
    for index in 8..wire.len() {
        let mut damaged = wire.clone();
        damaged[index] ^= 0x01;
        refuse(&damaged, FrameError::DataCrcMismatch { length });
    }
    // A zero code octet, or one that runs past the field, even with a
    // CRC-32K that matches: Clause 9.10.3 treats both as a bad CRC.
    for cobs in [vec![0x00, 0x11], vec![0x03, 0x11]] {
        let encoded = masked(&cobs);
        let wire = raw_extended(0x20, 9, 8, &[encoded.clone(), crc_field(&encoded)].concat());
        refuse(&wire, FrameError::DataCrcMismatch { length: 5 });
    }
}

/// The Encoded CRC-32K field's code octets are checked as the data's are
/// (Clause 9.10.3): a zero code, or one that counts past the field's five
/// octets, refuses the frame even when the CRC octets themselves are right.
#[test]
fn the_crc_fields_code_octet_is_checked() {
    let data = [0x11, 0x22];
    let encoded = masked(&[0x03, 0x11, 0x22]);
    let crc = crc32k(&encoded).to_le_bytes();
    let frame = |field: &[u8]| raw_extended(0x21, 1, 2, &[&encoded[..], field].concat());
    decodes_to(
        &frame(&masked(&[0x05, crc[0], crc[1], crc[2], crc[3]])),
        Frame::new(NOT_EXPECTING, 1, 2, &data),
    );
    // A code past the end over the right four octets, which would decode to
    // exactly them if the code went unchecked.
    for code in [0x06, 0x07, 0xFE, 0xFF] {
        let mut wire = frame(&masked(&[code, crc[0], crc[1], crc[2], crc[3]]));
        assert_eq!(
            decode_frame_in_place(&mut wire),
            Err(FrameError::DataCrcMismatch { length: 6 }),
            "code {code:#04X}"
        );
    }

    // A zero code: Encoded Data whose CRC opens with a zero octet, and a
    // field of a zero code then a block of the other three, which would
    // decode to the right four octets if the zero went unchecked.
    let (encoded, crc) = (1..=u8::MAX)
        .flat_map(|first| (1..=u8::MAX).map(move |second| masked(&[0x03, first, second])))
        .map(|encoded| {
            let crc = crc32k(&encoded).to_le_bytes();
            (encoded, crc)
        })
        .find(|(_, crc)| crc[0] == 0)
        .expect("some data has a CRC that opens with a zero");
    let frame = |field: &[u8]| raw_extended(0x21, 1, 2, &[&encoded[..], field].concat());
    let mut strict = frame(&masked(&[0x01, 0x04, crc[1], crc[2], crc[3]]));
    assert!(decode_frame_in_place(&mut strict).is_ok());
    let mut zero = frame(&masked(&[0x00, 0x04, crc[1], crc[2], crc[3]]));
    assert_eq!(
        decode_frame_in_place(&mut zero),
        Err(FrameError::DataCrcMismatch { length: 6 })
    );
}

/// The decoder follows Clause 9.10.3, which accepts two encodings a strict
/// encoder never sends; encoding the result again gives the strict form.
#[test]
fn lenient_encodings_decode_to_the_same_data() {
    // A full block that ends the data, followed by an empty block.
    let mut cobs = vec![0xFF];
    cobs.extend_from_slice(&[0x22; 254]);
    cobs.push(0x01);
    // A data octet that decodes to zero inside a block.
    let inner_zero = vec![0x04, 0x22, 0x00, 0x33];
    for (cobs, data) in [
        (cobs, vec![0x22; 254]),
        (inner_zero, vec![0x22, 0x00, 0x33]),
    ] {
        let encoded = masked(&cobs);
        let wire = raw_extended(0x21, 1, 2, &[encoded.clone(), crc_field(&encoded)].concat());
        let frame = Frame::new(NOT_EXPECTING, 1, 2, &data);
        decodes_to(&wire, frame);
        let strict = encode(frame);
        assert_ne!(strict, wire);
        decodes_to(&strict, frame);
    }
}

/// The CRC-32K a bit at a time, from the generator's terms (Clause 9.6.3):
/// each term x^n sets bit 31 - n of the register that shifts right.
fn bitwise_crc32k(data: &[u8]) -> u32 {
    let terms = [0, 1, 2, 4, 6, 7, 10, 11, 15, 16, 17, 19, 20, 26, 28, 29, 30];
    let generator = terms.iter().fold(0u32, |acc, term| acc | 1 << (31 - term));
    let mut register = u32::MAX;
    for &octet in data {
        for bit in 0..8 {
            let feedback = (register ^ u32::from(octet >> bit)) & 1;
            register >>= 1;
            if feedback != 0 {
                register ^= generator;
            }
        }
    }
    !register
}

proptest! {
    /// The codec's CRC-32K, from whichever table this build has, matches
    /// the bit-at-a-time one.
    #[test]
    fn crc32k_matches_the_bitwise_definition(data in prop::collection::vec(any::<u8>(), 0..600)) {
        prop_assert_eq!(crc32k(&data), bitwise_crc32k(&data));
    }

    /// Any data within the bounds round-trips through encode and both
    /// decodes, and the Length follows the rule of Clause 9.3.
    #[test]
    fn extended_frames_round_trip(
        data in prop::collection::vec(prop_oneof![Just(0u8), any::<u8>()], 1..=MAX_EXTENDED_MPDU_DATA),
        expecting in any::<bool>(),
        destination in any::<u8>(),
        source in 0u8..=254,
    ) {
        let frame_type = if expecting { EXPECTING } else { NOT_EXPECTING };
        let frame = Frame::new(frame_type, destination, source, &data);
        let wire = encode(frame);
        prop_assert!(wire.len() <= MAX_EXTENDED_FRAME_LENGTH);
        prop_assert_eq!(wire.len(), 8 + length_field(&wire) + 2);
        // No preamble octet anywhere past the header.
        prop_assert!(!wire[8..].contains(&0x55));
        let mut copy = wire.clone();
        prop_assert_eq!(decode_frame_in_place(&mut copy), Ok((frame, wire.len())));
        for end in 0..wire.len() {
            prop_assert!(decode_frame_in_place(&mut wire[..end].to_vec()).is_err());
        }
    }
}

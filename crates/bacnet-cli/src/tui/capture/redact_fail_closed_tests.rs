//! The masking fails closed (#2062) and covers segmented requests (#2084):
//! damaged link headers, MS/TP frames that don't decode or can't be
//! encoded again, tags the strict decoder refuses, and long multi-octet
//! passwords split across segments.

use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, encode_apdu};
use bacnet_encoding::segmentation::{SegmentedPduType, max_segment_payload, split_payload};
use bacnet_transport::sc_frame::{ScFunction, ScMessage, encode_sc_message};
use bytes::BytesMut;

use super::tests::{ctx_string, holds, masked_in_place, npdu, reinit_npdu, request};
use super::*;
use bacnet_ops::decode::testing;

const DCC: ConfirmedServiceChoice = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
const REINIT: ConfirmedServiceChoice = ConfirmedServiceChoice::REINITIALIZE_DEVICE;

/// Twenty characters of four UTF-8 octets each, all different: 80 octets,
/// within the character limit (#2060).
fn long_password() -> String {
    (0x1F600..0x1F614).filter_map(char::from_u32).collect()
}

/// Whether `frame` holds any four consecutive octets of `secret`.
fn holds_any(frame: &[u8], secret: &[u8]) -> bool {
    secret.windows(4).any(|part| holds(frame, part))
}

/// A DCC request's parameters, with `password` as character set 0.
fn dcc_body(password: &[u8]) -> Vec<u8> {
    [&[0x09, 0x1E, 0x19, 0x01][..], &ctx_string(2, 0, password)].concat()
}

/// A ReinitializeDevice request's parameters, warm start.
fn reinit_body(password: &[u8]) -> Vec<u8> {
    [&[0x09, 0x01][..], &ctx_string(1, 0, password)].concat()
}

/// `body` as the client sends it to a peer taking at most `max_apdu`
/// octets: each segment's APDU.
fn segments(service: ConfirmedServiceChoice, body: &[u8], max_apdu: u16) -> Vec<Vec<u8>> {
    let size = max_segment_payload(max_apdu, SegmentedPduType::ConfirmedRequest);
    segments_of(service, body, size)
}

/// `body` split into segments of `size` octets of service data.
fn segments_of(service: ConfirmedServiceChoice, body: &[u8], size: usize) -> Vec<Vec<u8>> {
    let parts = split_payload(body, size).unwrap();
    let count = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            let mut apdu = BytesMut::new();
            let pdu = ConfirmedRequest {
                segmented: true,
                more_follows: index + 1 < count,
                segmented_response_accepted: true,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: 9,
                sequence_number: Some(index as u8),
                proposed_window_size: Some(4),
                service_choice: service,
                service_request: part,
            };
            encode_apdu(&mut apdu, &Apdu::ConfirmedRequest(pdu)).unwrap();
            apdu.to_vec()
        })
        .collect()
}

/// Mask each segment as a B/IP frame and return the service data they
/// carry after masking, joined, checking each frame on the way.
fn masked_segments(apdus: &[Vec<u8>], secret: &[u8]) -> Vec<u8> {
    let mut joined = Vec::new();
    for apdu in apdus {
        let frame = testing::bip(&npdu(apdu), false);
        let masked = redacted(Framing::Bip, &frame).expect("every segment holds some password");
        assert_eq!(masked.len(), frame.len());
        assert!(!holds_any(&masked, secret), "{masked:?}");
        // Four octets of BVLL, two of NPDU, six of segment header.
        assert_eq!(masked[..12], frame[..12]);
        joined.extend_from_slice(&masked[12..]);
    }
    joined
}

#[test]
fn a_segmented_dcc_straddling_two_segments_is_masked() {
    let password = long_password();
    let body = dcc_body(password.as_bytes());
    assert_eq!(body.len(), 87);
    let apdus = segments(DCC, &body, 50);
    assert_eq!(apdus.len(), 2, "the password straddles both");
    let joined = masked_segments(&apdus, password.as_bytes());
    // The fields before the password, its tag and character set are kept;
    // all 80 octets of text are `*`.
    assert_eq!(joined[..7], body[..7]);
    assert_eq!(joined[7..], [b'*'; 80]);
}

#[test]
fn a_segmented_reinitialize_straddling_two_segments_is_masked() {
    let password = long_password();
    let body = reinit_body(password.as_bytes());
    let apdus = segments(REINIT, &body, 50);
    assert_eq!(apdus.len(), 2);
    let joined = masked_segments(&apdus, password.as_bytes());
    assert_eq!(joined[..5], body[..5]);
    assert_eq!(joined[5..], [b'*'; 80]);
}

/// A first segment that holds the whole password, and a later segment
/// that holds only the end of it, on MS/TP too.
#[test]
fn a_password_in_the_first_segment_only_is_masked() {
    let password = long_password();
    for (service, body, kept) in [
        (DCC, dcc_body(password.as_bytes()), 7),
        (REINIT, reinit_body(password.as_bytes()), 5),
    ] {
        let apdus = segments(service, &body, 128);
        assert_eq!(apdus.len(), 1);
        let joined = masked_segments(&apdus, password.as_bytes());
        assert_eq!(joined[..kept], body[..kept]);
        assert_eq!(joined[kept..], [b'*'; 80]);
    }
    // A last segment carrying only the password's final octets.
    let apdus = segments(DCC, &dcc_body(password.as_bytes()), 50);
    let wire = testing::mstp(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        5,
        1,
        &npdu(&apdus[1]),
    );
    let masked = redacted(Framing::Mstp, &wire).expect("masked");
    assert!(!holds_any(&masked, &password.as_bytes()[40..]));
}

/// Encoding a valid MS/TP frame again failing (it can't today) leaves the
/// masked data in place rather than the original frame.
#[test]
fn an_mstp_frame_that_cant_be_encoded_again_is_masked_in_place() {
    let secret = b"Zq7-hunter2-Xv9";
    let npdu = reinit_npdu("Zq7-hunter2-Xv9");
    let refuse: Encoder = |_| None;
    let wire = testing::mstp(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &npdu);
    let masked = mstp(&wire, refuse).expect("masked");
    assert_eq!(masked.len(), wire.len());
    assert!(!holds(&masked, secret));
    assert!(holds(&masked, &[b'*'; 15]));
    // An extended frame: the octets written back are masked too.
    let wire = testing::mstp(FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY, 5, 1, &npdu);
    let masked = mstp(&wire, refuse).expect("masked");
    assert_eq!(masked.len(), wire.len());
    let data = decoded_extended(&masked);
    assert!(!holds(&data, secret), "{data:?}");
    assert!(holds(&data, &[b'*'; 15]));
}

/// An extended frame's data, decoded without the checks.
fn decoded_extended(frame: &[u8]) -> Vec<u8> {
    let field = 8..frame.len() - 5;
    cobs_decode(&frame[field])
        .into_iter()
        .map(|(octet, _)| octet)
        .collect()
}

/// An extended frame with a bad CRC-32K is decoded without it and masked
/// in place, its CRC and length kept; so is one whose header CRC is bad.
#[test]
fn an_extended_mstp_frame_that_doesnt_decode_is_masked_in_place() {
    let secret = b"Zq7-hunter2-Xv9";
    let npdu = reinit_npdu("Zq7-hunter2-Xv9");
    let mut wire = testing::mstp(
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
        5,
        1,
        &npdu,
    );
    assert!(holds(&decoded_extended(&wire), secret));
    let last = wire.len() - 1;
    wire[last] ^= 0x01;
    for damaged_header in [false, true] {
        if damaged_header {
            wire[7] ^= 0xFF;
        }
        let masked = redacted(Framing::Mstp, &wire).expect("masked");
        assert_eq!(masked.len(), wire.len());
        assert_eq!(masked[last - 4..], wire[last - 4..], "the CRC is kept");
        let data = decoded_extended(&masked);
        assert!(!holds(&data, secret), "{data:?}");
        assert!(holds(&data, &[b'*'; 15]));
    }
}

/// A standard frame whose type octet was damaged into a COBS type, so its
/// header CRC fails: its plain data is still found.
#[test]
fn a_standard_frame_with_a_damaged_type_is_still_masked() {
    let mut wire = testing::mstp(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        5,
        1,
        &reinit_npdu("Zq7-hunter2-Xv9"),
    );
    wire[2] = FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY.to_raw();
    masked_in_place(Framing::Mstp, &wire, b"Zq7-hunter2-Xv9");
}

/// Tags the strict decoder refuses: a short length written in the
/// extended form, a length written wider than needed, and one over the
/// decoder's limit.
#[test]
fn non_canonical_and_oversized_lengths_are_masked() {
    let forms: [(&[u8], &[u8]); 3] = [
        (&[0x2D, 4], b"Zq7"),
        (&[0x2D, 0xFE, 0x00, 12], b"Zq7-hunter2"),
        (&[0x2D, 0xFF, 0x7F, 0, 0, 0], b"Zq7-hunter2"),
    ];
    for (form, secret) in forms {
        let body = [&[0x19, 0x01][..], form, &[0x00], secret].concat();
        assert!(bacnet_encoding::tags::decode_tag(&body, 2).is_err());
        let frame = testing::bip(&npdu(&request(DCC, &body)), false);
        masked_in_place(Framing::Bip, &frame, secret);
    }
    let secret = b"Zq7-hunter2";
    // A field before the password claiming more than it may holds the
    // password: masked to the end. So is a password under another tag.
    for body in [
        [&[0x0D, 0x40][..], &ctx_string(2, 0, secret)].concat(),
        [&[0x19, 0x01][..], &ctx_string(3, 0, secret)].concat(),
    ] {
        let frame = testing::bip(&npdu(&request(DCC, &body)), false);
        masked_in_place(Framing::Bip, &frame, secret);
    }
}

/// B/IP and B/IPv6 frames whose Length disagrees with the datagram, and a
/// BACnet/SC frame with reserved control bits set, don't decode; the
/// request inside is found and masked.
#[test]
fn frames_whose_link_header_doesnt_decode_are_searched() {
    let secret = b"Zq7-hunter2";
    let npdu = npdu(&request(DCC, &dcc_body(secret)));
    let mut bip = testing::bip(&npdu, false);
    bip[3] = bip[3].wrapping_add(7);
    masked_in_place(Framing::Bip, &bip, secret);
    let mut bip6 = testing::bip6_broadcast(&npdu);
    bip6[3] = bip6[3].wrapping_sub(3);
    masked_in_place(Framing::Bip6, &bip6, secret);
    let mut sc = BytesMut::new();
    encode_sc_message(
        &mut sc,
        &ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 7,
            originating_vmac: Some([2, 0, 0, 0, 0, 1]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(npdu.clone()),
        },
    );
    let mut sc = sc.to_vec();
    sc[1] |= 0xF0;
    masked_in_place(Framing::Sc, &sc, secret);
    // A damaged frame without either request is still left alone.
    let read = request(ConfirmedServiceChoice::READ_PROPERTY, &[0x0C, 0, 0, 0, 1]);
    let mut damaged = testing::bip(&super::tests::npdu(&read), false);
    damaged[3] ^= 0x40;
    assert_eq!(redacted(Framing::Bip, &damaged), None);
}

/// A segment boundary inside the password tag's header: the first
/// segment ends on its first octet, the length octet starts the next.
#[test]
fn a_password_tag_header_split_across_segments_is_masked() {
    let password = long_password();
    let body = dcc_body(password.as_bytes());
    assert_eq!(body[4], 0x2D);
    let apdus = segments_of(DCC, &body, 5);
    let joined = masked_segments(&apdus, password.as_bytes());
    assert_eq!(joined[..4], body[..4]);
    assert!(joined[4..].iter().all(|&octet| octet == b'*'));
}

/// A time duration of four octets is still a field to walk past; one of
/// five isn't, and is masked to the end.
#[test]
fn the_time_duration_boundary() {
    let secret = b"Zq7-hunter2";
    let four = [
        &[0x0C, 0, 0, 0x0E, 0x10, 0x19, 0x01][..],
        &ctx_string(2, 0, secret),
    ]
    .concat();
    let frame = testing::bip(&npdu(&request(DCC, &four)), false);
    let masked = masked_in_place(Framing::Bip, &frame, secret);
    let start = frame.len() - four.len();
    assert_eq!(
        masked[..start + 9],
        frame[..start + 9],
        "all before the text kept"
    );
    let five = [
        &[0x0D, 5, 0, 0, 0, 0x0E, 0x10, 0x19, 0x01][..],
        &ctx_string(2, 0, secret),
    ]
    .concat();
    let frame = testing::bip(&npdu(&request(DCC, &five)), false);
    let masked = masked_in_place(Framing::Bip, &frame, secret);
    let start = frame.len() - five.len();
    assert!(masked[start + 2..].iter().all(|&octet| octet == b'*'));
}

/// Damaged frames of random octets, and damaged frames of other traffic,
/// are left alone: a search match needs a plausible request with its
/// password where it belongs.
#[test]
fn damaged_frames_without_a_request_are_left_alone() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut masked = 0;
    for _ in 0..300 {
        let length = 200 + (next() % 1200) as usize;
        let octets: Vec<u8> = (0..length).map(|_| next() as u8).collect();
        for framing in [Framing::Bip, Framing::Bip6, Framing::Sc, Framing::Mstp] {
            masked += usize::from(redacted(framing, &octets).is_some());
        }
    }
    assert_eq!(masked, 0);
    // A later segment of a segmented WriteProperty, in a frame whose
    // Length is wrong.
    let write = segments_of(ConfirmedServiceChoice::WRITE_PROPERTY, &[0x2D; 90], 44);
    let mut frame = testing::bip(&npdu(&write[1]), false);
    frame[3] ^= 0x10;
    assert_eq!(redacted(Framing::Bip, &frame), None);
}

/// A located request without its required field still has its password
/// masked; a searched one isn't taken for a request. A field repeated
/// ends the walk, so the rest is masked.
#[test]
fn required_and_repeated_fields() {
    let secret = b"Zq7-hunter2";
    let no_enable = [&[0x09, 0x1E][..], &ctx_string(2, 0, secret)].concat();
    let sound = testing::bip(&npdu(&request(DCC, &no_enable)), false);
    masked_in_place(Framing::Bip, &sound, secret);
    let mut damaged = sound.clone();
    damaged[3] ^= 0x10;
    assert_eq!(redacted(Framing::Bip, &damaged), None);
    let twice = [
        &[0x09, 0x1E, 0x09, 0x1E, 0x19, 0x01][..],
        &ctx_string(2, 0, secret),
    ]
    .concat();
    let frame = testing::bip(&npdu(&request(DCC, &twice)), false);
    let masked = masked_in_place(Framing::Bip, &frame, secret);
    let start = frame.len() - twice.len();
    assert!(masked[start + 3..].iter().all(|&octet| octet == b'*'));
}

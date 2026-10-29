//! The capture's password masking (#1799) on every framing, on passwords
//! the service decoders refuse, on malformed input, and on random bytes.

use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_transport::bip6::{Bip6Address, Bip6Vmac, Bvll6, encode_bvll6};
use bacnet_transport::sc_frame::{ScFunction, ScMessage, ScOption, encode_sc_message};
use bacnet_types::enums::{EnableDisable, ReinitializedState};
use bytes::BytesMut;

use super::*;
use crate::tui::capture::Direction;
use bacnet_ops::decode::testing;

const DCC: ConfirmedServiceChoice = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
const REINIT: ConfirmedServiceChoice = ConfirmedServiceChoice::REINITIALIZE_DEVICE;

/// An unsegmented confirmed request of `service` with `body`.
pub(super) fn request(service: ConfirmedServiceChoice, body: &[u8]) -> Vec<u8> {
    [&[0x00, 0x05, 0x01, service.to_raw()][..], body].concat()
}

/// A local NPDU expecting a reply, around `apdu`.
pub(super) fn npdu(apdu: &[u8]) -> Vec<u8> {
    [&[0x01, 0x04][..], apdu].concat()
}

/// A DCC request as the client encodes it.
fn dcc(password: Option<&str>) -> Vec<u8> {
    let mut body = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: Some(30),
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: password.map(str::to_string),
    }
    .encode(&mut body)
    .unwrap();
    request(DCC, &body)
}

/// A context-tagged character string, any character set and octets.
pub(super) fn ctx_string(tag: u8, charset: u8, text: &[u8]) -> Vec<u8> {
    let length = text.len() + 1;
    let mut out = if length <= 4 {
        vec![(tag << 4) | 0x08 | length as u8]
    } else {
        vec![(tag << 4) | 0x0D, length as u8]
    };
    out.push(charset);
    out.extend_from_slice(text);
    out
}

/// Whether `frame` still holds `secret` anywhere.
pub(super) fn holds(frame: &[u8], secret: &[u8]) -> bool {
    frame.windows(secret.len()).any(|w| w == secret)
}

/// The masked frame: same length, `secret` gone, and that many `*`.
pub(super) fn masked_in_place(framing: Framing, frame: &[u8], secret: &[u8]) -> Bytes {
    let masked = redacted(framing, frame).expect("masked");
    assert_eq!(masked.len(), frame.len());
    assert!(!holds(&masked, secret), "{masked:?}");
    assert!(holds(&masked, &vec![b'*'; secret.len()]), "{masked:?}");
    masked
}

#[test]
fn a_dcc_password_is_masked_in_place() {
    let frame = testing::bip(&npdu(&dcc(Some("Zq7-hunter2"))), false);
    let masked = masked_in_place(Framing::Bip, &frame, b"Zq7-hunter2");
    let sent = DeviceCommunicationControlRequest::decode(&masked[10..]).unwrap();
    assert_eq!(sent.password.as_deref(), Some("***********"));
    assert_eq!(sent.time_duration, Some(30));
    // Nothing to mask without a password, or in another service.
    assert_eq!(
        redacted(Framing::Bip, &testing::bip(&npdu(&dcc(None)), false)),
        None
    );
    let read = request(ConfirmedServiceChoice::READ_PROPERTY, &[0x0C, 0, 0, 0, 1]);
    assert_eq!(
        redacted(Framing::Bip, &testing::bip(&npdu(&read), false)),
        None
    );
}

/// A non-ASCII password over the 20-character limit: the service decoder
/// refuses it (#2060 counts characters, not octets), and it is masked anyway.
#[test]
fn a_non_ascii_password_the_decoder_refuses_is_masked() {
    let password = "pässwörd-ñandú-éé-ñäö";
    assert_eq!(password.chars().count(), 21);
    // The encoder refuses it (#2085), so build the request by hand: a
    // 30-minute disable-initiation, then the UTF-8 (set 0) password.
    let body = [
        &[0x09, 30, 0x19, 0x02][..],
        &ctx_string(2, 0, password.as_bytes()),
    ]
    .concat();
    let apdu = request(DCC, &body);
    assert!(DeviceCommunicationControlRequest::decode(&apdu[4..]).is_err());
    let frame = testing::bip(&npdu(&apdu), false);
    masked_in_place(Framing::Bip, &frame, password.as_bytes());
}

/// Other character sets, trailing octets and a cut-short string are
/// masked as far as the frame holds the password.
#[test]
fn odd_charsets_trailing_octets_and_truncation_are_masked() {
    let reinit_state = [0x09, 0x00];
    // UCS-2 (character set 4), with octets the decoder won't take after.
    let ucs2 = [
        0x00, b'h', 0x00, b'u', 0x00, b'n', 0x00, b't', 0x00, b'e', 0x00, b'r',
    ];
    let body = [&reinit_state[..], &ctx_string(1, 4, &ucs2), &[0xAA, 0xBB]].concat();
    let frame = testing::bip(&npdu(&request(REINIT, &body)), false);
    let masked = redacted(Framing::Bip, &frame).expect("masked");
    assert!(!holds(&masked, b"\0h\0u\0n"), "{masked:?}");
    // What follows the password is masked too: nothing after it is trusted.
    assert_eq!(&masked[masked.len() - 2..], b"**", "the tail too");
    // Invalid UTF-8 under character set 0.
    let body = [&reinit_state[..], &ctx_string(1, 0, b"\xFF\xFEsecret-xyz")].concat();
    let frame = testing::bip(&npdu(&request(REINIT, &body)), false);
    masked_in_place(Framing::Bip, &frame, b"secret-xyz");
    // The tag claims 16 octets; the frame stops after 6 of them.
    let mut apdu = request(
        DCC,
        &[&[0x19, 0x02][..], &ctx_string(2, 0, b"hunter2-longer!")].concat(),
    );
    apdu.truncate(apdu.len() - 9);
    let frame = testing::bip(&npdu(&apdu), false);
    let masked = redacted(Framing::Bip, &frame).expect("masked");
    assert!(!holds(&masked, b"hunt"), "{masked:?}");
    assert!(masked.ends_with(b"******"), "{masked:?}");
}

#[test]
fn bip6_frames_are_masked_unicast_and_forwarded() {
    let npdu = npdu(&dcc(Some("Zq7-hunter2")));
    let messages = [
        Bvll6::OriginalUnicastNpdu {
            source: Bip6Vmac::new([0, 1, 2]),
            destination: Bip6Vmac::new([0, 1, 3]),
            npdu: &npdu,
        },
        Bvll6::ForwardedNpdu {
            original_source: Bip6Vmac::new([0, 1, 2]),
            original_address: Bip6Address::new(std::net::Ipv6Addr::LOCALHOST, 47808),
            npdu: &npdu,
        },
    ];
    for message in messages {
        let mut frame = BytesMut::new();
        encode_bvll6(&mut frame, &message).unwrap();
        masked_in_place(Framing::Bip6, &frame, b"Zq7-hunter2");
    }
}

#[test]
fn an_sc_frame_with_header_options_is_masked() {
    let message = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 7,
        originating_vmac: Some([2, 0, 0, 0, 0, 1]),
        destination_vmac: Some([2, 0, 0, 0, 0, 2]),
        dest_options: vec![ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![9, 9, 9],
        }],
        data_options: vec![ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![8],
        }],
        payload: Bytes::from(npdu(&dcc(Some("Zq7-hunter2")))),
    };
    let mut frame = BytesMut::new();
    encode_sc_message(&mut frame, &message);
    masked_in_place(Framing::Sc, &frame, b"Zq7-hunter2");
}

pub(super) fn reinit_npdu(password: &str) -> Vec<u8> {
    let mut body = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::COLDSTART,
        password: Some(password.into()),
    }
    .encode(&mut body)
    .unwrap();
    npdu(&request(REINIT, &body))
}

#[test]
fn mstp_frames_keep_a_valid_crc_plain_and_extended() {
    let npdu = reinit_npdu("Zq7-hunter2-Xv9");
    for frame_type in [
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
    ] {
        let wire = testing::mstp(frame_type, 5, 1, &npdu);
        let masked = redacted(Framing::Mstp, &wire).expect("masked");
        assert!(!holds(&masked, b"hunter2"), "{frame_type:?}");
        let mut copy = masked.to_vec();
        let (decoded, _) = bacnet_transport::mstp::codec::decode_frame_in_place(&mut copy).unwrap();
        let sent = ReinitializeDeviceRequest::decode(&decoded.data[6..]).unwrap();
        assert_eq!(sent.password.as_deref(), Some("***************"));
    }
}

/// A frame whose data CRC is wrong doesn't decode; its header is sound, so
/// the data is masked in place and the CRC left as it was.
#[test]
fn an_mstp_frame_with_a_bad_data_crc_is_masked_in_place() {
    let mut wire = testing::mstp(
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        5,
        1,
        &reinit_npdu("Zq7-hunter2-Xv9"),
    );
    let last = wire.len() - 1;
    wire[last] ^= 0xFF;
    let masked = masked_in_place(Framing::Mstp, &wire, b"Zq7-hunter2-Xv9");
    assert_eq!(masked[last], wire[last]);
    // With a bad header CRC too, the frame is searched as it stands (#2062).
    wire[7] ^= 0xFF;
    masked_in_place(Framing::Mstp, &wire, b"Zq7-hunter2-Xv9");
}

#[test]
fn malformed_and_out_of_scope_frames_are_left_alone() {
    let bip = |apdu: &[u8]| testing::bip(&npdu(apdu), false);
    // A short APDU.
    assert_eq!(redacted(Framing::Bip, &bip(&[0x00, 0x05, 0x01])), None);
    // A network-layer message whose octets look like a request.
    let mut network = vec![0x01, 0x80, 0x00];
    network.extend_from_slice(&dcc(Some("Zq7-hunter2")));
    assert_eq!(redacted(Framing::Bip, &testing::bip(&network, false)), None);
    // Length octets claiming far more than the frame holds, as a 2-octet
    // extended length: masked up to the frame's end, no panic.
    let body = [
        &[0x19, 0x02, 0x2D, 0xFE, 0x01, 0x00, 0x00][..],
        b"Zq7-hunter2",
    ]
    .concat();
    let masked = redacted(Framing::Bip, &bip(&request(DCC, &body))).expect("masked");
    assert!(!holds(&masked, b"hunter2"));
    // An unexpected tag (here with an extended tag number) before the
    // password: masked from there to the end (#2062).
    let body = [&[0xF9, 0x20, 0x00][..], &ctx_string(2, 0, b"Zq7-hunter2")].concat();
    let frame = bip(&request(DCC, &body));
    masked_in_place(Framing::Bip, &frame, b"Zq7-hunter2");
}

/// The masking never panics, whatever the octets, on any framing: random
/// strings, and random damage to a real request.
#[test]
fn arbitrary_octets_never_panic() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let framings = [Framing::Bip, Framing::Bip6, Framing::Sc, Framing::Mstp];
    let base = testing::bip(&npdu(&dcc(Some("Zq7-hunter2"))), false);
    for round in 0..4_000 {
        let length = (next() % 96) as usize;
        let mut octets: Vec<u8> = (0..length).map(|_| next() as u8).collect();
        if round % 2 == 0 {
            // A real request with a few octets changed and maybe cut.
            octets = base.clone();
            for _ in 0..3 {
                let at = (next() as usize) % octets.len();
                octets[at] = next() as u8;
            }
            octets.truncate((next() as usize) % (octets.len() + 1));
        }
        for framing in framings {
            let _ = redacted(framing, &octets);
        }
    }
}

/// Every frame is scrubbed, whichever way it went: a request addressed to
/// this node keeps no password in the ring either.
#[test]
fn frames_are_scrubbed_whichever_way_they_went() {
    let frame = |direction| CaptureFrame {
        time: std::time::UNIX_EPOCH,
        framing: Framing::Bip,
        direction,
        udp: None,
        source: String::new(),
        destination: String::new(),
        via: None,
        bytes: Bytes::from(testing::bip(&npdu(&dcc(Some("Zq7-hunter2"))), false)),
        summary: Default::default(),
        redacted: false,
    };
    for direction in [Direction::In, Direction::Out, Direction::Wire] {
        let scrubbed = scrub(frame(direction));
        assert!(scrubbed.redacted);
        assert!(!holds(&scrubbed.bytes, b"hunter2"));
    }
}

/// Every label of a decode tree, depth first.
fn tree_labels(fields: &[bacnet_ops::decode::tree::Field], out: &mut Vec<String>) {
    for field in fields {
        out.push(field.label.clone());
        tree_labels(&field.children, out);
    }
}

/// A masked DCC or ReinitializeDevice frame decodes with its fields named
/// (#2074), and the tree shows the password as hidden: neither the secret
/// nor its mask, plain or as hex.
#[test]
fn the_decode_tree_of_a_masked_frame_names_fields_and_hides_the_password() {
    let cases = [
        (
            testing::bip(&npdu(&dcc(Some("Zq7-hunter2"))), false),
            "Enable/disable: disable-initiation",
        ),
        (
            testing::bip(&reinit_npdu("Zq7-hunter2"), false),
            "Reinitialized state: coldstart",
        ),
    ];
    for (frame, named) in cases {
        let masked = masked_in_place(Framing::Bip, &frame, b"Zq7-hunter2");
        let mut labels = Vec::new();
        tree_labels(
            &bacnet_ops::decode::tree::decode_tree(Framing::Bip, &masked),
            &mut labels,
        );
        assert!(labels.iter().any(|l| l == named), "{labels:?}");
        assert!(
            labels.iter().any(|l| l == "Password: (hidden)"),
            "{labels:?}"
        );
        let all = labels.join("\n");
        for leaked in ["hunter2", "***", "2A2A2A", "68756E74"] {
            assert!(!all.contains(leaked), "{leaked} in {all}");
        }
    }
}

/// A password tag whose length claims too little leaves the rest of the
/// password to read as more tags: it is masked from the password on to the
/// end of the APDU, so neither the ring nor the decode tree holds it.
#[test]
fn a_password_that_claims_too_little_is_masked_to_the_end() {
    // DCC: [1] disable, a [2] of 2 octets ("h"), then "unte" as a [0].
    let dcc = [0x19, 0x00, 0x2A, 0x00, 0x68, 0x0C, 0x75, 0x6E, 0x74, 0x65];
    // Reinitialize: [0] coldstart, a [1] of 2 octets, then "unt" as a [0].
    let reinit = [0x09, 0x00, 0x1A, 0x00, 0x68, 0x0B, 0x75, 0x6E, 0x74];
    for apdu in [request(DCC, &dcc), request(REINIT, &reinit)] {
        let frame = testing::bip(&npdu(&apdu), false);
        let masked = redacted(Framing::Bip, &frame).expect("masked");
        assert_eq!(masked.len(), frame.len());
        for secret in [&b"unt"[..], b"h", &[0x75, 0x6E]] {
            assert!(!holds(&masked[frame.len() - apdu.len() + 4..], secret));
        }
        let mut labels = Vec::new();
        tree_labels(
            &bacnet_ops::decode::tree::decode_tree(Framing::Bip, &masked),
            &mut labels,
        );
        let all = labels.join("\n");
        for leaked in ["unt", "756E74", "68756E74", "2A2A"] {
            assert!(!all.contains(leaked), "{leaked} in {all}");
        }
    }
}

/// Masked segments of a ReinitializeDevice request decode with the
/// password hidden: the first segment's tag walk names it hidden, and a
/// later segment is one opaque leaf. Neither the secret nor its mask shows.
#[test]
fn the_decode_tree_of_masked_segments_hides_the_password() {
    let mut body = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::WARMSTART,
        password: Some("Zq7-hunter2".into()),
    }
    .encode(&mut body)
    .unwrap();
    // First segment (sequence 0, window 4), then a later one (sequence 1)
    // carrying more of the password.
    let first = [&[0x0E, 0x05, 0x01, 0x00, 0x04, REINIT.to_raw()][..], &body].concat();
    let later = [
        &[0x0E, 0x05, 0x01, 0x01, 0x04, REINIT.to_raw()][..],
        b"hunter2-Xv9",
    ]
    .concat();
    for (apdu, expected) in [
        (first, "Password: (hidden)"),
        (
            later,
            "Segment data: 11 octets, hidden (may hold a password)",
        ),
    ] {
        let frame = testing::bip(&npdu(&apdu), false);
        let masked = redacted(Framing::Bip, &frame).expect("masked");
        assert!(!holds(&masked, b"hunter2"));
        let mut labels = Vec::new();
        tree_labels(
            &bacnet_ops::decode::tree::decode_tree(Framing::Bip, &masked),
            &mut labels,
        );
        assert!(labels.iter().any(|l| l == expected), "{labels:?}");
        let all = labels.join("\n");
        for leaked in ["hunter2", "***", "2A2A2A", "68756E74"] {
            assert!(!all.contains(leaked), "{leaked} in {all}");
        }
    }
}

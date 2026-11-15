use bacnet_transport::mstp::codec::FrameType;
use bacnet_types::enums::ConfirmedServiceChoice;

use super::{Framing, summarize_frame};
use crate::decode::testing::*;

#[test]
fn a_bip_request_shows_its_function_pdu_invoke_id_and_service() {
    let frame = bip(&npdu(&rp_request(7), true), false);
    let summary = summarize_frame(Framing::Bip, &frame);
    assert_eq!(summary.link_message, "original-unicast-npdu");
    assert_eq!(summary.pdu.as_deref(), Some("Confirmed-Request"));
    assert_eq!(summary.invoke_id, Some(7));
    assert_eq!(summary.service.as_deref(), Some("read-property"));
    assert_eq!(summary.problem, None);
    assert_eq!(summary.parse_error, None);
    assert!(!summary.broadcast);
}

#[test]
fn errors_naks_and_bad_frames_carry_a_problem() {
    let frame = bip(
        &npdu(&error(9, ConfirmedServiceChoice::READ_PROPERTY), false),
        false,
    );
    let summary = summarize_frame(Framing::Bip, &frame);
    assert_eq!(summary.pdu.as_deref(), Some("Error"));
    assert_eq!(summary.service.as_deref(), Some("read-property"));
    assert_eq!(summary.parse_error, None);
    assert_eq!(
        summary.problem.as_deref(),
        Some("property:unknown-property")
    );

    let summary = summarize_frame(Framing::Bip, &bip_nak());
    assert_eq!(summary.link_message, "bvlc-result");
    assert_eq!(summary.parse_error, None);
    assert!(summary.problem.unwrap().starts_with("NAK"));
    assert_eq!(summary.pdu, None);

    let summary = summarize_frame(Framing::Bip, &[0x81, 0x0A, 0x00]);
    assert!(summary.parse_error.is_some());
    assert!(summary.problem.unwrap().starts_with("malformed"));
}

#[test]
fn an_i_am_names_its_device_and_a_broadcast_is_marked() {
    let frame = bip(&npdu(&i_am(1234), false), true);
    let summary = summarize_frame(Framing::Bip, &frame);
    assert_eq!(summary.device, Some(1234));
    assert!(summary.broadcast);
    assert_eq!(summary.service.as_deref(), Some("i-am"));
}

#[test]
fn mstp_frames_name_their_stations_and_carry_their_npdu() {
    let token = mstp(FrameType::TOKEN, 2, 1, &[]);
    let summary = summarize_frame(Framing::Mstp, &token);
    assert_eq!(summary.link_message, "token");
    assert_eq!(summary.link_source.as_deref(), Some("mstp:1"));
    assert_eq!(summary.link_destination.as_deref(), Some("mstp:2"));
    assert_eq!(summary.pdu, None);

    let data = npdu(&i_am(5), false);
    let frame = mstp(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 255, 5, &data);
    let summary = summarize_frame(Framing::Mstp, &frame);
    assert!(summary.broadcast);
    assert_eq!(summary.device, Some(5));

    // A bad data CRC is a problem, as the monitor would count it.
    let mut bad = frame.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0xFF;
    assert!(summarize_frame(Framing::Mstp, &bad).problem.is_some());
}

#[test]
fn sc_and_bip6_frames_give_their_vmacs() {
    let data = npdu(&rp_request(3), true);
    let summary = summarize_frame(Framing::Sc, &sc_npdu(&data, Some([0xFF; 6])));
    assert_eq!(summary.link_message, "encapsulated-npdu");
    assert_eq!(summary.link_source.as_deref(), Some("02:00:00:00:00:01"));
    assert!(summary.broadcast);
    assert_eq!(summary.invoke_id, Some(3));

    let summary = summarize_frame(Framing::Bip6, &bip6_broadcast(&data));
    assert_eq!(summary.link_source.as_deref(), Some("bip6:0x000102"));
    assert!(summary.broadcast);
    assert_eq!(summary.service.as_deref(), Some("read-property"));
}

#[test]
fn an_sc_nak_names_its_error_code() {
    // A BACnet/SC connection code (#2434) and an older one, by name.
    let summary = summarize_frame(Framing::Sc, &sc_nak(169));
    assert_eq!(summary.link_message, "result");
    assert_eq!(summary.parse_error, None);
    assert_eq!(
        summary.problem.as_deref(),
        Some("NAK websocket-closed-by-peer")
    );
    let summary = summarize_frame(Framing::Sc, &sc_nak(151));
    assert_eq!(summary.problem.as_deref(), Some("NAK node-duplicate-vmac"));
    // A code with no name stays a number.
    let summary = summarize_frame(Framing::Sc, &sc_nak(300));
    assert_eq!(summary.problem.as_deref(), Some("NAK 300"));
}

/// A BVLC function its annex leaves unassigned is a parse failure on every
/// BVLC link, and the message goes unnamed (`malformed`) on each: Annex J.2
/// stops at 0x0B, Table U-1 at 0x0C with 0x0B removed, Table AB-1 at 0x0C.
#[test]
fn an_unassigned_bvlc_function_is_a_parse_error_on_every_link() {
    let cases: [(Framing, &[u8]); 6] = [
        (Framing::Bip, &[0x81, 0x0C, 0x00, 0x04]),
        (Framing::Bip, &[0x81, 0x7F, 0x00, 0x04]),
        (Framing::Bip6, &[0x82, 0x0B, 0x00, 0x07, 0, 1, 2]),
        (Framing::Bip6, &[0x82, 0x7F, 0x00, 0x04]),
        (Framing::Sc, &[0x0D, 0x00, 0x00, 0x01]),
        (Framing::Sc, &[0x7F, 0x00, 0x00, 0x01]),
    ];
    for (framing, frame) in cases {
        let summary = summarize_frame(framing, frame);
        let error = summary
            .parse_error
            .clone()
            .unwrap_or_else(|| panic!("{framing:?} {frame:02X?}: {summary:?}"));
        assert!(error.contains("unknown"), "{framing:?}: {error}");
        assert_eq!(summary.problem, summary.parse_error);
        assert_eq!(summary.link_message, "malformed", "{framing:?}");
    }
    // The last assigned function of each still reads.
    let last = summarize_frame(Framing::Sc, &[0x0C, 0x00, 0x00, 0x01]);
    assert_eq!(
        (last.link_message.as_str(), last.parse_error),
        ("proprietary-message", None)
    );
}

/// Octets of one link read as another are a parse failure, never a quiet
/// summary of an odd message: B/IP's type octet 0x81 is no BACnet/SC
/// function, nor is B/IPv6's 0x82 or an MS/TP preamble's 0x55.
#[test]
fn a_frame_read_as_another_link_is_a_parse_error() {
    let data = npdu(&rp_request(1), true);
    let frames = [
        (Framing::Bip, bip(&data, false)),
        (Framing::Bip6, bip6_broadcast(&data)),
        (Framing::Sc, sc_npdu(&data, None)),
        (
            Framing::Mstp,
            mstp(FrameType::BACNET_DATA_EXPECTING_REPLY, 5, 1, &data),
        ),
    ];
    for (own, frame) in &frames {
        assert_eq!(summarize_frame(*own, frame).parse_error, None, "{own:?}");
        for (other, _) in &frames {
            if other == own {
                continue;
            }
            let summary = summarize_frame(*other, frame);
            assert!(
                summary.parse_error.is_some(),
                "{own:?} octets read as {other:?}: {summary:?}"
            );
        }
    }
}

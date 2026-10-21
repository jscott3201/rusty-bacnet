use bacnet_transport::mstp::codec::FrameType;
use bacnet_types::enums::ConfirmedServiceChoice;

use super::{summarize_frame, Framing};
use crate::decode::testing::*;

#[test]
fn a_bip_request_shows_its_function_pdu_invoke_id_and_service() {
    let frame = bip(&npdu(&rp_request(7), true), false);
    let summary = summarize_frame(Framing::Bip, &frame);
    assert_eq!(summary.link, "original-unicast-npdu");
    assert_eq!(summary.pdu.as_deref(), Some("Confirmed-Request"));
    assert_eq!(summary.invoke_id, Some(7));
    assert_eq!(summary.service.as_deref(), Some("read-property"));
    assert_eq!(summary.problem, None);
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
    assert_eq!(
        summary.problem.as_deref(),
        Some("property:unknown-property")
    );

    let summary = summarize_frame(Framing::Bip, &bip_nak());
    assert_eq!(summary.link, "bvlc-result");
    assert!(summary.problem.unwrap().starts_with("NAK"));
    assert_eq!(summary.pdu, None);

    let summary = summarize_frame(Framing::Bip, &[0x81, 0x0A, 0x00]);
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
    assert_eq!(summary.link, "token");
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
    assert_eq!(summary.link, "encapsulated-npdu");
    assert_eq!(summary.link_source.as_deref(), Some("02:00:00:00:00:01"));
    assert!(summary.broadcast);
    assert_eq!(summary.invoke_id, Some(3));

    let summary = summarize_frame(Framing::Bip6, &bip6_broadcast(&data));
    assert_eq!(summary.link_source.as_deref(), Some("bip6:0x000102"));
    assert!(summary.broadcast);
    assert_eq!(summary.service.as_deref(), Some("read-property"));
}

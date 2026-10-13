//! `FrameType`'s open set of codes (#1684): every octet converts and comes
//! back, the ten types Clause 9.3 defines have names, and every other octet,
//! reserved (8-31, 34-127) or proprietary (128-255), prints as its decimal
//! number.

use bacnet_mstp_codec::FrameType;

/// The types Clause 9.3 defines, by octet, in order.
const NAMED: [(u8, &str); 10] = [
    (0, "TOKEN"),
    (1, "POLL_FOR_MASTER"),
    (2, "REPLY_TO_POLL_FOR_MASTER"),
    (3, "TEST_REQUEST"),
    (4, "TEST_RESPONSE"),
    (5, "BACNET_DATA_EXPECTING_REPLY"),
    (6, "BACNET_DATA_NOT_EXPECTING_REPLY"),
    (7, "REPLY_POSTPONED"),
    (32, "BACNET_EXTENDED_DATA_EXPECTING_REPLY"),
    (33, "BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY"),
];

#[test]
fn all_named_lists_the_ten_defined_types_in_order() {
    let listed: Vec<(u8, &str)> = FrameType::ALL_NAMED
        .iter()
        .map(|&(name, frame_type)| (frame_type.to_raw(), name))
        .collect();
    assert_eq!(listed, NAMED);
    let constants = [
        FrameType::TOKEN,
        FrameType::POLL_FOR_MASTER,
        FrameType::REPLY_TO_POLL_FOR_MASTER,
        FrameType::TEST_REQUEST,
        FrameType::TEST_RESPONSE,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        FrameType::REPLY_POSTPONED,
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
        FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
    ];
    assert_eq!(constants.map(FrameType::to_raw), NAMED.map(|(raw, _)| raw));
}

#[test]
fn every_octet_round_trips_and_only_the_defined_ones_have_names() {
    for raw in 0..=u8::MAX {
        let frame_type = FrameType::from_raw(raw);
        assert_eq!(frame_type.to_raw(), raw);
        assert_eq!(frame_type, FrameType::from_raw(raw));

        let name = NAMED
            .iter()
            .find(|&&(named, _)| named == raw)
            .map(|&(_, name)| name);
        let reserved = (8..=31).contains(&raw) || (34..=127).contains(&raw);
        let proprietary = raw >= 128;
        assert_eq!(name.is_none(), reserved || proprietary, "{raw}");
        assert_eq!(frame_type.name(), name, "{raw}");
        assert_eq!(frame_type.is_defined(), name.is_some(), "{raw}");

        assert_eq!(frame_type.is_proprietary(), proprietary, "{raw}");
        assert_eq!(frame_type.is_cobs(), (32..=127).contains(&raw), "{raw}");
        assert_eq!(
            frame_type.is_extended_data(),
            matches!(raw, 32 | 33),
            "{raw}"
        );
        assert_eq!(
            frame_type.allows_data(),
            !matches!(raw, 0 | 1 | 2 | 7),
            "{raw}"
        );

        let (debug, display) = match name {
            Some(name) => (format!("FrameType::{name}"), name.to_string()),
            None => (format!("FrameType({raw})"), raw.to_string()),
        };
        assert_eq!(format!("{frame_type:?}"), debug);
        assert_eq!(frame_type.to_string(), display);
    }
}

//! The written-configuration format's kind 5 (#2364): a Description wrapped
//! around kinds 2, 3 and 4. The octets it writes, that a file without a
//! Description keeps its plain kind, and what it refuses.

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::{SnapshotErrorKind, write_config};
use crate::write_config::{
    ChannelWriteConfig, ReportingWriteConfig, TimerWriteConfig, WriteConfigSnapshot,
};

fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 7).unwrap()
}

/// One snapshot of each kind kind 5 wraps, for its object, with
/// `description`.
fn described(description: Option<&str>) -> [(ObjectIdentifier, WriteConfigSnapshot); 3] {
    let description = description.map(String::from);
    let timer = TimerWriteConfig {
        default_timeout: Some(90_000),
        description: description.clone(),
        ..Default::default()
    };
    let channel = ChannelWriteConfig {
        channel_number: Some(12),
        description: description.clone(),
        ..Default::default()
    };
    let reporting = ReportingWriteConfig {
        notification_class: Some(9),
        description,
        ..Default::default()
    };
    [
        (oid(ObjectType::TIMER), WriteConfigSnapshot::Timer(timer)),
        (
            oid(ObjectType::CHANNEL),
            WriteConfigSnapshot::Channel(channel),
        ),
        (
            oid(ObjectType::ANALOG_INPUT),
            WriteConfigSnapshot::Reporting(reporting),
        ),
    ]
}

/// A Description wraps each kind: kind 5, its length and octets, then the
/// plain file's kind and body unchanged, which round-trips.
#[test]
fn a_description_wraps_each_kind_and_round_trips() {
    let plain = described(None);
    for (index, (object, snapshot)) in described(Some("Lobby")).into_iter().enumerate() {
        let octets = write_config::encode(object, &snapshot).unwrap();
        let unwrapped = write_config::encode(object, &plain[index].1).unwrap();
        assert_ne!(unwrapped[12], write_config::KIND_WITH_DESCRIPTION);
        let mut expected = unwrapped[..12].to_vec();
        expected.extend_from_slice(&[write_config::KIND_WITH_DESCRIPTION, 0, 0, 0, 5]);
        expected.extend_from_slice(b"Lobby");
        expected.extend_from_slice(&unwrapped[12..]);
        assert_eq!(octets, expected, "{object}");
        assert_eq!(write_config::decode(object, &octets), Ok(snapshot));
    }
}

/// Without a Description the reporting kind's octets are the ones #2331
/// wrote, and an empty written Description is kept as one.
#[test]
fn a_file_without_a_description_keeps_its_plain_octets() {
    let [_, _, (object, snapshot)] = described(None);
    let octets = write_config::encode(object, &snapshot).unwrap();
    assert_eq!(
        octets[12..],
        [
            write_config::KIND_REPORTING,
            1,
            0,
            0,
            0,
            9,
            0,
            0,
            0,
            0,
            0,
            0
        ]
    );
    let [_, _, (object, snapshot)] = described(Some(""));
    let octets = write_config::encode(object, &snapshot).unwrap();
    assert_eq!(
        octets[12..17],
        [write_config::KIND_WITH_DESCRIPTION, 0, 0, 0, 0]
    );
    assert_eq!(write_config::decode(object, &octets), Ok(snapshot));
}

/// A Description of 16 KiB is kept; one octet more is refused both ways.
#[test]
fn a_description_past_its_cap_is_refused_both_ways() {
    let longest = "d".repeat(16 * 1024);
    for (object, snapshot) in described(Some(&longest)) {
        let octets = write_config::encode(object, &snapshot).unwrap();
        assert_eq!(write_config::decode(object, &octets), Ok(snapshot));
    }
    let longer = format!("{longest}d");
    for (object, snapshot) in described(Some(&longer)) {
        let error = write_config::encode(object, &snapshot).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{object}");
    }
    let [_, _, (object, snapshot)] = described(Some(&longest));
    let mut octets = write_config::encode(object, &snapshot).unwrap();
    // The length field, one past the cap, with the octet it names.
    octets[13..17].copy_from_slice(&(16 * 1024 + 1_u32).to_be_bytes());
    octets.insert(17, b'd');
    let error = write_config::decode(object, &octets).unwrap_err();
    assert!(error.is_damage(), "{error:?}");
}

/// What decode refuses inside kind 5: a Description that isn't UTF-8, a
/// wrapped kind 1 or 5, which encode never writes, a wrapped kind of another
/// object type, and a wrapped kind this build doesn't know.
#[test]
fn what_kind_5_refuses() {
    let [(object, snapshot), ..] = described(Some("Lobby"));
    let octets = write_config::encode(object, &snapshot).unwrap();
    let patched = |at: usize, value: u8| {
        let mut octets = octets.clone();
        octets[at] = value;
        write_config::decode(object, &octets).unwrap_err().kind()
    };
    assert_eq!(patched(17, 0xFF), SnapshotErrorKind::Damaged);
    // The wrapped kind octet follows the five octets of "Lobby".
    let wrapped = 22;
    assert_eq!(octets[wrapped], write_config::KIND_TIMER);
    for kind in [
        write_config::KIND_TIMER_SET_ONLY,
        write_config::KIND_WITH_DESCRIPTION,
    ] {
        assert_eq!(patched(wrapped, kind), SnapshotErrorKind::Damaged, "{kind}");
    }
    assert_eq!(
        patched(wrapped, write_config::KIND_CHANNEL),
        SnapshotErrorKind::OtherObject
    );
    assert_eq!(patched(wrapped, 9), SnapshotErrorKind::UnknownKind(9));
}

/// A kind 5 file cut anywhere is refused, and so is one with octets past
/// its end.
#[test]
fn every_cut_of_a_described_file_is_refused() {
    for (object, snapshot) in described(Some("Lobby")) {
        let octets = write_config::encode(object, &snapshot).unwrap();
        for cut in 0..octets.len() {
            assert!(
                write_config::decode(object, &octets[..cut]).is_err(),
                "{object}: a file cut at {cut} loaded"
            );
        }
        let mut longer = octets.clone();
        longer.push(0);
        let error = write_config::decode(object, &longer).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{object}");
    }
}

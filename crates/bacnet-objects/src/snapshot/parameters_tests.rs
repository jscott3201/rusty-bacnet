//! The written-configuration format's kind 6 (#2331): the event algorithm
//! parameters after a kind 4 body. The octets it writes, that a file
//! without a parameter keeps kind 4, and what it refuses.

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::ObjectIdentifier;

use super::write_config::{self, parameters};
use super::{SnapshotError, SnapshotErrorKind};
use crate::write_config::{AlgorithmWriteConfig, ReportingWriteConfig, WriteConfigSnapshot};

const ANALOG: [ObjectType; 3] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
];

fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 7).unwrap()
}

/// Every parameter an analog object keeps.
fn every_parameter() -> AlgorithmWriteConfig {
    AlgorithmWriteConfig {
        time_delay: Some(30),
        time_delay_normal: Some(0x0102_0304),
        high_limit: Some(80.5),
        low_limit: Some(-12.25),
        deadband: Some(1.5),
        limit_enable: Some(LimitEnable::HIGH_LIMIT_ENABLE),
    }
}

/// A snapshot with a Notification_Class of 9 and `algorithm`.
fn snapshot(algorithm: AlgorithmWriteConfig) -> WriteConfigSnapshot {
    WriteConfigSnapshot::Reporting(ReportingWriteConfig {
        notification_class: Some(9),
        algorithm,
        ..Default::default()
    })
}

fn encode(kind: ObjectType, algorithm: AlgorithmWriteConfig) -> Result<Vec<u8>, SnapshotError> {
    write_config::encode(oid(kind), &snapshot(algorithm))
}

/// Each parameter alone, with its mask bit and its octets.
fn each_alone() -> Vec<(AlgorithmWriteConfig, u32, Vec<u8>)> {
    let all = every_parameter();
    let none = AlgorithmWriteConfig::default;
    vec![
        (
            AlgorithmWriteConfig {
                time_delay: all.time_delay,
                ..none()
            },
            1 << 0,
            vec![0, 0, 0, 30],
        ),
        (
            AlgorithmWriteConfig {
                time_delay_normal: all.time_delay_normal,
                ..none()
            },
            1 << 1,
            vec![1, 2, 3, 4],
        ),
        (
            AlgorithmWriteConfig {
                high_limit: all.high_limit,
                ..none()
            },
            1 << 2,
            80.5f32.to_be_bytes().to_vec(),
        ),
        (
            AlgorithmWriteConfig {
                low_limit: all.low_limit,
                ..none()
            },
            1 << 3,
            (-12.25f32).to_be_bytes().to_vec(),
        ),
        (
            AlgorithmWriteConfig {
                deadband: all.deadband,
                ..none()
            },
            1 << 4,
            1.5f32.to_be_bytes().to_vec(),
        ),
        (
            AlgorithmWriteConfig {
                limit_enable: all.limit_enable,
                ..none()
            },
            1 << 5,
            vec![0x02],
        ),
    ]
}

/// A snapshot without a parameter keeps kind 4 and the octets #2331 PR 1
/// wrote; one with a parameter is the same body under kind 6, then the mask
/// and the parameters in bit order.
#[test]
fn parameters_follow_the_kind_4_body_and_none_keeps_kind_4() {
    for kind in ANALOG {
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        assert_eq!(
            plain[12..],
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
            ],
            "{kind}"
        );
        let octets = encode(kind, every_parameter()).unwrap();
        let mut expected = plain[..12].to_vec();
        expected.push(write_config::KIND_REPORTING_PARAMETERS);
        expected.extend_from_slice(&plain[13..]);
        expected.extend_from_slice(&[0, 0, 0, 0x3F]);
        for (_, _, row) in each_alone() {
            expected.extend_from_slice(&row);
        }
        assert_eq!(octets, expected, "{kind}");
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(every_parameter()))
        );
    }
}

/// Each parameter alone sets its own bit, takes its own octets and comes
/// back as itself, so no row can stand in for another.
#[test]
fn each_parameter_has_its_own_bit() {
    let kind = ObjectType::ANALOG_VALUE;
    let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
    for (algorithm, bit, row) in each_alone() {
        let octets = encode(kind, algorithm.clone()).unwrap();
        let mut tail = bit.to_be_bytes().to_vec();
        tail.extend_from_slice(&row);
        assert_eq!(octets[plain.len()..], tail, "{algorithm:?}");
        assert_eq!(parameters::mask(&algorithm), bit);
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(algorithm))
        );
    }
}

/// Kind 5 wraps kind 6 as it wraps kind 4.
#[test]
fn a_description_wraps_the_parameters_kind() {
    let object = oid(ObjectType::ANALOG_INPUT);
    let WriteConfigSnapshot::Reporting(mut config) = snapshot(every_parameter()) else {
        unreachable!()
    };
    let plain = write_config::encode(object, &WriteConfigSnapshot::Reporting(config.clone()));
    let plain = plain.unwrap();
    config.description = Some("Lobby".into());
    let described = WriteConfigSnapshot::Reporting(config);
    let octets = write_config::encode(object, &described).unwrap();
    let mut expected = plain[..12].to_vec();
    expected.extend_from_slice(&[write_config::KIND_WITH_DESCRIPTION, 0, 0, 0, 5]);
    expected.extend_from_slice(b"Lobby");
    expected.extend_from_slice(&plain[12..]);
    assert_eq!(octets, expected);
    assert_eq!(write_config::decode(object, &octets), Ok(described));
}

/// A file cut anywhere, or going on past its last parameter, is refused.
#[test]
fn cut_and_over_long_files_are_refused() {
    let object = oid(ObjectType::ANALOG_INPUT);
    let octets = encode(ObjectType::ANALOG_INPUT, every_parameter()).unwrap();
    for cut in 12..octets.len() {
        let error = write_config::decode(object, &octets[..cut]).unwrap_err();
        assert!(error.is_damage(), "cut at {cut}: {error:?}");
    }
    let mut long = octets.clone();
    long.push(0);
    let error = write_config::decode(object, &long).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
}

/// The octets of a kind 6 file for `kind` whose tail after the kind 4 body
/// is `tail`.
fn with_tail(kind: ObjectType, tail: &[u8]) -> Vec<u8> {
    let mut octets = encode(kind, AlgorithmWriteConfig::default()).unwrap();
    octets[12] = write_config::KIND_REPORTING_PARAMETERS;
    octets.extend_from_slice(tail);
    octets
}

/// A mask bit this build doesn't know is a newer build's file, refused as
/// an unknown kind and not as damage; an empty mask is damage, since a save
/// without a parameter writes kind 4.
#[test]
fn an_unknown_bit_is_an_unknown_kind_and_an_empty_mask_is_damage() {
    let kind = ObjectType::ANALOG_INPUT;
    for mask in [1u32 << 6, 1 << 31, (1 << 6) | 1] {
        let mut tail = mask.to_be_bytes().to_vec();
        tail.extend_from_slice(&[0; 8]);
        let error = write_config::decode(oid(kind), &with_tail(kind, &tail)).unwrap_err();
        assert_eq!(
            error.kind(),
            SnapshotErrorKind::UnknownKind(write_config::KIND_REPORTING_PARAMETERS),
            "mask {mask:#x}"
        );
        assert!(!error.is_damage());
    }
    let error = write_config::decode(oid(kind), &with_tail(kind, &[0; 4])).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
}

/// The ten object types with a reporting store that keep no parameter in
/// this build.
const OTHERS: [ObjectType; 10] = [
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_ZONE,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::ALERT_ENROLLMENT,
];

/// `octets`, a kind 6 file, under kind 5 with a Description of "Lobby".
fn described(octets: &[u8]) -> Vec<u8> {
    let mut wrapped = octets[..12].to_vec();
    wrapped.extend_from_slice(&[write_config::KIND_WITH_DESCRIPTION, 0, 0, 0, 5]);
    wrapped.extend_from_slice(b"Lobby");
    wrapped.extend_from_slice(&octets[12..]);
    wrapped
}

/// That decoding `octets` for `object` is refused as a sound file of a
/// newer build, an unknown kind 6, and not as damage, bare and under kind
/// 5: a backend with two copies must stop there, not fall back to the older
/// one.
fn assert_a_newer_builds(object: ObjectIdentifier, octets: &[u8], what: &str) {
    for file in [octets.to_vec(), described(octets)] {
        let error = write_config::decode(object, &file).unwrap_err();
        assert_eq!(
            error.kind(),
            SnapshotErrorKind::UnknownKind(write_config::KIND_REPORTING_PARAMETERS),
            "{object} {what}"
        );
        assert!(!error.is_damage(), "{object} {what}");
    }
}

/// A Binary Value file with Time_Delay's bit, which the next build keeps
/// for it, is that build's sound file: an unknown kind here, never damage,
/// bare and under kind 5, with its row or cut before it.
#[test]
fn a_known_bit_the_type_does_not_keep_is_an_unknown_kind_not_damage() {
    let kind = ObjectType::BINARY_VALUE;
    let with_row = with_tail(kind, &[0, 0, 0, 1, 0, 0, 0, 30]);
    assert_a_newer_builds(oid(kind), &with_row, "with its row");
    // Refused at the mask, before a row is read: the mask alone answers the
    // same, where a kept row cut short is truncation.
    let mask_only = with_tail(kind, &[0, 0, 0, 1]);
    assert_a_newer_builds(oid(kind), &mask_only, "cut after the mask");
    // An analog file with a kept bit beside one it doesn't keep, too.
    let analog = ObjectType::ANALOG_INPUT;
    let mixed = with_tail(analog, &[0, 0, 0, 0x41, 0, 0, 0, 30]);
    assert_a_newer_builds(oid(analog), &mixed, "a kept bit and a later one");
}

/// A parameter is kept only for an object type that has it. For each of
/// the ten other types with a reporting store: `keeps_parameter` names
/// none, a snapshot never takes kind 6 (one without a parameter keeps kind
/// 4 and one with a parameter is refused as it is encoded), a kind 6 file
/// with any parameter is refused as an unknown kind, and one with an empty
/// mask is damage.
#[test]
fn a_parameter_of_another_object_type_is_refused() {
    let properties = [
        P::TIME_DELAY,
        P::TIME_DELAY_NORMAL,
        P::HIGH_LIMIT,
        P::LOW_LIMIT,
        P::DEADBAND,
        P::LIMIT_ENABLE,
    ];
    for kind in ANALOG {
        for property in properties {
            assert!(write_config::keeps_parameter(kind, property), "{kind}");
        }
        assert!(!write_config::keeps_parameter(kind, P::NOTIFICATION_CLASS));
        assert!(!write_config::keeps_parameter(kind, P::ALARM_VALUES));
    }
    for property in properties {
        assert!(!write_config::keeps_parameter(ObjectType::TIMER, property));
    }
    for kind in OTHERS {
        for property in properties {
            assert!(!write_config::keeps_parameter(kind, property), "{kind}");
        }
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        assert_eq!(plain[12], write_config::KIND_REPORTING, "{kind}");
        for (algorithm, bit, row) in each_alone() {
            let error = encode(kind, algorithm).unwrap_err();
            assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{kind}");
            let mut tail = bit.to_be_bytes().to_vec();
            tail.extend_from_slice(&row);
            assert_a_newer_builds(oid(kind), &with_tail(kind, &tail), "one bit");
        }
        let error = write_config::decode(oid(kind), &with_tail(kind, &[0; 4])).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{kind}");
        assert!(error.is_damage(), "{kind}");
    }
}

/// A value the object refuses on the wire is refused as it is encoded and
/// as damage in a file: a limit or deadband that isn't finite, a negative
/// deadband, Limit_Enable bits past its two.
#[test]
fn a_value_no_write_leaves_is_refused() {
    let kind = ObjectType::ANALOG_INPUT;
    let none = AlgorithmWriteConfig::default;
    let mut refused: Vec<(AlgorithmWriteConfig, u32, Vec<u8>)> = Vec::new();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let row = value.to_be_bytes().to_vec();
        refused.push((
            AlgorithmWriteConfig {
                high_limit: Some(value),
                ..none()
            },
            1 << 2,
            row.clone(),
        ));
        refused.push((
            AlgorithmWriteConfig {
                low_limit: Some(value),
                ..none()
            },
            1 << 3,
            row.clone(),
        ));
        refused.push((
            AlgorithmWriteConfig {
                deadband: Some(value),
                ..none()
            },
            1 << 4,
            row,
        ));
    }
    refused.push((
        AlgorithmWriteConfig {
            deadband: Some(-0.5),
            ..none()
        },
        1 << 4,
        (-0.5f32).to_be_bytes().to_vec(),
    ));
    refused.push((
        AlgorithmWriteConfig {
            limit_enable: Some(LimitEnable::from_bits_retain(0x04)),
            ..none()
        },
        1 << 5,
        vec![0x04],
    ));
    for (algorithm, bit, row) in refused {
        let error = encode(kind, algorithm.clone()).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{algorithm:?}");
        let mut tail = bit.to_be_bytes().to_vec();
        tail.extend_from_slice(&row);
        let error = write_config::decode(oid(kind), &with_tail(kind, &tail)).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{algorithm:?}");
    }
    // The edges a write takes are kept: a deadband of zero, either sign.
    for deadband in [0.0, -0.0, f32::MAX] {
        let algorithm = AlgorithmWriteConfig {
            deadband: Some(deadband),
            ..none()
        };
        let octets = encode(kind, algorithm.clone()).unwrap();
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(algorithm))
        );
    }
}

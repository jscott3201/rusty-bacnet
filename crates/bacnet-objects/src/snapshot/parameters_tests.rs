//! The written-configuration format's kind 6 (#2331): the event algorithm
//! parameters after a kind 4 body. The octets it writes, that a file
//! without a parameter keeps kind 4, which object types keep which
//! parameter, and what it refuses. The list row's own refusals are in
//! `parameter_list_tests`.

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::enums::{BinaryPV, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::ObjectIdentifier;

use super::write_config::{self, parameters};
use super::{SnapshotError, SnapshotErrorKind};
use crate::write_config::{AlgorithmWriteConfig, ReportingWriteConfig, WriteConfigSnapshot};

pub(super) const ANALOG: [ObjectType; 3] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
];

/// The thirteen object types with a reporting store.
pub(super) const REPORTERS: [ObjectType; 13] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
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

/// The mask of the parameters `kind` keeps, written out here by hand: the
/// codec's own table is what these tests check.
pub(super) fn kept(kind: ObjectType) -> u32 {
    match kind {
        ObjectType::ANALOG_INPUT | ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE => 0x3F,
        ObjectType::BINARY_INPUT | ObjectType::BINARY_VALUE => 0x43,
        ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT => 0x03,
        ObjectType::MULTI_STATE_INPUT | ObjectType::MULTI_STATE_VALUE => 0x83,
        _ => 0,
    }
}

/// What the build before this one kept (#2331 PR 2): the first six rows on
/// the analog types, nothing elsewhere.
fn kept_before(kind: ObjectType) -> u32 {
    if ANALOG.contains(&kind) { 0x3F } else { 0 }
}

pub(super) fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 7).unwrap()
}

/// Each parameter in bit order: its property and its octets in a file.
fn rows() -> [(P, Vec<u8>); 8] {
    [
        (P::TIME_DELAY, vec![0, 0, 0, 30]),
        (P::TIME_DELAY_NORMAL, vec![1, 2, 3, 4]),
        (P::HIGH_LIMIT, 80.5f32.to_be_bytes().to_vec()),
        (P::LOW_LIMIT, (-12.25f32).to_be_bytes().to_vec()),
        (P::DEADBAND, 1.5f32.to_be_bytes().to_vec()),
        (P::LIMIT_ENABLE, vec![0x02]),
        (P::ALARM_VALUE, vec![1]),
        (
            P::ALARM_VALUES,
            vec![0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 1, 1, 2, 3, 4],
        ),
    ]
}

/// The parameters whose bits are in `mask`, with the values [`rows`] gives
/// their octets for.
pub(super) fn holding(mask: u32) -> AlgorithmWriteConfig {
    let has = |bit: u32| mask & (1 << bit) != 0;
    AlgorithmWriteConfig {
        time_delay: has(0).then_some(30),
        time_delay_normal: has(1).then_some(0x0102_0304),
        high_limit: has(2).then_some(80.5),
        low_limit: has(3).then_some(-12.25),
        deadband: has(4).then_some(1.5),
        limit_enable: has(5).then_some(LimitEnable::HIGH_LIMIT_ENABLE),
        alarm_value: has(6).then_some(BinaryPV::ACTIVE),
        alarm_values: has(7).then(|| vec![3, 1, 0x0102_0304]),
    }
}

/// The octets after a kind 4 body for the parameters in `mask`: the mask,
/// then each row in bit order.
fn tail(mask: u32) -> Vec<u8> {
    let mut tail = mask.to_be_bytes().to_vec();
    for (bit, (_, row)) in rows().into_iter().enumerate() {
        if mask & (1 << bit) != 0 {
            tail.extend_from_slice(&row);
        }
    }
    tail
}

/// A snapshot with a Notification_Class of 9 and `algorithm`.
pub(super) fn snapshot(algorithm: AlgorithmWriteConfig) -> WriteConfigSnapshot {
    WriteConfigSnapshot::Reporting(ReportingWriteConfig {
        notification_class: Some(9),
        algorithm,
        ..Default::default()
    })
}

pub(super) fn encode(
    kind: ObjectType,
    algorithm: AlgorithmWriteConfig,
) -> Result<Vec<u8>, SnapshotError> {
    write_config::encode(oid(kind), &snapshot(algorithm))
}

/// The octets of a kind 6 file for `kind` whose tail after the kind 4 body
/// is `tail`.
pub(super) fn with_tail(kind: ObjectType, tail: &[u8]) -> Vec<u8> {
    let mut octets = encode(kind, AlgorithmWriteConfig::default()).unwrap();
    octets[12] = write_config::KIND_REPORTING_PARAMETERS;
    octets.extend_from_slice(tail);
    octets
}

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

/// The table names every parameter once: the bits are 0 to 7 in order,
/// each property has its own, and each type keeps what [`kept`] says and
/// nothing else, a Timer nothing at all.
#[test]
fn each_type_keeps_its_own_parameters() {
    for (bit, (property, _)) in rows().into_iter().enumerate() {
        assert_eq!(parameters::bit_of(property), Some(1 << bit), "{property:?}");
        assert_eq!(parameters::mask(&holding(1 << bit)), 1 << bit);
        for kind in REPORTERS {
            assert_eq!(
                write_config::keeps_parameter(kind, property),
                kept(kind) & (1 << bit) != 0,
                "{kind} {property:?}"
            );
        }
        assert!(!write_config::keeps_parameter(ObjectType::TIMER, property));
    }
    for kind in REPORTERS {
        assert_eq!(parameters::kept(kind), kept(kind), "{kind}");
        for property in [P::NOTIFICATION_CLASS, P::FAULT_VALUES, P::PRESENT_VALUE] {
            assert!(!write_config::keeps_parameter(kind, property), "{kind}");
        }
    }
    assert_eq!(parameters::mask(&holding(0xFF)), 0xFF);
    assert_eq!(parameters::mask(&AlgorithmWriteConfig::default()), 0);
}

/// On every type, a snapshot without a parameter keeps kind 4 and the
/// octets #2331 PR 1 wrote; one with every parameter the type keeps is the
/// same body under kind 6, then the mask and the parameters in bit order.
#[test]
fn parameters_follow_the_kind_4_body_and_none_keeps_kind_4() {
    for kind in REPORTERS {
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
        if kept(kind) == 0 {
            continue;
        }
        let octets = encode(kind, holding(kept(kind))).unwrap();
        let mut expected = plain[..12].to_vec();
        expected.push(write_config::KIND_REPORTING_PARAMETERS);
        expected.extend_from_slice(&plain[13..]);
        expected.extend_from_slice(&tail(kept(kind)));
        assert_eq!(octets, expected, "{kind}");
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(holding(kept(kind))))
        );
    }
}

/// An analog file is what #2331 PR 2 wrote, octet for octet: the mask of
/// its six rows and their octets, spelled out.
#[test]
fn an_analog_file_keeps_the_octets_of_the_build_before() {
    let mut expected = vec![0, 0, 0, 0x3F, 0, 0, 0, 30, 1, 2, 3, 4];
    expected.extend_from_slice(&[0x42, 0xA1, 0, 0, 0xC1, 0x44, 0, 0, 0x3F, 0xC0, 0, 0, 0x02]);
    for kind in ANALOG {
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        let octets = encode(kind, holding(0x3F)).unwrap();
        assert_eq!(octets[plain.len()..], expected, "{kind}");
    }
}

/// On each type, each parameter it keeps sets its own bit alone, takes its
/// own octets and comes back as itself, so no row can stand in for another.
#[test]
fn each_parameter_has_its_own_bit() {
    for kind in REPORTERS {
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        for bit in (0..8).map(|bit| 1 << bit) {
            if kept(kind) & bit == 0 {
                continue;
            }
            let algorithm = holding(bit);
            let octets = encode(kind, algorithm.clone()).unwrap();
            assert_eq!(octets[plain.len()..], tail(bit), "{kind} {algorithm:?}");
            assert_eq!(
                write_config::decode(oid(kind), &octets),
                Ok(snapshot(algorithm))
            );
        }
    }
}

/// Kind 5 wraps kind 6 as it wraps kind 4.
#[test]
fn a_description_wraps_the_parameters_kind() {
    for kind in [ObjectType::ANALOG_INPUT, ObjectType::MULTI_STATE_VALUE] {
        let object = oid(kind);
        let WriteConfigSnapshot::Reporting(mut config) = snapshot(holding(kept(kind))) else {
            unreachable!()
        };
        let plain = write_config::encode(object, &WriteConfigSnapshot::Reporting(config.clone()));
        let plain = plain.unwrap();
        config.description = Some("Lobby".into());
        let wrapped = WriteConfigSnapshot::Reporting(config);
        let octets = write_config::encode(object, &wrapped).unwrap();
        assert_eq!(octets, described(&plain), "{kind}");
        assert_eq!(write_config::decode(object, &octets), Ok(wrapped));
    }
}

/// A file cut anywhere, or going on past its last parameter, is refused, on
/// each type that keeps a parameter.
#[test]
fn cut_and_over_long_files_are_refused() {
    for kind in REPORTERS {
        if kept(kind) == 0 {
            continue;
        }
        let octets = encode(kind, holding(kept(kind))).unwrap();
        for cut in 12..octets.len() {
            let error = write_config::decode(oid(kind), &octets[..cut]).unwrap_err();
            assert!(error.is_damage(), "{kind} cut at {cut}: {error:?}");
        }
        let mut long = octets.clone();
        long.push(0);
        let error = write_config::decode(oid(kind), &long).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{kind}");
    }
}

/// A mask bit no build knows is a newer build's file, refused as an unknown
/// kind and not as damage, on every type; an empty mask is damage, since a
/// save without a parameter writes kind 4.
#[test]
fn an_unknown_bit_is_an_unknown_kind_and_an_empty_mask_is_damage() {
    for kind in REPORTERS {
        for mask in [1u32 << 8, 1 << 31, (1 << 8) | (kept(kind) & 1)] {
            let mut tail = mask.to_be_bytes().to_vec();
            tail.extend_from_slice(&[0; 8]);
            let file = with_tail(kind, &tail);
            assert_a_newer_builds(oid(kind), &file, &format!("mask {mask:#x}"));
        }
        let error = write_config::decode(oid(kind), &with_tail(kind, &[0; 4])).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{kind}");
        assert!(error.is_damage(), "{kind}");
    }
}

/// A parameter is kept only for an object type that has it. On each of the
/// thirteen types, each parameter the type doesn't keep is refused as it is
/// encoded, and a kind 6 file with its bit is a newer build's: an unknown
/// kind and never damage, bare and under kind 5, with its row, cut after
/// the mask (so it is refused before a row is read), and beside a row the
/// type does keep.
#[test]
fn a_parameter_of_another_object_type_is_refused() {
    for kind in REPORTERS {
        for bit in (0..8).map(|bit| 1u32 << bit) {
            if kept(kind) & bit != 0 {
                continue;
            }
            let error = encode(kind, holding(bit)).unwrap_err();
            assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{kind}");
            let what = format!("bit {bit:#x}");
            assert_a_newer_builds(oid(kind), &with_tail(kind, &tail(bit)), &what);
            let mask_only = with_tail(kind, &bit.to_be_bytes());
            assert_a_newer_builds(oid(kind), &mask_only, &what);
            let beside = bit | (kept(kind) & 1);
            assert_a_newer_builds(oid(kind), &with_tail(kind, &tail(beside)), &what);
        }
    }
}

/// The build before this one (#2331 PR 2) refuses a file with a set bit
/// outside what it kept for the object's type as an unknown kind, at the
/// mask. Every file this build writes that the older one couldn't have is
/// such a file: kind 6 with a mask holding a bit the older build didn't
/// keep for the type. This build's reader, standing where the older one
/// stood (a type it keeps none of the rows for), refuses the same tail the
/// same way. An analog file, which the older build wrote too, has no such
/// bit.
#[test]
fn the_build_before_refuses_what_only_this_build_writes_as_an_unknown_kind() {
    let stand_ins = REPORTERS.into_iter().filter(|kind| kept(*kind) == 0);
    for kind in REPORTERS {
        let new = kept(kind) & !kept_before(kind);
        assert_eq!(
            new == 0,
            ANALOG.contains(&kind) || kept(kind) == 0,
            "{kind}"
        );
        for mask in (1..=0xFFu32).filter(|mask| mask & !kept(kind) == 0) {
            let octets = encode(kind, holding(mask)).unwrap();
            let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
            assert_eq!(octets[12], write_config::KIND_REPORTING_PARAMETERS);
            let written = u32::from_be_bytes(octets[plain.len()..][..4].try_into().unwrap());
            assert_eq!(written, mask, "{kind}");
            // Outside the older build's rows for the type whenever this
            // build's rows for it go past them.
            assert_eq!(written & !kept_before(kind) != 0, new != 0, "{kind}");
            if new == 0 {
                continue;
            }
            for stand_in in stand_ins.clone() {
                let file = with_tail(stand_in, &octets[plain.len()..]);
                assert_a_newer_builds(oid(stand_in), &file, &format!("{kind} {mask:#x}"));
            }
        }
    }
}

/// A value the object refuses on the wire is refused as it is encoded and
/// as damage in a file: a limit or deadband that isn't finite, a negative
/// deadband, Limit_Enable bits past its two, an Alarm_Value past ACTIVE.
#[test]
fn a_value_no_write_leaves_is_refused() {
    let none = AlgorithmWriteConfig::default;
    let mut refused: Vec<(ObjectType, AlgorithmWriteConfig, u32, Vec<u8>)> = Vec::new();
    let analog = ObjectType::ANALOG_INPUT;
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let row = value.to_be_bytes().to_vec();
        let high = AlgorithmWriteConfig {
            high_limit: Some(value),
            ..none()
        };
        let low = AlgorithmWriteConfig {
            low_limit: Some(value),
            ..none()
        };
        let deadband = AlgorithmWriteConfig {
            deadband: Some(value),
            ..none()
        };
        refused.push((analog, high, 1 << 2, row.clone()));
        refused.push((analog, low, 1 << 3, row.clone()));
        refused.push((analog, deadband, 1 << 4, row));
    }
    let negative = AlgorithmWriteConfig {
        deadband: Some(-0.5),
        ..none()
    };
    refused.push((analog, negative, 1 << 4, (-0.5f32).to_be_bytes().to_vec()));
    let bits = AlgorithmWriteConfig {
        limit_enable: Some(LimitEnable::from_bits_retain(0x04)),
        ..none()
    };
    refused.push((analog, bits, 1 << 5, vec![0x04]));
    for kind in [ObjectType::BINARY_INPUT, ObjectType::BINARY_VALUE] {
        for raw in [2u8, 255] {
            let value = AlgorithmWriteConfig {
                alarm_value: Some(BinaryPV::from_raw(u32::from(raw))),
                ..none()
            };
            refused.push((kind, value, 1 << 6, vec![raw]));
        }
    }
    for (kind, algorithm, bit, row) in refused {
        let error = encode(kind, algorithm.clone()).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{algorithm:?}");
        let mut tail = bit.to_be_bytes().to_vec();
        tail.extend_from_slice(&row);
        let error = write_config::decode(oid(kind), &with_tail(kind, &tail)).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{algorithm:?}");
    }
    // An Alarm_Value past one octet can't be written as one.
    let wide = AlgorithmWriteConfig {
        alarm_value: Some(BinaryPV::from_raw(0x100)),
        ..none()
    };
    let error = encode(ObjectType::BINARY_VALUE, wide).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    // The edges a write takes are kept: a deadband of zero, either sign,
    // and both binary states.
    for deadband in [0.0, -0.0, f32::MAX] {
        let algorithm = AlgorithmWriteConfig {
            deadband: Some(deadband),
            ..none()
        };
        let octets = encode(analog, algorithm.clone()).unwrap();
        assert_eq!(
            write_config::decode(oid(analog), &octets),
            Ok(snapshot(algorithm))
        );
    }
    for state in [BinaryPV::INACTIVE, BinaryPV::ACTIVE] {
        let algorithm = AlgorithmWriteConfig {
            alarm_value: Some(state),
            ..none()
        };
        let kind = ObjectType::BINARY_INPUT;
        let octets = encode(kind, algorithm.clone()).unwrap();
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(algorithm))
        );
    }
}

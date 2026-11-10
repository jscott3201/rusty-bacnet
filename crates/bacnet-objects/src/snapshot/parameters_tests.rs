//! The written-configuration format's kind 6 (#2331): the event algorithm
//! parameters after a kind 4 body. The octets it writes, that a file
//! without a parameter keeps kind 4, which object types keep which
//! parameter, and what it refuses. The list rows' own refusals, and the
//! rule each object type holds a listed state to, are in
//! `parameter_list_tests`, and an Event Enrollment's two rows held in their
//! BACnet encoding in `enrollment_parameters_tests`.

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::constructed::{BACnetEventParameter, FaultParameters};
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
        ObjectType::ACCESS_DOOR => 0x383,
        ObjectType::ACCESS_ZONE => 0x83,
        // Time_Delay_Normal, Event_Parameters and Fault_Parameters.
        ObjectType::EVENT_ENROLLMENT => 0xC02,
        _ => 0,
    }
}

/// What the build before this one kept (#2331 PR 4): this build's rows on
/// every type but Event Enrollment, and nothing on that.
fn kept_before(kind: ObjectType) -> u32 {
    match kind {
        ObjectType::EVENT_ENROLLMENT => 0,
        kind => kept(kind),
    }
}

/// Every row's bit.
const ALL: u32 = 0xFFF;

/// How many rows the table has.
const ROWS: u32 = 12;

/// The Event_Parameters [`holding`] gives an Event Enrollment.
pub(super) fn event_parameters() -> BACnetEventParameter {
    BACnetEventParameter::OutOfRange {
        time_delay: 30,
        low_limit: 10.0,
        high_limit: 80.5,
        deadband: 1.5,
    }
}

/// The Fault_Parameters [`holding`] gives an Event Enrollment.
pub(super) fn fault_parameters() -> FaultParameters {
    FaultParameters::FaultOutOfRange {
        min_normal: -5.0,
        max_normal: 50.0,
    }
}

/// The row of [`event_parameters`]: its length, then the out-of-range
/// choice under tag 5 with its four members under tags 0 to 3.
const EVENT_ROW: [u8; 23] = [
    0, 0, 0, 19, 0x5E, 0x09, 30, 0x1C, 0x41, 0x20, 0, 0, 0x2C, 0x42, 0xA1, 0, 0, 0x3C, 0x3F, 0xC0,
    0, 0, 0x5F,
];

/// The row of [`fault_parameters`]: its length, then the fault-out-of-range
/// choice under tag 6, each bound a Double inside its own tag.
const FAULT_ROW: [u8; 30] = [
    0, 0, 0, 26, 0x6E, 0x0E, 0x55, 8, 0xC0, 0x14, 0, 0, 0, 0, 0, 0, 0x0F, 0x1E, 0x55, 8, 0x40,
    0x49, 0, 0, 0, 0, 0, 0, 0x1F, 0x6F,
];

/// A list of states only `kind`'s rule takes whole: a multi-state object's
/// goes past both enumerations, an Access Door's holds a named state an
/// Access Zone has none for, and an Access Zone's a proprietary state
/// below where an Access Door's begin.
pub(super) fn listed(kind: ObjectType) -> Vec<u32> {
    match kind {
        ObjectType::ACCESS_DOOR => vec![3, 8, 0x0102],
        ObjectType::ACCESS_ZONE => vec![3, 1, 100],
        _ => vec![3, 1, 0x0102_0304],
    }
}

pub(super) fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 7).unwrap()
}

/// A list row's octets: the count, then each state.
fn list_row(states: &[u32]) -> Vec<u8> {
    let mut row = (states.len() as u32).to_be_bytes().to_vec();
    for state in states {
        row.extend_from_slice(&state.to_be_bytes());
    }
    row
}

/// Each parameter in bit order: its property and its octets in a file for
/// an object of `kind`.
fn rows(kind: ObjectType) -> [(P, Vec<u8>); ROWS as usize] {
    let list = list_row(&listed(kind));
    // What the build before this one wrote for a multi-state object.
    if !matches!(kind, ObjectType::ACCESS_DOOR | ObjectType::ACCESS_ZONE) {
        assert_eq!(list, [0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 1, 1, 2, 3, 4]);
    }
    [
        (P::TIME_DELAY, vec![0, 0, 0, 30]),
        (P::TIME_DELAY_NORMAL, vec![1, 2, 3, 4]),
        (P::HIGH_LIMIT, 80.5f32.to_be_bytes().to_vec()),
        (P::LOW_LIMIT, (-12.25f32).to_be_bytes().to_vec()),
        (P::DEADBAND, 1.5f32.to_be_bytes().to_vec()),
        (P::LIMIT_ENABLE, vec![0x02]),
        (P::ALARM_VALUE, vec![1]),
        (P::ALARM_VALUES, list),
        (P::FAULT_VALUES, vec![0, 0, 0, 1, 0, 0, 0, 5]),
        (
            P::MASKED_ALARM_VALUES,
            vec![0, 0, 0, 2, 0, 0, 0, 7, 0, 0, 0xFF, 0xFF],
        ),
        (P::EVENT_PARAMETERS, EVENT_ROW.to_vec()),
        (P::FAULT_PARAMETERS, FAULT_ROW.to_vec()),
    ]
}

/// The parameters whose bits are in `mask`, with the values [`rows`] gives
/// their octets for on an object of `kind`.
pub(super) fn holding(kind: ObjectType, mask: u32) -> AlgorithmWriteConfig {
    let has = |bit: u32| mask & (1 << bit) != 0;
    AlgorithmWriteConfig {
        time_delay: has(0).then_some(30),
        time_delay_normal: has(1).then_some(0x0102_0304),
        high_limit: has(2).then_some(80.5),
        low_limit: has(3).then_some(-12.25),
        deadband: has(4).then_some(1.5),
        limit_enable: has(5).then_some(LimitEnable::HIGH_LIMIT_ENABLE),
        alarm_value: has(6).then_some(BinaryPV::ACTIVE),
        alarm_values: has(7).then(|| listed(kind)),
        fault_values: has(8).then(|| vec![5]),
        masked_alarm_values: has(9).then(|| vec![7, 0xFFFF]),
        event_parameters: has(10).then(event_parameters),
        fault_parameters: has(11).then(fault_parameters),
    }
}

/// The octets after a kind 4 body for the parameters in `mask` on an
/// object of `kind`: the mask, then each row in bit order.
fn tail(kind: ObjectType, mask: u32) -> Vec<u8> {
    let mut tail = mask.to_be_bytes().to_vec();
    for (bit, (_, row)) in rows(kind).into_iter().enumerate() {
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

/// The table names every parameter once: the bits are 0 to 11 in order,
/// each property has its own, and each type keeps what [`kept`] says and
/// nothing else, a Timer nothing at all.
#[test]
fn each_type_keeps_its_own_parameters() {
    let analog = ObjectType::ANALOG_INPUT;
    assert_eq!(parameters::ROWS.len(), rows(analog).len());
    for (bit, (property, _)) in rows(analog).into_iter().enumerate() {
        assert_eq!(parameters::ROWS[bit], (1 << bit, property));
        assert_eq!(parameters::bit_of(property), Some(1 << bit), "{property:?}");
        assert_eq!(parameters::mask(&holding(analog, 1 << bit)), 1 << bit);
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
        let never = [
            P::NOTIFICATION_CLASS,
            P::PRESENT_VALUE,
            P::DOOR_ALARM_STATE,
            P::OCCUPANCY_UPPER_LIMIT,
            P::OCCUPANCY_LOWER_LIMIT,
            P::EVENT_TYPE,
            P::FAULT_TYPE,
            P::OBJECT_PROPERTY_REFERENCE,
        ];
        for property in never {
            assert!(!write_config::keeps_parameter(kind, property), "{kind}");
        }
    }
    assert_eq!(parameters::mask(&holding(analog, ALL)), ALL);
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
        let octets = encode(kind, holding(kind, kept(kind))).unwrap();
        let mut expected = plain[..12].to_vec();
        expected.push(write_config::KIND_REPORTING_PARAMETERS);
        expected.extend_from_slice(&plain[13..]);
        expected.extend_from_slice(&tail(kind, kept(kind)));
        assert_eq!(octets, expected, "{kind}");
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(holding(kind, kept(kind))))
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
        let octets = encode(kind, holding(kind, 0x3F)).unwrap();
        assert_eq!(octets[plain.len()..], expected, "{kind}");
    }
}

/// A binary or multi-state file is what #2331 PR 3 wrote, octet for octet:
/// the mask of its rows and their octets, spelled out.
#[test]
fn a_state_file_keeps_the_octets_of_the_build_before() {
    let delays = [0, 0, 0, 30, 1, 2, 3, 4];
    let list = [0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 1, 1, 2, 3, 4];
    let files: [(ObjectType, u8, &[u8]); 6] = [
        (ObjectType::BINARY_INPUT, 0x43, &[1]),
        (ObjectType::BINARY_VALUE, 0x43, &[1]),
        (ObjectType::BINARY_OUTPUT, 0x03, &[]),
        (ObjectType::MULTI_STATE_OUTPUT, 0x03, &[]),
        (ObjectType::MULTI_STATE_INPUT, 0x83, &list),
        (ObjectType::MULTI_STATE_VALUE, 0x83, &list),
    ];
    for (kind, mask, own) in files {
        let mut expected = vec![0, 0, 0, mask];
        expected.extend_from_slice(&delays);
        expected.extend_from_slice(own);
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        let octets = encode(kind, holding(kind, u32::from(mask))).unwrap();
        assert_eq!(octets[plain.len()..], expected, "{kind}");
    }
}

/// An Access Door's and an Access Zone's file, spelled out: the two delays,
/// then each list as its count and its states, in bit order.
#[test]
fn an_access_file_holds_its_delays_and_then_its_lists() {
    let delays = [0, 0, 0, 30, 1, 2, 3, 4];
    let mut door = vec![0, 0, 0x03, 0x83];
    door.extend_from_slice(&delays);
    door.extend_from_slice(&[0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 8, 0, 0, 1, 2]);
    door.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 5]);
    door.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 7, 0, 0, 0xFF, 0xFF]);
    let mut zone = vec![0, 0, 0, 0x83];
    zone.extend_from_slice(&delays);
    zone.extend_from_slice(&[0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 100]);
    for (kind, expected) in [
        (ObjectType::ACCESS_DOOR, door),
        (ObjectType::ACCESS_ZONE, zone),
    ] {
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        let octets = encode(kind, holding(kind, kept(kind))).unwrap();
        assert_eq!(octets[plain.len()..], expected, "{kind}");
    }
}

/// On each type, each parameter it keeps sets its own bit alone, takes its
/// own octets and comes back as itself, so no row can stand in for another.
#[test]
fn each_parameter_has_its_own_bit() {
    for kind in REPORTERS {
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        for bit in (0..ROWS).map(|bit| 1 << bit) {
            if kept(kind) & bit == 0 {
                continue;
            }
            let algorithm = holding(kind, bit);
            let octets = encode(kind, algorithm.clone()).unwrap();
            assert_eq!(
                octets[plain.len()..],
                tail(kind, bit),
                "{kind} {algorithm:?}"
            );
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
    for kind in [
        ObjectType::ANALOG_INPUT,
        ObjectType::MULTI_STATE_VALUE,
        ObjectType::ACCESS_DOOR,
        ObjectType::ACCESS_ZONE,
        ObjectType::EVENT_ENROLLMENT,
    ] {
        let object = oid(kind);
        let WriteConfigSnapshot::Reporting(mut config) = snapshot(holding(kind, kept(kind))) else {
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
        let octets = encode(kind, holding(kind, kept(kind))).unwrap();
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
        for mask in [1u32 << ROWS, 1 << 31, (1 << ROWS) | (kept(kind) & 2)] {
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
        for bit in (0..ROWS).map(|bit| 1u32 << bit) {
            if kept(kind) & bit != 0 {
                continue;
            }
            let error = encode(kind, holding(kind, bit)).unwrap_err();
            assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{kind}");
            let what = format!("bit {bit:#x}");
            assert_a_newer_builds(oid(kind), &with_tail(kind, &tail(kind, bit)), &what);
            let mask_only = with_tail(kind, &bit.to_be_bytes());
            assert_a_newer_builds(oid(kind), &mask_only, &what);
            let beside = bit | (kept(kind) & 2);
            let file = with_tail(kind, &tail(kind, beside));
            assert_a_newer_builds(oid(kind), &file, &what);
        }
    }
}

/// The build before this one (#2331 PR 4) refuses a file with a set bit
/// outside what it kept for the object's type as an unknown kind, at the
/// mask. Every file this build writes that the older one couldn't have is
/// such a file: kind 6 for an Event Enrollment, for which the older build
/// kept no row, so any mask at all is outside its rows, a mask of
/// Time_Delay_Normal alone included. This build's reader, standing where
/// the older one stood (a type it keeps none of the rows for), refuses the
/// same tail the same way, before a row is read: with the mask alone, too.
/// A file of any other type, which the older build wrote too, has no such
/// bit.
#[test]
fn the_build_before_refuses_what_only_this_build_writes_as_an_unknown_kind() {
    let stand_ins = REPORTERS.into_iter().filter(|kind| kept(*kind) == 0);
    assert_eq!(stand_ins.clone().count(), 1, "the Alert Enrollment");
    for kind in REPORTERS {
        let new = kept(kind) & !kept_before(kind);
        assert_eq!(new != 0, kind == ObjectType::EVENT_ENROLLMENT, "{kind}");
        for mask in (1..=ALL).filter(|mask| mask & !kept(kind) == 0) {
            let octets = encode(kind, holding(kind, mask)).unwrap();
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
                let what = format!("{kind} {mask:#x}");
                let file = with_tail(stand_in, &octets[plain.len()..]);
                assert_a_newer_builds(oid(stand_in), &file, &what);
                let mask_only = with_tail(stand_in, &mask.to_be_bytes());
                assert_a_newer_builds(oid(stand_in), &mask_only, &what);
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

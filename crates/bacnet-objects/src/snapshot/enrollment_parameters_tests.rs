//! An Event Enrollment's two rows of the written-configuration format's
//! kind 6 that are held in their BACnet encoding (#2331): Event_Parameters
//! and Fault_Parameters, each a length and then the value. That every
//! choice a write leaves comes back as itself and encodes to the octets it
//! was read from, the cap, and what a row refuses: octets that don't
//! decode, octets after the value, an encoding no save writes, and a length
//! past the cap or the file, before anything is read.

use bacnet_encoding::constructed::{encode_event_parameter, encode_fault_parameters};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    BACnetProprietaryPropertyState, ChangeOfValueCriteria, FaultParameters,
};
use bacnet_types::enums::{
    AccessEvent, LifeSafetyState, ObjectType, PropertyIdentifier as P, TimerState,
};
use bacnet_types::primitives::{ObjectIdentifier, StatusFlags};
use bytes::BytesMut;

use super::parameters_tests::{encode, oid, snapshot, with_tail};
use super::{SnapshotErrorKind, write_config};
use crate::write_config::AlgorithmWriteConfig;

const ENROLLMENT: ObjectType = ObjectType::EVENT_ENROLLMENT;
/// The rows' bits of the presence mask.
const EVENT: u32 = 1 << 10;
const FAULT: u32 = 1 << 11;
/// The most octets a row's value takes.
const CAP: usize = 64 * 1024;

fn reference(index: Option<u32>, device: Option<u32>) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 12).unwrap(),
        property_identifier: P::PRESENT_VALUE.to_raw(),
        property_array_index: index,
        device_identifier: device.map(|d| ObjectIdentifier::new(ObjectType::DEVICE, d).unwrap()),
    }
}

/// The items an `extended` parameter list may hold: application values (an
/// Unsigned, a REAL and a BOOLEAN here) and a reference inside tag 0.
fn extended_items() -> Vec<u8> {
    let mut items = BytesMut::from(&[0x21, 0x07, 0x44, 0x42, 0x28, 0, 0, 0x11, 0x0E][..]);
    bacnet_encoding::constructed::encode_device_object_property_reference(
        &mut items,
        &reference(None, None),
    );
    items.extend_from_slice(&[0x0F]);
    items.to_vec()
}

/// Every alternative of Event_Parameters a write leaves on an enrollment,
/// the short and the long of each list, the legacy octet layout and the
/// preserved tags among them.
pub(crate) fn every_event_choice() -> Vec<BACnetEventParameter> {
    use BACnetEventParameter as E;
    use BACnetPropertyStates as S;
    let proprietary = |constructed: bool| {
        let state = if constructed {
            BACnetProprietaryPropertyState::constructed(
                200,
                vec![0x09, 0x01, 0x1E, 0x21, 0x05, 0x1F],
            )
        } else {
            BACnetProprietaryPropertyState::primitive(64, vec![1, 2, 3])
        };
        S::Other(state.unwrap())
    };
    vec![
        E::None,
        E::ChangeOfBitstring {
            time_delay: 0,
            bitmask: (0, vec![]),
            list_of_values: vec![],
        },
        E::ChangeOfBitstring {
            time_delay: u32::MAX,
            bitmask: (3, vec![0xFF, 0xF8]),
            list_of_values: vec![(3, vec![0x12, 0x30]), (0, vec![]), (7, vec![0x80])],
        },
        E::ChangeOfState {
            time_delay: 4,
            list_of_values: vec![],
        },
        E::ChangeOfState {
            time_delay: 300,
            list_of_values: vec![
                S::BooleanValue(true),
                S::BinaryValue(1),
                S::State(3),
                S::UnsignedValue(u32::MAX),
                S::IntegerValue(-70_000),
                S::LiftFault(u32::MAX),
                proprietary(false),
                proprietary(true),
            ],
        },
        E::ChangeOfValue {
            time_delay: 1,
            criteria: ChangeOfValueCriteria::Bitmask {
                unused_bits: 4,
                data: vec![0xA0],
            },
        },
        E::ChangeOfValue {
            time_delay: 1,
            criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(-0.0),
        },
        E::CommandFailure {
            time_delay: 9,
            feedback_property_reference: reference(Some(2), Some(77)),
        },
        E::FloatingLimit {
            time_delay: 65_536,
            setpoint_reference: reference(None, None),
            low_diff_limit: 1.5,
            high_diff_limit: f32::MAX,
            deadband: 0.0,
        },
        E::OutOfRange {
            time_delay: 30,
            low_limit: f32::MIN,
            high_limit: f32::INFINITY,
            deadband: 0.25,
        },
        E::ChangeOfLifeSafety {
            time_delay: 2,
            list_of_life_safety_alarm_values: vec![LifeSafetyState::ALARM],
            list_of_alarm_values: vec![LifeSafetyState::from_raw(300), LifeSafetyState::FAULT],
            mode_property_reference: reference(None, Some(1)),
        },
        E::Extended {
            vendor_id: 555,
            extended_event_type: 0x0102_0304,
            parameters: vec![],
        },
        E::Extended {
            vendor_id: u16::MAX,
            extended_event_type: 0,
            parameters: extended_items(),
        },
        E::BufferReady {
            notification_threshold: 10,
            previous_notification_count: u32::MAX,
        },
        E::UnsignedRange {
            time_delay: 0,
            low_limit: 0,
            high_limit: u64::MAX,
        },
        E::AccessEvent {
            list_of_access_events: vec![AccessEvent::GRANTED, AccessEvent::from_raw(600)],
            access_event_time_reference: reference(Some(1), None),
        },
        E::DoubleOutOfRange {
            time_delay: 5,
            low_limit: -1.0e300,
            high_limit: 1.0e300,
            deadband: 0.5,
        },
        E::SignedOutOfRange {
            time_delay: 5,
            low_limit: i32::MIN,
            high_limit: i32::MAX,
            deadband: 1 << 40,
        },
        E::UnsignedOutOfRange {
            time_delay: 5,
            low_limit: 1,
            high_limit: 1 << 33,
            deadband: 0,
        },
        E::ChangeOfCharacterstring {
            time_delay: 8,
            list_of_alarm_values: vec![String::new(), "FAULT".into(), "Störung ⚠".into()],
        },
        E::ChangeOfStatusFlags {
            time_delay: 8,
            selected_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
        },
        E::ChangeOfDiscreteValue { time_delay: 255 },
        E::ChangeOfTimer {
            time_delay: 256,
            alarm_values: vec![TimerState::EXPIRED, TimerState::RUNNING],
            update_time_reference: reference(None, None),
        },
        // The legacy octet layout, empty and not.
        E::Opaque {
            tag: 0xFF,
            data: vec![],
        },
        E::Opaque {
            tag: 0xFF,
            data: vec![0, 0, 0x20, 0x42, 0, 0, 0xA0, 0x40],
        },
        // A tag past the standard's own, and a proprietary one.
        E::Opaque {
            tag: 23,
            data: vec![0x09, 0x05, 0x1E, 0x21, 0x01, 0x1F],
        },
        E::Opaque {
            tag: 200,
            data: vec![],
        },
    ]
}

/// Every alternative of Fault_Parameters a write leaves.
pub(crate) fn every_fault_choice() -> Vec<FaultParameters> {
    use FaultParameters as F;
    vec![
        F::FaultNone,
        F::FaultCharacterString {
            fault_values: vec![],
        },
        F::FaultCharacterString {
            fault_values: vec!["OFFLINE".into(), String::new(), "défaut".into()],
        },
        F::FaultExtended {
            vendor_id: 555,
            extended_fault_type: u32::MAX,
            parameters: extended_items(),
        },
        F::FaultLifeSafety {
            fault_values: vec![LifeSafetyState::FAULT, LifeSafetyState::from_raw(4000)],
            mode_property_reference: reference(Some(3), Some(9)),
        },
        F::FaultState {
            fault_values: vec![
                BACnetPropertyStates::State(2),
                BACnetPropertyStates::BooleanValue(false),
            ],
        },
        F::FaultStatusFlags {
            reference: reference(None, None),
        },
        F::FaultOutOfRange {
            min_normal: -5.0,
            max_normal: 1.0e300,
        },
        F::FaultListed {
            reference: reference(None, Some(4_194_302)),
        },
    ]
}

fn event(parameters: BACnetEventParameter) -> AlgorithmWriteConfig {
    AlgorithmWriteConfig {
        event_parameters: Some(parameters),
        ..Default::default()
    }
}

fn fault(parameters: FaultParameters) -> AlgorithmWriteConfig {
    AlgorithmWriteConfig {
        fault_parameters: Some(parameters),
        ..Default::default()
    }
}

/// The tail of a file holding one row: its bit, the `length` it states and
/// the `octets` after it.
fn row(bit: u32, length: u32, octets: &[u8]) -> Vec<u8> {
    let mut tail = bit.to_be_bytes().to_vec();
    tail.extend_from_slice(&length.to_be_bytes());
    tail.extend_from_slice(octets);
    tail
}

/// The row holding all of `octets`.
fn whole(bit: u32, octets: &[u8]) -> Vec<u8> {
    row(bit, octets.len() as u32, octets)
}

fn decode_tail(tail: &[u8]) -> Result<AlgorithmWriteConfig, super::SnapshotError> {
    write_config::decode(oid(ENROLLMENT), &with_tail(ENROLLMENT, tail)).map(|snapshot| {
        let crate::write_config::WriteConfigSnapshot::Reporting(config) = snapshot else {
            unreachable!("a reporting kind")
        };
        config.algorithm
    })
}

/// A file holding `config`, which decodes to it and encodes again to the
/// same octets; returns the octets after the kind 4 body.
fn stable(config: &AlgorithmWriteConfig, what: &str) -> Vec<u8> {
    let plain = encode(ENROLLMENT, AlgorithmWriteConfig::default()).unwrap();
    let file = encode(ENROLLMENT, config.clone())
        .unwrap_or_else(|error| panic!("{what}: a written value isn't saved: {error:?}"));
    let decoded = write_config::decode(oid(ENROLLMENT), &file)
        .unwrap_or_else(|error| panic!("{what}: a saved file doesn't load: {error:?}"));
    assert_eq!(decoded, snapshot(config.clone()), "{what}");
    assert_eq!(
        write_config::encode(oid(ENROLLMENT), &decoded).unwrap(),
        file,
        "{what}: a load and a save change the file"
    );
    file[plain.len()..].to_vec()
}

/// Every choice of both properties is held as its length and its BACnet
/// encoding, comes back as itself, and encodes again to the octets it was
/// read from, so a load followed by a save of another row leaves this one
/// as it was.
#[test]
fn every_choice_comes_back_and_encodes_to_the_octets_it_was_read_from() {
    let events = every_event_choice();
    // Each alternative the type has is among them.
    let mut tags: Vec<u8> = events.iter().map(BACnetEventParameter::tag).collect();
    tags.sort_unstable();
    tags.dedup();
    let assigned = [
        0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 13, 14, 15, 16, 17, 18, 20, 21, 22,
    ];
    assert_eq!(tags[..assigned.len()], assigned);
    assert_eq!(tags[assigned.len()..], [23, 200, 255]);
    for parameters in events {
        let what = format!("{parameters:?}");
        let mut octets = BytesMut::new();
        encode_event_parameter(&mut octets, &parameters).unwrap();
        let tail = stable(&event(parameters), &what);
        assert_eq!(tail, whole(EVENT, &octets), "{what}");
    }
    let faults = every_fault_choice();
    let mut types: Vec<u32> = faults.iter().map(|f| f.fault_type().to_raw()).collect();
    types.sort_unstable();
    types.dedup();
    assert_eq!(types, [0, 1, 2, 3, 4, 5, 6, 7]);
    for parameters in faults {
        let what = format!("{parameters:?}");
        let mut octets = BytesMut::new();
        encode_fault_parameters(&mut octets, &parameters).unwrap();
        let tail = stable(&fault(parameters), &what);
        assert_eq!(tail, whole(FAULT, &octets), "{what}");
    }
}

/// A limit that isn't a number is no value `==` finds again, but its octets
/// are kept and written back as they were: the object takes such a write.
#[test]
fn limits_that_are_not_numbers_keep_their_octets() {
    let nan = f32::from_bits(0x7FC0_1234);
    let parameters = BACnetEventParameter::OutOfRange {
        time_delay: 1,
        low_limit: nan,
        high_limit: f32::NEG_INFINITY,
        deadband: nan,
    };
    let file = encode(ENROLLMENT, event(parameters)).unwrap();
    let decoded = write_config::decode(oid(ENROLLMENT), &file).unwrap();
    assert_eq!(
        write_config::encode(oid(ENROLLMENT), &decoded).unwrap(),
        file
    );
    let bounds = FaultParameters::FaultOutOfRange {
        min_normal: f64::from_bits(0x7FF8_0000_0000_4321),
        max_normal: f64::NAN,
    };
    let file = encode(ENROLLMENT, fault(bounds)).unwrap();
    let decoded = write_config::decode(oid(ENROLLMENT), &file).unwrap();
    assert_eq!(
        write_config::encode(oid(ENROLLMENT), &decoded).unwrap(),
        file
    );
}

/// Both rows with Time_Delay_Normal: the mask, the delay, then each row in
/// bit order, alone and together.
#[test]
fn the_rows_follow_time_delay_normal_in_bit_order() {
    let config = AlgorithmWriteConfig {
        time_delay_normal: Some(9),
        event_parameters: Some(BACnetEventParameter::None),
        fault_parameters: Some(FaultParameters::FaultNone),
        ..Default::default()
    };
    let tail = stable(&config, "all three");
    // `none` is a NULL under tag 20 of the one CHOICE and tag 0 of the other.
    assert_eq!(
        tail,
        [
            0, 0, 0x0C, 0x02, 0, 0, 0, 9, 0, 0, 0, 2, 0xF8, 20, 0, 0, 0, 1, 0x08
        ]
    );
}

/// Octets that are no value of the property are damage: an empty row, a
/// tag the CHOICE doesn't have, a value cut short inside its row, and
/// octets after the value inside its row.
#[test]
fn a_row_that_is_not_one_whole_value_is_damage() {
    let mut out_of_range = BytesMut::new();
    let parameters = BACnetEventParameter::OutOfRange {
        time_delay: 30,
        low_limit: 10.0,
        high_limit: 80.5,
        deadband: 1.5,
    };
    encode_event_parameter(&mut out_of_range, &parameters).unwrap();
    let mut bounds = BytesMut::new();
    let fault = FaultParameters::FaultOutOfRange {
        min_normal: -5.0,
        max_normal: 50.0,
    };
    encode_fault_parameters(&mut bounds, &fault).unwrap();
    assert!(decode_tail(&whole(EVENT, &out_of_range)).is_ok());
    assert!(decode_tail(&whole(FAULT, &bounds)).is_ok());

    for (bit, value) in [(EVENT, &out_of_range[..]), (FAULT, &bounds[..])] {
        let mut rows: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("an application value", vec![0x21, 0x05]),
            ("a closing tag", vec![0x5F]),
            ("a tag the CHOICE lacks", vec![0x6E, 0x6F, 0x7E, 0x7F]),
            ("all ones", vec![0xFF; 16]),
        ];
        // tag 6 is fault-out-of-range in the one CHOICE; 8 is in neither
        // form an alternative of the other.
        if bit == FAULT {
            rows[3].1 = vec![0x8E, 0x8F];
        }
        for cut in 1..value.len() {
            rows.push(("a value cut short", value[..cut].to_vec()));
        }
        for extra in [vec![0u8], vec![0xF8, 20], value.to_vec()] {
            let mut long = value.to_vec();
            long.extend_from_slice(&extra);
            rows.push(("octets after the value", long));
        }
        for (what, octets) in rows {
            let error = decode_tail(&whole(bit, &octets)).unwrap_err();
            assert_eq!(
                error.kind(),
                SnapshotErrorKind::Damaged,
                "{bit:#x} {what} {octets:02x?}"
            );
        }
    }
}

/// A value in an encoding a save doesn't write is damage, though the
/// decoder a write runs takes it: the wrapped form of the legacy layout, a
/// time delay with a leading zero octet, and fault bounds as REAL.
#[test]
fn a_value_in_an_encoding_no_save_writes_is_damage() {
    let wrapped = [0xFE, 0xFF, 1, 2, 3, 0xFF, 0xFF];
    let padded = [0xFE, 0x15, 0x0A, 0x00, 0x05, 0xFF, 0x15];
    let real_bounds = [
        0x6E, 0x0E, 0x44, 0x3F, 0x80, 0, 0, 0x0F, 0x1E, 0x44, 0x40, 0, 0, 0, 0x1F, 0x6F,
    ];
    use crate::event_enrollment::parameters::{framed_event_parameters, framed_fault_parameters};
    assert!(
        framed_event_parameters(&wrapped).is_ok(),
        "a write takes it"
    );
    assert!(framed_event_parameters(&padded).is_ok(), "a write takes it");
    assert!(
        framed_fault_parameters(&real_bounds).is_ok(),
        "a write takes it"
    );
    for (bit, octets) in [
        (EVENT, &wrapped[..]),
        (EVENT, &padded[..]),
        (FAULT, &real_bounds[..]),
    ] {
        let error = decode_tail(&whole(bit, octets)).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{octets:02x?}");
        assert!(error.to_string().contains("no save writes"), "{error}");
    }
}

/// A row's length is held to the cap before the body is looked at: one past
/// the cap is refused for its length, whether its octets are there or not,
/// and one within the cap that the file doesn't hold is cut short. Neither
/// reads or allocates the octets it promises.
#[test]
fn a_length_past_the_cap_or_the_file_is_refused_before_it_is_read() {
    for bit in [EVENT, FAULT] {
        for length in [CAP as u32 + 1, 1 << 20, u32::MAX] {
            let error = decode_tail(&row(bit, length, &[])).unwrap_err();
            assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{length}");
            assert!(error.to_string().contains("longer than its cap"), "{error}");
        }
        // With its octets behind it too: the length alone refuses it.
        let over = vec![0u8; CAP + 1];
        let error = decode_tail(&whole(bit, &over)).unwrap_err();
        assert!(error.to_string().contains("longer than its cap"), "{error}");
        for length in [1, 19, CAP as u32] {
            let error = decode_tail(&row(bit, length, &[])).unwrap_err();
            assert_eq!(error.kind(), SnapshotErrorKind::Truncated, "{length}");
        }
    }
}

/// The legacy octet layout of `length` octets: its encoding is four octets
/// longer, a tag and a three-octet length.
pub(crate) fn legacy(length: usize) -> BACnetEventParameter {
    BACnetEventParameter::Opaque {
        tag: 0xFF,
        data: vec![0x5A; length],
    }
}

/// A fault-extended value whose encoding is `length` octets: one octet
/// string as its parameters, behind 12 octets of framing.
pub(crate) fn extended_fault(length: usize) -> FaultParameters {
    let mut parameters = vec![0x65, 0xFE];
    let data = length - 12;
    parameters.extend_from_slice(&(data as u16).to_be_bytes());
    parameters.resize(4 + data, 0);
    FaultParameters::FaultExtended {
        vendor_id: 7,
        extended_fault_type: 1,
        parameters,
    }
}

/// A value whose encoding is the cap long is kept and comes back; one octet
/// longer is refused as it is saved, and as damage in a file.
#[test]
fn the_cap_is_64_kib_of_encoding_on_both_rows() {
    let tail = stable(&event(legacy(CAP - 4)), "Event_Parameters at the cap");
    assert_eq!(tail.len(), 8 + CAP);
    let tail = stable(&fault(extended_fault(CAP)), "Fault_Parameters at the cap");
    assert_eq!(tail.len(), 8 + CAP);

    let error = encode(ENROLLMENT, event(legacy(CAP - 3))).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    let error = encode(ENROLLMENT, fault(extended_fault(CAP + 1))).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    let mut octets = BytesMut::new();
    encode_event_parameter(&mut octets, &legacy(CAP - 3)).unwrap();
    assert_eq!(octets.len(), CAP + 1);
    let error = decode_tail(&whole(EVENT, &octets)).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
}

/// The cap takes a list of standard property states as long as the decoder
/// does, each in its longest encoding, so no such list a write carries is
/// too long to keep.
#[test]
fn the_longest_list_of_standard_states_fits_the_cap() {
    let parameters = BACnetEventParameter::ChangeOfState {
        time_delay: u32::MAX,
        list_of_values: vec![BACnetPropertyStates::LiftFault(u32::MAX); 10_000],
    };
    let mut octets = BytesMut::new();
    encode_event_parameter(&mut octets, &parameters).unwrap();
    assert_eq!(octets.len(), 10_000 * 6 + 9);
    assert!(octets.len() <= CAP);
    stable(&event(parameters), "10,000 states");
    let states = FaultParameters::FaultState {
        fault_values: vec![BACnetPropertyStates::LiftFault(u32::MAX); 10_000],
    };
    stable(&fault(states), "10,000 fault states");
}

/// What bounds the decoder inside the cap: a list one entry past the
/// 10,000 it takes, and tags nested past the 32 it follows, are damage and
/// not a panic or a deeper walk.
#[test]
fn the_decoder_is_bounded_inside_the_cap() {
    // change-of-state [1] { [0] 0, [1] { 10,001 BOOLEAN states } }.
    let mut long = vec![0x1E, 0x09, 0x00, 0x1E];
    for _ in 0..10_001 {
        long.extend_from_slice(&[0x09, 0x01]);
    }
    long.extend_from_slice(&[0x1F, 0x1F]);
    assert!(long.len() <= CAP);
    let error = decode_tail(&whole(EVENT, &long)).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
    // The same with 10,000 is a value.
    let at = long.len() - 4;
    long.drain(at..at + 2);
    assert!(decode_tail(&whole(EVENT, &long)).is_ok());

    // A preserved alternative under tag 23 whose body opens tag 1 over and
    // over, closed as often: the cap would hold thousands of levels.
    for depth in [40usize, 30_000] {
        let mut deep = vec![0xFE, 23];
        deep.extend(core::iter::repeat_n(0x1E, depth));
        deep.extend(core::iter::repeat_n(0x1F, depth));
        deep.extend_from_slice(&[0xFF, 23]);
        assert!(deep.len() <= CAP);
        let error = decode_tail(&whole(EVENT, &deep)).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "depth {depth}");
    }
    // fault-extended [2] whose parameters nest the same way.
    let mut deep = vec![0x2E, 0x09, 0x01, 0x19, 0x01, 0x2E];
    deep.extend(core::iter::repeat_n(0x0E, 30_000));
    deep.extend(core::iter::repeat_n(0x0F, 30_000));
    deep.extend_from_slice(&[0x2F, 0x2F]);
    let error = decode_tail(&whole(FAULT, &deep)).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
}

/// A value with no encoding, which no write leaves and only a setter can
/// put on an object, is refused as it is saved.
#[test]
fn a_value_with_no_encoding_is_not_saved() {
    let opaque = BACnetEventParameter::Opaque {
        tag: 5,
        data: vec![],
    };
    let error = encode(ENROLLMENT, event(opaque)).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    let untagged = FaultParameters::FaultExtended {
        vendor_id: 1,
        extended_fault_type: 1,
        parameters: vec![0x1E],
    };
    let error = encode(ENROLLMENT, fault(untagged)).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
}

/// A small deterministic generator, so a failure names the same input on
/// every run.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// One change to a row's octets: an octet replaced, a bit flipped, an octet
/// dropped or added, the end cut off, or a stretch copied over another.
fn mutated(octets: &[u8], random: &mut XorShift) -> Vec<u8> {
    let mut octets = octets.to_vec();
    if octets.is_empty() {
        return vec![random.next() as u8];
    }
    let at = random.below(octets.len());
    match random.below(6) {
        0 => octets[at] = random.next() as u8,
        1 => octets[at] ^= 1 << random.below(8),
        2 => {
            octets.remove(at);
        }
        3 => octets.insert(at, random.next() as u8),
        4 => octets.truncate(at),
        _ => {
            let from = random.below(octets.len());
            let len = random.below(octets.len() - from.max(at) + 1);
            octets.copy_within(from..from + len, at);
        }
    }
    octets
}

/// The decoders on stored octets that aren't a save's: each sample row,
/// changed one to four times over, 2,000 times a sample, is refused or
/// read, never a panic, and a row that is read is the row a save writes,
/// octet for octet.
#[test]
fn changed_rows_are_refused_or_read_back_as_they_are_written() {
    let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
    for parameters in every_event_choice() {
        let mut octets = BytesMut::new();
        encode_event_parameter(&mut octets, &parameters).unwrap();
        rows.push((EVENT, octets.to_vec()));
    }
    for parameters in every_fault_choice() {
        let mut octets = BytesMut::new();
        encode_fault_parameters(&mut octets, &parameters).unwrap();
        rows.push((FAULT, octets.to_vec()));
    }
    let mut random = XorShift(0x2331_5EED_0BAC_0E77);
    let (mut read, mut refused) = (0u32, 0u32);
    for (bit, row) in &rows {
        for _ in 0..2_000 {
            let mut octets = row.clone();
            for _ in 0..=random.below(4) {
                octets = mutated(&octets, &mut random);
            }
            let tail = whole(*bit, &octets);
            match decode_tail(&tail) {
                Ok(config) => {
                    read += 1;
                    let file = encode(ENROLLMENT, config).unwrap();
                    assert!(file.ends_with(&tail), "{bit:#x} {octets:02x?}");
                }
                Err(error) => {
                    refused += 1;
                    assert!(error.is_damage(), "{bit:#x} {octets:02x?}: {error:?}");
                }
            }
        }
    }
    // Both happen: a changed limit or state is still a value, and most
    // other changes aren't.
    assert!(read > 1_000, "{read} read");
    assert!(refused > 10_000, "{refused} refused");
}

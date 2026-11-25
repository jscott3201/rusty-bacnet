//! The per-choice split of `decode_bounded` (#2558) held to the function as
//! it was, kept in `parameters_decode_base`: over every choice's encodings
//! and their truncations, deletions, insertions and single-octet changes,
//! both give the same value, or the same error with the same kind, offset
//! and text. Then one exact error per choice, from inside its own decoder.

use super::parameters_decode::{decode_bounded, decode_notification_parameters};
use super::parameters_decode_base as base;
use super::*;
use alloc::collections::BTreeSet;
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

fn date_time() -> (Date, Time) {
    (
        Date {
            year: 124,
            month: 6,
            day: 15,
            day_of_week: 3,
        },
        Time {
            hour: 10,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
    )
}

fn object(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

const FLAGS: StatusFlags = StatusFlags::IN_ALARM;
const ALL_FLAGS: StatusFlags = StatusFlags::from_bits_retain(0x0f);
const REAL_50: &[u8] = &[0x44, 0x42, 0x48, 0x00, 0x00];

/// Values of every choice the decoder has an arm for, with each optional
/// field present and absent and the integer fields at their extremes.
fn corpus() -> Vec<NotificationParameters> {
    use NotificationParameters as P;
    let mut values = vec![
        P::ChangeOfBitstring {
            referenced_bitstring: (0, vec![]),
            status_flags: StatusFlags::empty(),
        },
        P::ChangeOfBitstring {
            referenced_bitstring: (3, vec![0xf0, 0x08]),
            status_flags: ALL_FLAGS,
        },
        P::ChangeOfValue {
            new_value: ChangeOfValueChoice::ChangedBits {
                unused_bits: 5,
                data: vec![0xa0],
            },
            status_flags: FLAGS,
        },
        P::ChangeOfValue {
            new_value: ChangeOfValueChoice::ChangedValue(-12.5),
            status_flags: ALL_FLAGS,
        },
        P::CommandFailure {
            command_value: vec![0x91, 0x01],
            status_flags: FLAGS,
            feedback_value: vec![0x91, 0x00],
        },
        P::CommandFailure {
            command_value: vec![0x00],
            status_flags: StatusFlags::empty(),
            feedback_value: REAL_50.to_vec(),
        },
        P::FloatingLimit {
            reference_value: 50.0,
            status_flags: FLAGS,
            setpoint_value: 45.0,
            error_limit: 2.0,
        },
        P::OutOfRange {
            exceeding_value: f32::MIN,
            status_flags: ALL_FLAGS,
            deadband: 0.0,
            exceeded_limit: f32::MAX,
        },
        P::ComplexEventType {
            property_values: vec![],
        },
        P::ComplexEventType {
            property_values: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: REAL_50.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
                    property_array_index: Some(3),
                    value: vec![0x21, 0x07],
                    priority: Some(8),
                },
            ],
        },
        P::ChangeOfLifeSafety {
            new_state: LifeSafetyState::PRE_ALARM,
            new_mode: LifeSafetyMode::TEST,
            status_flags: FLAGS,
            operation_expected: LifeSafetyOperation::SILENCE_VISUAL,
        },
        P::ChangeOfLifeSafety {
            new_state: LifeSafetyState::from_raw(u32::MAX),
            new_mode: LifeSafetyMode::from_raw(0),
            status_flags: ALL_FLAGS,
            operation_expected: LifeSafetyOperation::from_raw(u32::MAX),
        },
        P::Extended {
            vendor_id: 0,
            extended_event_type: 0,
            parameters: vec![],
        },
        P::Extended {
            vendor_id: u16::MAX,
            extended_event_type: u32::MAX,
            parameters: vec![0x91, 0x01, 0x44, 0x42, 0x48, 0x00, 0x00],
        },
        P::BufferReady {
            buffer_property: BACnetDeviceObjectPropertyReference {
                object_identifier: object(ObjectType::TREND_LOG, 1),
                property_identifier: PropertyIdentifier::LOG_BUFFER.to_raw(),
                property_array_index: None,
                device_identifier: None,
            },
            previous_notification: 0,
            current_notification: 1,
        },
        P::BufferReady {
            buffer_property: BACnetDeviceObjectPropertyReference {
                object_identifier: object(ObjectType::TREND_LOG, 4_194_302),
                property_identifier: PropertyIdentifier::LOG_BUFFER.to_raw(),
                property_array_index: Some(2),
                device_identifier: Some(object(ObjectType::DEVICE, 9)),
            },
            previous_notification: u32::MAX,
            current_notification: 0,
        },
        P::UnsignedRange {
            exceeding_value: 0,
            status_flags: StatusFlags::empty(),
            exceeded_limit: 0,
        },
        P::UnsignedRange {
            exceeding_value: u64::MAX,
            status_flags: ALL_FLAGS,
            exceeded_limit: 1,
        },
        P::DoubleOutOfRange {
            exceeding_value: f64::MIN_POSITIVE,
            status_flags: FLAGS,
            deadband: 0.0,
            exceeded_limit: -1e300,
        },
        P::SignedOutOfRange {
            exceeding_value: i32::MIN,
            status_flags: FLAGS,
            deadband: u64::MAX,
            exceeded_limit: i32::MAX,
        },
        P::SignedOutOfRange {
            exceeding_value: -1,
            status_flags: ALL_FLAGS,
            deadband: 2,
            exceeded_limit: -3,
        },
        P::UnsignedOutOfRange {
            exceeding_value: u64::MAX,
            status_flags: FLAGS,
            deadband: 0,
            exceeded_limit: u64::MAX,
        },
        P::ChangeOfCharacterstring {
            changed_value: "changed".into(),
            status_flags: FLAGS,
            alarm_value: "alarm".into(),
        },
        P::ChangeOfCharacterstring {
            changed_value: String::new(),
            status_flags: ALL_FLAGS,
            alarm_value: "\u{e7}\u{20ac}".into(),
        },
        P::ChangeOfStatusFlags {
            present_value: None,
            referenced_flags: FLAGS,
        },
        P::ChangeOfStatusFlags {
            present_value: Some(REAL_50.to_vec()),
            referenced_flags: ALL_FLAGS,
        },
        P::ChangeOfReliability {
            reliability: Reliability::OVER_RANGE,
            status_flags: FLAGS,
            property_values: vec![],
        },
        P::ChangeOfReliability {
            reliability: Reliability::from_raw(u32::MAX),
            status_flags: ALL_FLAGS,
            property_values: vec![0x09, 0x55, 0x2e, 0x44, 0x3f, 0x80, 0x00, 0x00, 0x2f],
        },
        P::ChangeOfDiscreteValue {
            new_value: vec![0x91, 0x01],
            status_flags: FLAGS,
        },
    ];
    for state in [
        BACnetPropertyStates::BooleanValue(true),
        BACnetPropertyStates::BinaryValue(1),
        BACnetPropertyStates::State(3),
        BACnetPropertyStates::UnsignedValue(u32::MAX),
        BACnetPropertyStates::ZoneOccupancyState(2),
    ] {
        values.push(P::ChangeOfState {
            new_state: state,
            status_flags: FLAGS,
        });
    }
    for authentication_factor in [None, Some(vec![0x09, 0x01, 0x19, 0x02, 0x2a, 0xab, 0xcd])] {
        values.push(P::AccessEvent {
            access_event: AccessEvent::TRACE,
            status_flags: FLAGS,
            access_event_tag: 10,
            access_event_time: date_time(),
            access_credential: BACnetDeviceObjectReference {
                device_identifier: authentication_factor
                    .as_ref()
                    .map(|_| object(ObjectType::DEVICE, 2)),
                object_identifier: object(ObjectType::ACCESS_CREDENTIAL, 1),
            },
            authentication_factor,
        });
    }
    for present in [false, true] {
        values.push(P::ChangeOfTimer {
            new_state: TimerState::RUNNING,
            status_flags: FLAGS,
            update_time: date_time(),
            last_state_change: present.then_some(TimerTransition::RUNNING_TO_IDLE),
            initial_timeout: present.then_some(u32::MAX),
            expiration_time: present.then_some(date_time()),
        });
    }
    for (tag, constructed, data) in [
        (7, true, vec![]),
        (12, true, vec![0x91, 0x01]),
        (23, true, REAL_50.to_vec()),
        (254, true, vec![]),
        (7, false, vec![0x01, 0x02]),
        (200, false, vec![]),
    ] {
        values.push(P::Opaque(OpaqueChoice {
            tag,
            constructed,
            data,
        }));
    }
    values
}

/// The inputs made from one encoding: itself, each prefix, each octet
/// deleted, an octet inserted before each, and each octet replaced by every
/// other value.
fn mutations(encoded: &[u8]) -> Vec<Vec<u8>> {
    let mut inputs = vec![encoded.to_vec()];
    for len in 0..encoded.len() {
        inputs.push(encoded[..len].to_vec());
    }
    for at in 0..encoded.len() {
        let mut deleted = encoded.to_vec();
        deleted.remove(at);
        inputs.push(deleted);
    }
    for at in 0..=encoded.len() {
        for octet in [0x00, 0x0f, 0x3e, 0xff] {
            let mut inserted = encoded.to_vec();
            inserted.insert(at, octet);
            inputs.push(inserted);
        }
    }
    for at in 0..encoded.len() {
        for octet in 0..=u8::MAX {
            if octet != encoded[at] {
                let mut changed = encoded.to_vec();
                changed[at] = octet;
                inputs.push(changed);
            }
        }
    }
    inputs
}

fn same(
    split: &Result<NotificationParameters, Error>,
    before: &Result<NotificationParameters, Error>,
) -> bool {
    match (split, before) {
        // A float field changed into a NaN isn't equal to itself.
        (Ok(split), Ok(before)) => split == before || format!("{split:?}") == format!("{before:?}"),
        (Err(split), Err(before)) => format!("{split:?}") == format!("{before:?}"),
        _ => false,
    }
}

#[test]
fn the_split_decodes_every_input_as_before() {
    let mut compared = 0usize;
    let mut accepted = 0usize;
    for params in corpus() {
        let mut encoded = BytesMut::new();
        encode_notification_parameters(&params, &mut encoded).unwrap();
        assert_eq!(decode_bounded(&encoded, 0, encoded.len()).unwrap(), params);
        for input in mutations(&encoded) {
            let len = input.len();
            let split = decode_bounded(&input, 0, len);
            assert!(
                same(&split, &base::decode_bounded(&input, 0, len)),
                "{input:02X?}"
            );
            accepted += usize::from(split.is_ok());
            let public = decode_notification_parameters(&input, 0);
            assert!(
                same(&public, &base::decode_notification_parameters(&input, 0)),
                "{input:02X?}"
            );
            // The same choice inside a request's `[12]` eventValues.
            let mut framed = vec![0x0e_u8 | 12 << 4];
            framed.extend_from_slice(&input);
            framed.push(0x0f | 12 << 4);
            let end = framed.len() - 1;
            assert!(
                same(
                    &decode_bounded(&framed, 1, end),
                    &base::decode_bounded(&framed, 1, end)
                ),
                "{framed:02X?}"
            );
            assert!(
                same(
                    &decode_notification_parameters(&framed, 1),
                    &base::decode_notification_parameters(&framed, 1)
                ),
                "{framed:02X?}"
            );
            compared += 1;
        }
    }
    // Each input went through both entry points, bare and framed: 175,090
    // inputs when this was written, 69,748 of them decoding.
    println!("{compared} inputs, {accepted} decoded");
    assert!(compared > 150_000, "{compared} inputs");
    assert!(accepted > 50_000, "{accepted} of {compared} decode");
}

/// Where the top-level field with context tag `number` starts in `encoded`,
/// one choice's encoding.
fn field_offset(encoded: &[u8], number: u8) -> usize {
    let (_, mut at) = tags::decode_tag(encoded, 0).unwrap();
    loop {
        let (tag, content) = tags::decode_tag(encoded, at).unwrap();
        if tag.number == number && !tag.is_closing {
            return at;
        }
        at = if tag.is_opening {
            tags::extract_context_value(encoded, content, tag.number)
                .unwrap()
                .1
        } else {
            content + tag.length as usize
        };
    }
}

/// The status flags field of a choice that has one: the name its decoder
/// gives the field in errors, and its context tag.
fn status_flags_field(params: &NotificationParameters) -> Option<(&'static str, u8)> {
    use NotificationParameters as P;
    Some(match params {
        P::ChangeOfBitstring { .. } => ("ChangeOfBitstring status-flags", 1),
        P::ChangeOfState { .. } => ("ChangeOfState status-flags", 1),
        P::ChangeOfValue { .. } => ("ChangeOfValue status-flags", 1),
        P::CommandFailure { .. } => ("CommandFailure status-flags", 1),
        P::FloatingLimit { .. } => ("FloatingLimit status-flags", 1),
        P::OutOfRange { .. } => ("OutOfRange status-flags", 1),
        P::ChangeOfLifeSafety { .. } => ("ChangeOfLifeSafety status-flags", 2),
        P::UnsignedRange { .. } => ("UnsignedRange status-flags", 1),
        P::AccessEvent { .. } => ("AccessEvent status-flags", 1),
        P::DoubleOutOfRange { .. } => ("DoubleOutOfRange status-flags", 1),
        P::SignedOutOfRange { .. } => ("SignedOutOfRange status-flags", 1),
        P::UnsignedOutOfRange { .. } => ("UnsignedOutOfRange status-flags", 1),
        P::ChangeOfCharacterstring { .. } => ("ChangeOfCharacterstring status-flags", 1),
        P::ChangeOfStatusFlags { .. } => ("ChangeOfStatusFlags referenced-flags", 1),
        P::ChangeOfReliability { .. } => ("ChangeOfReliability status-flags", 1),
        P::ChangeOfDiscreteValue { .. } => ("ChangeOfDiscreteValue status-flags", 1),
        P::ChangeOfTimer { .. } => ("ChangeOfTimer status-flags", 1),
        P::ComplexEventType { .. } | P::Extended { .. } | P::BufferReady { .. } | P::Opaque(_) => {
            return None;
        }
    })
}

/// Each choice's decoder reports a fault in its own status flags with its
/// own name, at the field's offset: a choice handed to another choice's
/// decoder would name that one, or decode.
#[test]
fn each_choice_names_its_own_status_flags_fault() {
    let mut checked = BTreeSet::new();
    for params in corpus() {
        let Some((context, number)) = status_flags_field(&params) else {
            continue;
        };
        let mut encoded = BytesMut::new();
        encode_notification_parameters(&params, &mut encoded).unwrap();
        let at = field_offset(&encoded, number);
        // The flags' last octet: a padding bit set.
        let (_, content) = tags::decode_tag(&encoded, at).unwrap();
        encoded[content + 1] |= 0x01;
        let Err(Error::Decoding(error)) = decode_notification_parameters(&encoded, 0) else {
            panic!("{params:?}: decoded");
        };
        assert_eq!(
            (error.kind, error.offset, error.context, error.label),
            (
                DecodingKind::InvalidEncoding,
                at,
                context,
                "must have zero padding"
            ),
            "{params:?}"
        );
        checked.insert(context);
    }
    // Every choice with status flags, the corpus having each.
    assert_eq!(checked.len(), 17, "{checked:?}");
}

/// The choices without status flags fail in a field of their own: Extended
/// in its vendor identifier, Buffer Ready in its notification counts.
#[test]
fn choices_without_status_flags_name_their_own_fields() {
    // Extended, vendor-id 0x10000.
    let extended = [0x9e, 0x0b, 0x01, 0x00, 0x00, 0x19, 0x00, 0x2e, 0x2f, 0x9f];
    let Err(Error::Decoding(error)) = decode_notification_parameters(&extended, 0) else {
        panic!("decoded");
    };
    assert_eq!(
        (error.kind, error.offset, error.context, error.label),
        (DecodingKind::OutOfRange, 1, "Extended vendor-id", "exceeds u16")
    );

    let mut encoded = BytesMut::new();
    let params = corpus()
        .into_iter()
        .find(|params| matches!(params, NotificationParameters::BufferReady { .. }))
        .unwrap();
    encode_notification_parameters(&params, &mut encoded).unwrap();
    // current-notification [2] retagged [3]: a later member, so [2] is
    // the one left out.
    let at = field_offset(&encoded, 2);
    encoded[at] = (encoded[at] & 0x0f) | 3 << 4;
    let Err(Error::Decoding(error)) = decode_notification_parameters(&encoded, 0) else {
        panic!("decoded");
    };
    assert_eq!(
        (error.kind, error.offset, error.context, error.label, error.values),
        (
            DecodingKind::Missing,
            at,
            "BufferReady current-notification",
            "expected context tag {}",
            [2, 0]
        )
    );
}

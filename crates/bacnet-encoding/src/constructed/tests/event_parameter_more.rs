//! `BACnetEventParameter` alternatives modelled in #2090: a round trip each,
//! byte fixtures built by hand from the Clause 21 production, the list cap,
//! and the tags `Opaque` may no longer carry.

use super::*;
use bacnet_types::enums::{AccessEvent, LifeSafetyState, TimerState};
use bacnet_types::primitives::StatusFlags;

/// One value of every alternative #2090 modelled.
pub(super) fn every_new_algorithm() -> Vec<BACnetEventParameter> {
    use BACnetEventParameter as EP;
    vec![
        EP::CommandFailure {
            time_delay: 4,
            feedback_property_reference: dopr_ai(3, 85),
        },
        EP::ChangeOfLifeSafety {
            time_delay: 10,
            list_of_life_safety_alarm_values: vec![LifeSafetyState::from_raw(2)],
            list_of_alarm_values: vec![LifeSafetyState::from_raw(1), LifeSafetyState::from_raw(3)],
            mode_property_reference: dopr_ai(1, 160),
        },
        EP::BufferReady {
            notification_threshold: 50,
            previous_notification_count: 4_000_000_000,
        },
        EP::UnsignedRange {
            time_delay: 30,
            low_limit: 10,
            high_limit: u64::MAX,
        },
        EP::AccessEvent {
            list_of_access_events: vec![AccessEvent::from_raw(1), AccessEvent::from_raw(129)],
            access_event_time_reference: dopr_ai(7, 4),
        },
        EP::DoubleOutOfRange {
            time_delay: 0,
            low_limit: -1.5,
            high_limit: 99.25,
            deadband: 0.5,
        },
        EP::SignedOutOfRange {
            time_delay: 30,
            low_limit: -5,
            high_limit: i32::MAX,
            deadband: 2,
        },
        EP::UnsignedOutOfRange {
            time_delay: 5,
            low_limit: 10,
            high_limit: 300,
            deadband: 1,
        },
        EP::ChangeOfCharacterstring {
            time_delay: 2,
            list_of_alarm_values: vec!["FIRE".into(), String::new(), "smoke ☁".into()],
        },
        EP::ChangeOfStatusFlags {
            time_delay: 1,
            selected_flags: StatusFlags::IN_ALARM | StatusFlags::OUT_OF_SERVICE,
        },
        EP::None,
        EP::ChangeOfDiscreteValue { time_delay: 9 },
        EP::ChangeOfTimer {
            time_delay: 3,
            alarm_values: vec![TimerState::from_raw(1), TimerState::from_raw(2)],
            update_time_reference: dopr_ai(2, 85),
        },
    ]
}

fn encoded(value: &BACnetEventParameter) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_event_parameter(&mut buf, value).unwrap();
    buf.to_vec()
}

#[test]
fn every_new_algorithm_round_trips() {
    for value in every_new_algorithm() {
        let octets = encoded(&value);
        let (decoded, end) = decode_event_parameter(&octets, 0).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(end, octets.len(), "{value:?}");
        // Empty lists are values too.
        if let BACnetEventParameter::ChangeOfTimer {
            time_delay,
            update_time_reference,
            ..
        } = value
        {
            let empty = BACnetEventParameter::ChangeOfTimer {
                time_delay,
                alarm_values: Vec::new(),
                update_time_reference,
            };
            assert_eq!(
                decode_event_parameter(&encoded(&empty), 0).unwrap().0,
                empty
            );
        }
    }
}

/// Fixtures hand-built from the production, one for every alternative
/// #2090 modelled: each alternative's opening tag, its members in order,
/// its closing tag.
#[test]
fn new_algorithms_match_hand_built_fixtures() {
    let double = |value: f64| value.to_be_bytes();
    let double_out_of_range = [
        &[0xEE, 0x09, 0x00, 0x1D, 0x08][..],
        &double(-1.5),
        &[0x2D, 0x08],
        &double(99.25),
        &[0x3D, 0x08],
        &double(0.5),
        &[0xEF],
    ]
    .concat();
    let cases: [(BACnetEventParameter, &[u8]); 13] = [
        (
            BACnetEventParameter::UnsignedRange {
                time_delay: 30,
                low_limit: 10,
                high_limit: 300,
            },
            // [11] { [0] 30, [1] 10, [2] 300 }
            &[0xBE, 0x09, 0x1E, 0x19, 0x0A, 0x2A, 0x01, 0x2C, 0xBF],
        ),
        (
            BACnetEventParameter::ChangeOfStatusFlags {
                time_delay: 5,
                selected_flags: StatusFlags::IN_ALARM | StatusFlags::FAULT,
            },
            // [18] { [0] 5, [1] four bits 1100 }: tag 18 takes the extended
            // tag-number octet.
            &[0xFE, 0x12, 0x09, 0x05, 0x1A, 0x04, 0xC0, 0xFF, 0x12],
        ),
        (
            BACnetEventParameter::ChangeOfLifeSafety {
                time_delay: 10,
                list_of_life_safety_alarm_values: vec![LifeSafetyState::from_raw(2)],
                list_of_alarm_values: vec![
                    LifeSafetyState::from_raw(1),
                    LifeSafetyState::from_raw(3),
                ],
                mode_property_reference: BACnetDeviceObjectPropertyReference {
                    object_identifier: ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1)
                        .unwrap(),
                    property_identifier: 160,
                    property_array_index: None,
                    device_identifier: None,
                },
            },
            // [8] { [0] 10, [1] { ENUM 2 }, [2] { ENUM 1, ENUM 3 },
            //       [3] { [0] life-safety-zone 1, [1] 160 } }
            &[
                0x8E, 0x09, 0x0A, 0x1E, 0x91, 0x02, 0x1F, 0x2E, 0x91, 0x01, 0x91, 0x03, 0x2F, 0x3E,
                0x0C, 0x05, 0x80, 0x00, 0x01, 0x19, 0xA0, 0x3F, 0x8F,
            ],
        ),
        (
            BACnetEventParameter::ChangeOfCharacterstring {
                time_delay: 0,
                list_of_alarm_values: vec!["hi".into()],
            },
            // [17] { [0] 0, [1] { CharacterString UTF-8 "hi" } }
            &[
                0xFE, 0x11, 0x09, 0x00, 0x1E, 0x73, 0x00, b'h', b'i', 0x1F, 0xFF, 0x11,
            ],
        ),
        (
            BACnetEventParameter::SignedOutOfRange {
                time_delay: 30,
                low_limit: -5,
                high_limit: 40,
                deadband: 2,
            },
            // [15] { [0] 30, [1] -5, [2] 40, [3] 2 }
            &[
                0xFE, 0x0F, 0x09, 0x1E, 0x19, 0xFB, 0x29, 0x28, 0x39, 0x02, 0xFF, 0x0F,
            ],
        ),
        (
            BACnetEventParameter::CommandFailure {
                time_delay: 4,
                feedback_property_reference: dopr_ai(3, 85),
            },
            // [3] { [0] 4, [1] { [0] analog-input 3, [1] 85 } }
            &[
                0x3E, 0x09, 0x04, 0x1E, 0x0C, 0x00, 0x00, 0x00, 0x03, 0x19, 0x55, 0x1F, 0x3F,
            ],
        ),
        (
            BACnetEventParameter::BufferReady {
                notification_threshold: 50,
                previous_notification_count: 4_000_000_000,
            },
            // [10] { [0] 50, [1] 4000000000 }
            &[0xAE, 0x09, 0x32, 0x1C, 0xEE, 0x6B, 0x28, 0x00, 0xAF],
        ),
        (
            BACnetEventParameter::AccessEvent {
                list_of_access_events: vec![AccessEvent::from_raw(1), AccessEvent::from_raw(129)],
                access_event_time_reference: dopr_ai(7, 4),
            },
            // [13] { [0] { ENUM 1, ENUM 129 }, [1] { [0] analog-input 7, [1] 4 } }
            &[
                0xDE, 0x0E, 0x91, 0x01, 0x91, 0x81, 0x0F, 0x1E, 0x0C, 0x00, 0x00, 0x00, 0x07, 0x19,
                0x04, 0x1F, 0xDF,
            ],
        ),
        (
            BACnetEventParameter::DoubleOutOfRange {
                time_delay: 0,
                low_limit: -1.5,
                high_limit: 99.25,
                deadband: 0.5,
            },
            // [14] { [0] 0, [1] -1.5, [2] 99.25, [3] 0.5 }, each a Double; tag 14
            // still fits the tag octet
            &double_out_of_range,
        ),
        (
            BACnetEventParameter::UnsignedOutOfRange {
                time_delay: 5,
                low_limit: 10,
                high_limit: 300,
                deadband: 1,
            },
            // [16] { [0] 5, [1] 10, [2] 300, [3] 1 }
            &[
                0xFE, 0x10, 0x09, 0x05, 0x19, 0x0A, 0x2A, 0x01, 0x2C, 0x39, 0x01, 0xFF, 0x10,
            ],
        ),
        // none [20]: a primitive NULL with no contents.
        (BACnetEventParameter::None, &[0xF8, 0x14]),
        (
            BACnetEventParameter::ChangeOfDiscreteValue { time_delay: 9 },
            // [21] { [0] 9 }
            &[0xFE, 0x15, 0x09, 0x09, 0xFF, 0x15],
        ),
        (
            BACnetEventParameter::ChangeOfTimer {
                time_delay: 3,
                alarm_values: vec![TimerState::from_raw(1), TimerState::from_raw(2)],
                update_time_reference: dopr_ai(2, 85),
            },
            // [22] { [0] 3, [1] { ENUM 1, ENUM 2 }, [2] { [0] analog-input 2, [1] 85 } }
            &[
                0xFE, 0x16, 0x09, 0x03, 0x1E, 0x91, 0x01, 0x91, 0x02, 0x1F, 0x2E, 0x0C, 0x00, 0x00,
                0x00, 0x02, 0x19, 0x55, 0x2F, 0xFF, 0x16,
            ],
        ),
    ];
    for (value, wire) in cases {
        assert_eq!(encoded(&value), wire, "{value:?}");
        assert_eq!(
            decode_event_parameter(wire, 0).unwrap(),
            (value, wire.len())
        );
    }
}

/// A list longer than the cap is refused before it is gathered.
#[test]
fn a_list_past_the_cap_is_refused() {
    let mut wire = vec![0xDE, 0x0E]; // access-event [13] { list [0] {
    for _ in 0..=10_000 {
        wire.extend_from_slice(&[0x91, 0x01]);
    }
    wire.push(0x0F);
    wire.extend_from_slice(&[0x1E, 0x0C, 0x00, 0x00, 0x00, 0x07, 0x19, 0x04, 0x1F, 0xDF]);
    let error = decode_event_parameter(&wire, 0).unwrap_err();
    assert!(error.to_string().contains("list exceeds limit"), "{error}");
}

/// The modelled tags, `none` among them, no longer travel as `Opaque`: the
/// encoder refuses them, and a body that isn't the production is an error.
#[test]
fn modelled_tags_are_not_opaque() {
    for tag in [3u8, 8, 10, 11, 13, 14, 15, 16, 17, 18, 20, 21, 22] {
        let opaque = BACnetEventParameter::Opaque {
            tag,
            data: vec![0x09, 0x01],
        };
        assert!(
            encode_event_parameter(&mut BytesMut::new(), &opaque).is_err(),
            "tag {tag}"
        );
    }
    // access-event [13] whose list holds NULLs, not enumerations.
    let wire = [
        0xDE, 0x0E, 0x00, 0x0F, 0x1E, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x1F, 0xDF,
    ];
    assert!(decode_event_parameter(&wire, 0).is_err());
    // Status flags of another width.
    let wire = [0xFE, 0x12, 0x09, 0x05, 0x1A, 0x03, 0xC0, 0xFF, 0x12];
    assert!(decode_event_parameter(&wire, 0).is_err());
    // A member out of order: buffer-ready's [1] before its [0].
    let wire = [0xAE, 0x19, 0x01, 0x09, 0x02, 0xAF];
    assert!(decode_event_parameter(&wire, 0).is_err());
}

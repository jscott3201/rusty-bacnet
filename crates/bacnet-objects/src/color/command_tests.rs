//! Color_Command of the Color and Color Temperature objects as a typed
//! BACnetColorCommand (#1386): what a write may carry and what each object
//! takes (Addendum 135-2020ca).

use super::*;
use crate::traits::BACnetObject;
use bacnet_types::constructed::BACnetColorCommand;
use bacnet_types::enums::{ColorOperation as Op, ErrorClass, ErrorCode, PropertyIdentifier};

/// D65 white, where a new Color object starts.
const D65: BACnetXyColor = BACnetXyColor::new(0.3127, 0.3290);

const CC: PropertyIdentifier = PropertyIdentifier::COLOR_COMMAND;

fn assert_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, expected.to_raw() as u32, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn command(operation: Op) -> BACnetColorCommand {
    BACnetColorCommand::new(operation)
}

fn octets(value: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(value.to_vec())
}

/// FADE_TO_COLOR to (0.5, 0.25) (0x3F000000, 0x3E800000) over 2,000 ms.
const FADE_TO_COLOR: [u8; 17] = [
    0x09, 0x01, 0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F, 0x3A, 0x07,
    0xD0,
];

/// FADE_TO_CCT to 2,700 K (0x0A8C) over 100 ms.
const FADE_TO_CCT: [u8; 7] = [0x09, 0x02, 0x2A, 0x0A, 0x8C, 0x39, 0x64];

fn fade_to_color() -> BACnetColorCommand {
    BACnetColorCommand {
        target_color: Some(BACnetXyColor::new(0.5, 0.25)),
        fade_time: Some(2_000),
        ..command(Op::FADE_TO_COLOR)
    }
}

#[test]
fn color_commands_start_at_none_and_serve_what_was_written() {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    assert_eq!(color.color_command(), command(Op::NONE));
    assert_eq!(
        color.read_property(CC, None).unwrap(),
        octets(&[0x09, 0x00])
    );
    color
        .write_property(CC, None, octets(&FADE_TO_COLOR), None)
        .unwrap();
    assert_eq!(
        color.read_property(CC, None).unwrap(),
        octets(&FADE_TO_COLOR)
    );
    assert_eq!(color.color_command(), fade_to_color());
    // The setter takes the same values; STOP is `09 06`.
    color.set_color_command(command(Op::STOP)).unwrap();
    assert_eq!(
        color.read_property(CC, None).unwrap(),
        octets(&[0x09, 0x06])
    );
    // With no clock bound, STOP comes at the instant the fade started, so
    // the colour stays where the fade began.
    assert_eq!(
        color
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        xy_value(D65)
    );

    let mut temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    assert_eq!(temperature.color_command(), command(Op::NONE));
    assert_eq!(
        temperature.read_property(CC, None).unwrap(),
        octets(&[0x09, 0x00])
    );
    temperature
        .write_property(CC, None, octets(&FADE_TO_CCT), None)
        .unwrap();
    assert_eq!(
        temperature.read_property(CC, None).unwrap(),
        octets(&FADE_TO_CCT)
    );
    assert_eq!(
        temperature.color_command(),
        BACnetColorCommand {
            target_color_temperature: Some(2_700),
            fade_time: Some(100),
            ..command(Op::FADE_TO_CCT)
        }
    );
    // The fade is under way: Present_Value holds its target, and
    // In_Progress is FADE_ACTIVE.
    assert_eq!(
        temperature
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(2_700)
    );
    assert_eq!(
        temperature
            .read_property(PropertyIdentifier::IN_PROGRESS, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
}

#[test]
fn color_commands_refuse_other_datatypes_and_broken_encodings() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(ColorObject::new(1, "CLR-1").unwrap()),
        Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()),
    ];
    for mut object in objects {
        for value in [
            // The octet-string form this property once took.
            PropertyValue::OctetString(FADE_TO_CCT.to_vec()),
            PropertyValue::Unsigned(1),
            PropertyValue::List(vec![octets(&FADE_TO_CCT[..2]), octets(&FADE_TO_CCT[2..])]),
            // Application-tagged octets, and a target with no operation.
            octets(&[0x21, 0x01]),
            octets(&FADE_TO_CCT[2..]),
        ] {
            assert_error(
                object.write_property(CC, None, value, None),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        // Nothing, an operation cut short, a target colour that never closes,
        // an octet after the command, and one after a target colour
        // temperature too wide for 32 bits: the broken encoding outranks the
        // oversized field.
        let trailing = [&FADE_TO_CCT[..], &[0x00]].concat();
        let wide_then_trailing = [0x09, 0x02, 0x2D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
        for broken in [
            &[][..],
            &[0x09][..],
            &FADE_TO_COLOR[..13],
            &trailing[..],
            &wide_then_trailing[..],
        ] {
            assert_error(
                object.write_property(CC, None, octets(broken), None),
                ErrorCode::INVALID_DATA_ENCODING,
            );
        }
        // A field too wide for its type alone is out of range.
        assert_error(
            object.write_property(CC, None, octets(&wide_then_trailing[..9]), None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            object.read_property(CC, None).unwrap(),
            octets(&[0x09, 0x00])
        );
    }
}

#[test]
fn color_object_takes_fade_to_color_and_stop_only() {
    let target = |x, y| BACnetColorCommand {
        target_color: Some(BACnetXyColor::new(x, y)),
        ..command(Op::FADE_TO_COLOR)
    };
    let refused = [
        // NONE, the Color Temperature operations, and past STOP.
        command(Op::NONE),
        BACnetColorCommand {
            target_color_temperature: Some(2_700),
            ..command(Op::FADE_TO_CCT)
        },
        BACnetColorCommand {
            target_color_temperature: Some(2_700),
            ..command(Op::RAMP_TO_CCT)
        },
        command(Op::STEP_UP_CCT),
        command(Op::STEP_DOWN_CCT),
        command(Op::from_raw(7)),
        command(Op::from_raw(u32::MAX)),
        // FADE_TO_COLOR needs a target colour within 0.0 to 1.0.
        command(Op::FADE_TO_COLOR),
        target(-0.1, 0.5),
        target(0.5, 1.5),
        target(f32::NAN, 0.5),
        target(0.5, f32::INFINITY),
        // A fade time outside 100 to 86,400,000 ms.
        BACnetColorCommand {
            fade_time: Some(99),
            ..target(0.5, 0.5)
        },
        BACnetColorCommand {
            fade_time: Some(86_400_001),
            ..target(0.5, 0.5)
        },
    ];
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    for value in refused {
        assert_error(
            color.set_color_command(value),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(color.color_command(), command(Op::NONE), "{value:?}");
    }
    let accepted = [
        // The range ends.
        BACnetColorCommand {
            fade_time: Some(100),
            ..target(0.0, 0.0)
        },
        BACnetColorCommand {
            fade_time: Some(86_400_000),
            ..target(1.0, 1.0)
        },
        target(0.3127, 0.3290),
        command(Op::STOP),
        // A field the operation doesn't use isn't checked.
        BACnetColorCommand {
            target_color_temperature: Some(0),
            ramp_rate: Some(0),
            step_increment: Some(u32::MAX),
            ..target(0.5, 0.5)
        },
        BACnetColorCommand {
            target_color: Some(BACnetXyColor::new(9.0, -9.0)),
            fade_time: Some(1),
            ..command(Op::STOP)
        },
    ];
    for value in accepted {
        color.set_color_command(value).unwrap();
        assert_eq!(color.color_command(), value);
    }
}

#[test]
fn color_temperature_object_takes_the_cct_operations_and_stop() {
    let target = |operation, kelvin| BACnetColorCommand {
        target_color_temperature: Some(kelvin),
        ..command(operation)
    };
    let refused = [
        // NONE, FADE_TO_COLOR even with a valid target, and past STOP.
        command(Op::NONE),
        BACnetColorCommand {
            target_color: Some(BACnetXyColor::new(0.5, 0.5)),
            ..command(Op::FADE_TO_COLOR)
        },
        command(Op::from_raw(7)),
        command(Op::from_raw(255)),
        // FADE_TO_CCT and RAMP_TO_CCT need a target within 1000 to 30000 K.
        command(Op::FADE_TO_CCT),
        command(Op::RAMP_TO_CCT),
        target(Op::FADE_TO_CCT, 999),
        target(Op::RAMP_TO_CCT, 30_001),
        target(Op::FADE_TO_CCT, 0),
        // A fade time outside 100 to 86,400,000 ms; a ramp rate or step
        // increment outside 1 to 30000.
        BACnetColorCommand {
            fade_time: Some(99),
            ..target(Op::FADE_TO_CCT, 2_700)
        },
        BACnetColorCommand {
            fade_time: Some(86_400_001),
            ..target(Op::FADE_TO_CCT, 2_700)
        },
        BACnetColorCommand {
            ramp_rate: Some(0),
            ..target(Op::RAMP_TO_CCT, 2_700)
        },
        BACnetColorCommand {
            ramp_rate: Some(30_001),
            ..target(Op::RAMP_TO_CCT, 2_700)
        },
        BACnetColorCommand {
            step_increment: Some(0),
            ..command(Op::STEP_UP_CCT)
        },
        BACnetColorCommand {
            step_increment: Some(30_001),
            ..command(Op::STEP_DOWN_CCT)
        },
    ];
    let mut temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    for value in refused {
        assert_error(
            temperature.set_color_command(value),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(temperature.color_command(), command(Op::NONE), "{value:?}");
    }
    let accepted = [
        // The range ends, and the bare operations.
        BACnetColorCommand {
            fade_time: Some(100),
            ..target(Op::FADE_TO_CCT, 1_000)
        },
        BACnetColorCommand {
            fade_time: Some(86_400_000),
            ..target(Op::FADE_TO_CCT, 30_000)
        },
        BACnetColorCommand {
            ramp_rate: Some(1),
            ..target(Op::RAMP_TO_CCT, 30_000)
        },
        BACnetColorCommand {
            ramp_rate: Some(30_000),
            ..target(Op::RAMP_TO_CCT, 1_000)
        },
        BACnetColorCommand {
            step_increment: Some(1),
            ..command(Op::STEP_UP_CCT)
        },
        BACnetColorCommand {
            step_increment: Some(30_000),
            ..command(Op::STEP_DOWN_CCT)
        },
        command(Op::STEP_UP_CCT),
        command(Op::STOP),
        // A target outside Min_Pres_Value and Max_Pres_Value is taken: the
        // object clamps it when carrying the command out.
        target(Op::FADE_TO_CCT, 1_000),
        // A field the operation doesn't use isn't checked.
        BACnetColorCommand {
            target_color: Some(BACnetXyColor::new(-1.0, 2.0)),
            ramp_rate: Some(0),
            step_increment: Some(0),
            ..target(Op::FADE_TO_CCT, 2_700)
        },
        BACnetColorCommand {
            target_color_temperature: Some(0),
            fade_time: Some(0),
            ..command(Op::STEP_DOWN_CCT)
        },
    ];
    temperature.set_min_max(2_000, 6_500).unwrap();
    for value in accepted {
        temperature.set_color_command(value).unwrap();
        assert_eq!(temperature.color_command(), value);
    }
}

#[test]
fn color_objects_no_longer_answer_to_508_to_511() {
    // 508 to 510 name Network Port properties (#887); the colour properties
    // moved past 4194303.
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(ColorObject::new(1, "CLR-1").unwrap()),
        Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()),
    ];
    for object in objects {
        for raw in [508, 509, 510, 511] {
            let property = PropertyIdentifier::from_raw(raw);
            assert!(!object.property_list().contains(&property));
            assert_error(
                object.read_property(property, None),
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
        let list = object.property_list();
        assert!(list.contains(&PropertyIdentifier::from_raw(4_194_334)));
        assert!(object
            .read_property(PropertyIdentifier::from_raw(4_194_334), None)
            .is_ok());
    }
    let color = ColorObject::new(1, "CLR-1").unwrap();
    assert_eq!(
        color
            .read_property(PropertyIdentifier::from_raw(4_194_330), None)
            .unwrap(),
        xy_value(D65)
    );
    let temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    assert_eq!(
        temperature
            .read_property(PropertyIdentifier::from_raw(4_194_331), None)
            .unwrap(),
        PropertyValue::Unsigned(4000)
    );
}

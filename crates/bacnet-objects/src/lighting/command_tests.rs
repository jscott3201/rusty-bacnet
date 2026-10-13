//! Lighting Output's Lighting_Command as a typed BACnetLightingCommand
//! (#1263): what a write may carry and what each operation takes (Clause
//! 12.54, Table 12-67).

use super::*;
use crate::channel::{coerce_channel_value, MemberDatatype};
use bacnet_types::enums::{ErrorClass, ErrorCode, LightingOperation as Op};

const LC: PropertyIdentifier = PropertyIdentifier::LIGHTING_COMMAND;

fn assert_error<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
            assert_eq!(code, expected.to_raw() as u32, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn command(operation: Op) -> BACnetLightingCommand {
    BACnetLightingCommand::new(operation)
}

fn octets(value: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(value.to_vec())
}

/// FADE_TO 50.0 % (0x42480000) at priority 8.
const FADE: [u8; 9] = [0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x59, 0x08];

#[test]
fn lighting_output_lighting_command_starts_at_none_and_serves_what_was_written() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    assert_eq!(lo.lighting_command(), command(Op::NONE));
    assert_eq!(lo.read_property(LC, None).unwrap(), octets(&[0x09, 0x00]));
    lo.write_property(LC, None, octets(&FADE), None).unwrap();
    assert_eq!(lo.read_property(LC, None).unwrap(), octets(&FADE));
    assert_eq!(
        lo.lighting_command(),
        BACnetLightingCommand {
            target_level: Some(50.0),
            priority: Some(8),
            ..command(Op::FADE_TO)
        }
    );
    // The setter takes the same values; STOP at priority 3 is `09 0A 59 03`.
    let stop = BACnetLightingCommand {
        priority: Some(3),
        ..command(Op::STOP)
    };
    lo.set_lighting_command(stop).unwrap();
    assert_eq!(lo.lighting_command(), stop);
    assert_eq!(
        lo.read_property(LC, None).unwrap(),
        octets(&[0x09, 0x0A, 0x59, 0x03])
    );
    // The FADE_TO was carried out at priority 8 (#1384). With no clock bound
    // its fade waits on logical time, so it is still running, and the STOP
    // at priority 3 had nothing there to stop.
    assert_eq!(
        lo.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(50.0)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap(),
        PropertyValue::Real(50.0)
    );
    assert_eq!(
        lo.read_property(PropertyIdentifier::IN_PROGRESS, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
}

#[test]
fn lighting_output_lighting_command_refuses_other_datatypes() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    for value in [
        // The octet-string form this property once took.
        PropertyValue::OctetString(FADE.to_vec()),
        PropertyValue::Real(50.0),
        PropertyValue::List(vec![octets(&FADE[..2]), octets(&FADE[2..])]),
        // Application-tagged octets, and a target level with no operation.
        octets(&[0x44, 0x42, 0x48, 0x00, 0x00]),
        octets(&FADE[2..7]),
    ] {
        assert_error(
            lo.write_property(LC, None, value, None),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    // Nothing, an operation cut short, a priority cut short, an octet after
    // the command, and one after a priority too wide for an Unsigned8: the
    // broken encoding outranks the oversized field.
    let trailing = [&FADE[..], &[0x00]].concat();
    let wide_then_trailing = [0x09, 0x07, 0x5A, 0x01, 0x00, 0xFF];
    for broken in [
        &[][..],
        &[0x09][..],
        &FADE[..8],
        &trailing[..],
        &wide_then_trailing[..],
    ] {
        assert_error(
            lo.write_property(LC, None, octets(broken), None),
            ErrorCode::INVALID_DATA_ENCODING,
        );
    }
    assert_eq!(lo.lighting_command(), command(Op::NONE));
}

#[test]
fn lighting_output_lighting_command_checks_what_each_operation_takes() {
    let level = |operation, target_level| BACnetLightingCommand {
        target_level,
        ..command(operation)
    };
    let refused = [
        command(Op::NONE),
        command(Op::from_raw(11)),
        command(Op::from_raw(255)),
        command(Op::from_raw(65_536)),
        // FADE_TO and RAMP_TO need a target level within 0.0 to 100.0.
        level(Op::FADE_TO, None),
        level(Op::RAMP_TO, None),
        level(Op::FADE_TO, Some(-0.5)),
        level(Op::RAMP_TO, Some(100.5)),
        level(Op::FADE_TO, Some(f32::NAN)),
        BACnetLightingCommand {
            fade_time: Some(99),
            ..level(Op::FADE_TO, Some(50.0))
        },
        BACnetLightingCommand {
            fade_time: Some(86_400_001),
            ..level(Op::FADE_TO, Some(50.0))
        },
        BACnetLightingCommand {
            ramp_rate: Some(0.05),
            ..level(Op::RAMP_TO, Some(50.0))
        },
        BACnetLightingCommand {
            step_increment: Some(f32::INFINITY),
            ..command(Op::STEP_OFF)
        },
        BACnetLightingCommand {
            priority: Some(0),
            ..command(Op::WARN_RELINQUISH)
        },
        BACnetLightingCommand {
            priority: Some(17),
            ..level(Op::FADE_TO, Some(50.0))
        },
        // A proprietary operation's priority is checked too, as a Channel
        // checks it.
        BACnetLightingCommand {
            priority: Some(0),
            ..command(Op::from_raw(256))
        },
        BACnetLightingCommand {
            priority: Some(17),
            ..command(Op::from_raw(65_535))
        },
    ];
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    for value in refused {
        assert_error(
            lo.set_lighting_command(value),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(lo.lighting_command(), command(Op::NONE), "{value:?}");
    }
    let accepted = [
        // The range ends.
        BACnetLightingCommand {
            fade_time: Some(100),
            priority: Some(1),
            ..level(Op::FADE_TO, Some(0.0))
        },
        BACnetLightingCommand {
            fade_time: Some(86_400_000),
            priority: Some(16),
            ..level(Op::FADE_TO, Some(100.0))
        },
        BACnetLightingCommand {
            ramp_rate: Some(0.1),
            ..level(Op::RAMP_TO, Some(1.0))
        },
        BACnetLightingCommand {
            step_increment: Some(100.0),
            ..command(Op::STEP_ON)
        },
        // Every other standard operation needs no field.
        command(Op::STEP_UP),
        command(Op::STEP_DOWN),
        command(Op::WARN),
        command(Op::WARN_OFF),
        // A field the operation doesn't use isn't checked.
        BACnetLightingCommand {
            ramp_rate: Some(0.0),
            step_increment: Some(-3.0),
            ..level(Op::FADE_TO, Some(50.0))
        },
        BACnetLightingCommand {
            fade_time: Some(5),
            ..level(Op::RAMP_TO, Some(50.0))
        },
        BACnetLightingCommand {
            target_level: Some(150.0),
            fade_time: Some(1),
            ..command(Op::STEP_UP)
        },
        // Nor is any field of a proprietary operation but its priority.
        BACnetLightingCommand {
            fade_time: Some(0),
            priority: Some(16),
            ..level(Op::from_raw(256), Some(-1.0))
        },
        command(Op::from_raw(65_535)),
    ];
    for value in accepted {
        lo.set_lighting_command(value).unwrap();
        assert_eq!(lo.lighting_command(), value);
    }
}

#[test]
fn lighting_output_takes_a_lighting_command_a_channel_passes_on() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    // The channel value frames the command in context tag 0.
    let framed = [&[0x0E][..], &FADE, &[0x0F]].concat();
    let current = lo.read_property(LC, None).ok();
    let target = MemberDatatype::of(ObjectType::LIGHTING_OUTPUT, LC, current.as_ref());
    assert_eq!(target, MemberDatatype::LightingCommand);
    let value = coerce_channel_value(&PropertyValue::ApplicationData(framed), target).unwrap();
    lo.write_property(LC, None, value, Some(10)).unwrap();
    assert_eq!(lo.read_property(LC, None).unwrap(), octets(&FADE));
}

#[test]
fn lighting_command_default_priority_refuses_six_and_commands_act_at_it() {
    const LCDP: PropertyIdentifier = PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY;
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    for taken in [5, 7] {
        lo.write_property(LCDP, None, PropertyValue::Unsigned(taken), None)
            .unwrap();
        assert_eq!(
            lo.read_property(LCDP, None).unwrap(),
            PropertyValue::Unsigned(taken)
        );
    }
    // 6 is Minimum On/Off's slot (Clause 12.54.27, Table 19-1).
    assert_error(
        lo.write_property(LCDP, None, PropertyValue::Unsigned(6), None),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(
        lo.read_property(LCDP, None).unwrap(),
        PropertyValue::Unsigned(7)
    );
    // A command without a priority acts at the stored default.
    lo.set_lighting_command(BACnetLightingCommand {
        step_increment: Some(5.0),
        ..command(Op::STEP_ON)
    })
    .unwrap();
    let slot = |priority: u32| {
        lo.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(priority))
            .unwrap()
    };
    assert_eq!(slot(7), PropertyValue::Real(1.0));
    assert_eq!(slot(6), PropertyValue::Null);
}

//! Value_Source on the colour objects (#1552, Tables 12-X and 12-Y): absent
//! until tracked, then the writer of the last Present_Value write or of the
//! last Color_Command that set Present_Value.

use super::*;
use crate::audit::ObjectAuditPolicy;
use crate::command_source::{CommandDeviceBinding, CommandOrigin};
use crate::property_metadata::PropertyPresenceCondition;
use crate::traits::BACnetObject;
use bacnet_types::constructed::{BACnetAddress, BACnetColorCommand, BACnetXyColor};
use bacnet_types::enums::{
    AuditLevel, ColorOperation, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn remote(instance: u32, mac: u8) -> CommandOrigin {
    CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[mac]),
        },
        binding: CommandDeviceBinding::Unique(device(instance)),
    }
}

/// Value_Source naming `object`.
fn names(object: ObjectIdentifier) -> PropertyValue {
    let mut bytes = vec![0x1E, 0x1C];
    bytes.extend(object.encode());
    bytes.push(0x1F);
    PropertyValue::ApplicationData(bytes)
}

fn none() -> PropertyValue {
    PropertyValue::ApplicationData(vec![0x08])
}

fn xy(x: f32, y: f32) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Real(x), PropertyValue::Real(y)])
}

fn encoded(command: BACnetColorCommand) -> PropertyValue {
    command::encode(&command)
}

fn fade(x: f32, y: f32) -> PropertyValue {
    encoded(BACnetColorCommand {
        target_color: Some(BACnetXyColor::new(x, y)),
        fade_time: Some(1_000),
        ..BACnetColorCommand::new(ColorOperation::FADE_TO_COLOR)
    })
}

fn stop() -> PropertyValue {
    encoded(BACnetColorCommand::new(ColorOperation::STOP))
}

fn assert_code(result: Result<impl std::fmt::Debug, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { code, .. }) => {
            assert_eq!(code, expected.to_raw() as u32, "expected {expected:?}")
        }
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

fn source(object: &dyn BACnetObject) -> PropertyValue {
    object.read_property(P::VALUE_SOURCE, None).unwrap()
}

fn write_from(
    object: &mut dyn BACnetObject,
    property: P,
    value: PropertyValue,
    origin: &CommandOrigin,
) -> Result<(), Error> {
    object.write_property_from(property, None, value, None, origin)
}

fn tracked_color() -> ColorObject {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    color.set_value_source_tracking(true);
    color
}

fn tracked_temperature() -> ColorTemperatureObject {
    let mut temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    temperature.set_value_source_tracking(true);
    temperature
}

#[test]
fn the_row_is_absent_until_tracked_then_required_after_transition() {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    let untracked = color.property_list().into_owned();
    assert!(!untracked.contains(&P::VALUE_SOURCE));
    assert_code(
        color.read_property(P::VALUE_SOURCE, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert_code(
        write_from(&mut color, P::VALUE_SOURCE, none(), &remote(30, 1)),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    // Untracked, a write needs no writer.
    color
        .write_property(P::PRESENT_VALUE, None, xy(0.2, 0.3), None)
        .unwrap();

    let objects: [(Box<dyn BACnetObject>, Box<dyn BACnetObject>); 2] = [
        (
            Box::new(ColorObject::new(1, "CLR-1").unwrap()),
            Box::new(tracked_color()),
        ),
        (
            Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()),
            Box::new(tracked_temperature()),
        ),
    ];
    for (untracked, tracked) in objects {
        let mut expected = untracked.property_list().into_owned();
        expected.push(P::VALUE_SOURCE);
        assert_eq!(tracked.property_list().as_ref(), expected);
        assert!(tracked.required_properties().contains(&P::VALUE_SOURCE));
        let metadata = tracked.property_metadata();
        let row = metadata
            .iter()
            .find(|r| r.property_identifier == P::VALUE_SOURCE);
        assert_eq!(
            row.unwrap().presence_condition,
            Some(PropertyPresenceCondition::ValueSourceTracking)
        );
        assert_eq!(source(tracked.as_ref()), none());
    }

    // Table order: Value_Source, then the audit rows, then Property_List.
    let mut color = tracked_color();
    color.set_audit_policy(ObjectAuditPolicy {
        level: Some(AuditLevel::AUDIT_ALL),
        ..ObjectAuditPolicy::default()
    });
    let rows: Vec<_> = color
        .property_metadata()
        .iter()
        .map(|row| row.property_identifier)
        .collect();
    assert_eq!(
        rows[rows.len() - 4..],
        [
            P::TRANSITION,
            P::VALUE_SOURCE,
            P::AUDIT_LEVEL,
            P::PROPERTY_LIST
        ]
    );
}

#[test]
fn present_value_and_color_command_writers_become_the_source() {
    let mut color = tracked_color();
    write_from(&mut color, P::PRESENT_VALUE, xy(0.2, 0.3), &remote(30, 1)).unwrap();
    assert_eq!(source(&color), names(device(30)));
    write_from(&mut color, P::COLOR_COMMAND, fade(0.4, 0.4), &remote(31, 2)).unwrap();
    assert_eq!(source(&color), names(device(31)));
    // STOP while the fade runs sets Present_Value where it stands.
    write_from(&mut color, P::COLOR_COMMAND, stop(), &remote(32, 3)).unwrap();
    assert_eq!(source(&color), names(device(32)));
    // STOP with nothing moving sets nothing, so the source stays.
    write_from(&mut color, P::COLOR_COMMAND, stop(), &remote(33, 4)).unwrap();
    assert_eq!(source(&color), names(device(32)));
    assert_code(
        write_from(&mut color, P::VALUE_SOURCE, none(), &remote(33, 4)),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    write_from(&mut color, P::VALUE_SOURCE, none(), &remote(32, 3)).unwrap();
    assert_eq!(source(&color), none());
    // A refused command names nobody.
    assert_code(
        write_from(&mut color, P::COLOR_COMMAND, fade(2.0, 0.0), &remote(34, 5)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(source(&color), none());

    let mut temperature = tracked_temperature();
    let step = encoded(BACnetColorCommand {
        step_increment: Some(100),
        ..BACnetColorCommand::new(ColorOperation::STEP_UP_CCT)
    });
    write_from(&mut temperature, P::COLOR_COMMAND, step, &remote(30, 1)).unwrap();
    assert_eq!(source(&temperature), names(device(30)));
    write_from(
        &mut temperature,
        P::PRESENT_VALUE,
        PropertyValue::Unsigned(3_000),
        &remote(31, 2),
    )
    .unwrap();
    assert_eq!(source(&temperature), names(device(31)));
}

#[test]
fn writes_naming_no_writer_are_refused_and_setters_name_nobody() {
    let mut color = tracked_color();
    for (property, value) in [
        (P::PRESENT_VALUE, xy(0.2, 0.3)),
        (P::COLOR_COMMAND, fade(0.4, 0.4)),
    ] {
        assert_code(
            color.write_property(property, None, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_code(
        color.write_property(P::VALUE_SOURCE, None, none(), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    // Other properties need no writer.
    color
        .write_property(
            P::DEFAULT_FADE_TIME,
            None,
            PropertyValue::Unsigned(500),
            None,
        )
        .unwrap();

    write_from(&mut color, P::PRESENT_VALUE, xy(0.2, 0.3), &remote(30, 1)).unwrap();
    color
        .set_present_value(BACnetXyColor::new(0.3, 0.3))
        .unwrap();
    assert_eq!(source(&color), none());
    assert_code(
        write_from(&mut color, P::VALUE_SOURCE, none(), &remote(30, 1)),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    write_from(&mut color, P::PRESENT_VALUE, xy(0.2, 0.3), &remote(30, 1)).unwrap();
    color
        .set_color_command(BACnetColorCommand::new(ColorOperation::STOP))
        .unwrap();
    assert_eq!(
        source(&color),
        names(device(30)),
        "an idle STOP sets nothing"
    );

    let mut temperature = tracked_temperature();
    write_from(
        &mut temperature,
        P::PRESENT_VALUE,
        PropertyValue::Unsigned(5_000),
        &remote(30, 1),
    )
    .unwrap();
    // Limits that keep Present_Value leave its source; limits that move it don't.
    temperature.set_min_max(1_000, 6_000).unwrap();
    assert_eq!(source(&temperature), names(device(30)));
    temperature.set_min_max(1_000, 4_000).unwrap();
    assert_eq!(source(&temperature), none());
}

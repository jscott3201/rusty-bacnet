//! Value_Source, Value_Source_Array and Last_Command_Time on the two
//! lighting outputs (#1866, Clause 19.5): each command's writer is its
//! slot's source, and only that writer may correct it.

use super::*;
use crate::command_source::CommandOrigin;
use bacnet_encoding::constructed::encode_value_source;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetValueSource};
use bacnet_types::enums::ErrorCode;
use bytes::BytesMut;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const LC: PropertyIdentifier = PropertyIdentifier::LIGHTING_COMMAND;
const VS: PropertyIdentifier = PropertyIdentifier::VALUE_SOURCE;
const LCT: PropertyIdentifier = PropertyIdentifier::LAST_COMMAND_TIME;

/// A local writer owned by Device `device`, published as `object` (an
/// Analog Value of that instance).
fn origin(device: u32, object: u32) -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, device).unwrap(),
        initiating_object: Some(ObjectIdentifier::new(ObjectType::ANALOG_VALUE, object).unwrap()),
    }
}

fn encoded(source: &BACnetValueSource) -> PropertyValue {
    let mut bytes = BytesMut::new();
    encode_value_source(&mut bytes, source).unwrap();
    PropertyValue::ApplicationData(bytes.to_vec())
}

/// What `origin(_, object)` publishes.
fn published(object: u32) -> PropertyValue {
    encoded(&BACnetValueSource::Object(BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, object).unwrap(),
    }))
}

fn read(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    object.read_property(property, index).unwrap()
}

fn code(result: Result<(), Error>) -> ErrorCode {
    match result {
        Err(Error::Protocol { code, .. }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A Lighting_Command FADE_TO `level` at `priority`, encoded.
fn fade_to(level: f32, priority: u8) -> PropertyValue {
    let mut command = BACnetLightingCommand::new(LightingOperation::FADE_TO);
    command.target_level = Some(level);
    command.priority = Some(priority);
    command::encode(&command)
}

#[test]
fn lighting_output_tracks_each_writer_and_lets_only_it_correct() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    assert_eq!(read(&lo, VS, None), encoded(&BACnetValueSource::None));
    let start = read(&lo, LCT, None);

    // A context-free command has no writer to record.
    for (property, value) in [(PV, PropertyValue::Real(50.0)), (LC, fade_to(50.0, 8))] {
        assert_eq!(
            code(lo.write_property(property, None, value, Some(8))),
            ErrorCode::WRITE_ACCESS_DENIED
        );
    }

    lo.write_property_from(PV, None, PropertyValue::Real(50.0), Some(8), &origin(1, 10))
        .unwrap();
    assert_eq!(read(&lo, VS, None), published(10));
    assert_eq!(
        read(&lo, PropertyIdentifier::VALUE_SOURCE_ARRAY, Some(8)),
        published(10)
    );
    let after_pv = read(&lo, LCT, None);
    assert_ne!(after_pv, start);

    // A Lighting_Command at priority 5 is that writer's command there.
    lo.write_property_from(LC, None, fade_to(80.0, 5), None, &origin(2, 20))
        .unwrap();
    assert_eq!(read(&lo, VS, None), published(20));
    assert_eq!(
        read(&lo, PropertyIdentifier::VALUE_SOURCE_ARRAY, Some(5)),
        published(20)
    );
    assert_ne!(read(&lo, LCT, None), after_pv);

    // WARN acts on no slot, so the slot keeps its writer.
    lo.write_property_from(PV, None, PropertyValue::Real(-1.0), Some(5), &origin(3, 30))
        .unwrap();
    assert_eq!(read(&lo, VS, None), published(20));

    // Only the slot's writer corrects its source.
    let claim = encoded(&BACnetValueSource::None);
    assert_eq!(
        code(lo.write_property_from(VS, None, claim.clone(), Some(5), &origin(3, 30))),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    lo.write_property_from(VS, None, claim.clone(), Some(5), &origin(2, 21))
        .unwrap();
    assert_eq!(read(&lo, VS, None), claim);

    // An application command names no writer: the slot reads NONE and
    // the earlier writer may no longer correct it.
    let mut command = BACnetLightingCommand::new(LightingOperation::FADE_TO);
    command.target_level = Some(30.0);
    command.priority = Some(8);
    lo.set_lighting_command(command).unwrap();
    assert_eq!(
        read(&lo, PropertyIdentifier::VALUE_SOURCE_ARRAY, Some(8)),
        encoded(&BACnetValueSource::None)
    );
    assert_eq!(
        code(lo.write_property_from(VS, None, published(10), Some(8), &origin(1, 10))),
        ErrorCode::WRITE_ACCESS_DENIED
    );
}

#[test]
fn an_egress_that_runs_out_moves_last_command_time_on() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.write_property(
        PropertyIdentifier::BLINK_WARN_ENABLE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    lo.write_property(
        PropertyIdentifier::EGRESS_TIME,
        None,
        PropertyValue::Unsigned(5),
        None,
    )
    .unwrap();
    let writer = origin(1, 10);
    lo.write_property_from(PV, None, PropertyValue::Real(60.0), Some(8), &writer)
        .unwrap();
    // WARN_RELINQUISH holds the level for Egress_Time.
    lo.write_property_from(PV, None, PropertyValue::Real(-2.0), Some(8), &writer)
        .unwrap();
    let held = read(&lo, LCT, None);
    assert_eq!(read(&lo, PV, None), PropertyValue::Real(60.0));
    assert!(lo.advance_time_internal(Duration::from_secs(5)));
    assert_eq!(read(&lo, PV, None), PropertyValue::Real(0.0));
    assert_ne!(read(&lo, LCT, None), held);
    // The relinquished slot keeps its writer.
    assert_eq!(
        read(&lo, PropertyIdentifier::VALUE_SOURCE_ARRAY, Some(8)),
        published(10)
    );

    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    blo.write_property(
        PropertyIdentifier::BLINK_WARN_ENABLE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    blo.write_property(
        PropertyIdentifier::EGRESS_TIME,
        None,
        PropertyValue::Unsigned(5),
        None,
    )
    .unwrap();
    blo.write_property_from(PV, None, PropertyValue::Enumerated(1), Some(8), &writer)
        .unwrap();
    blo.write_property_from(PV, None, PropertyValue::Enumerated(4), Some(8), &writer)
        .unwrap();
    let held = read(&blo, LCT, None);
    assert!(blo.advance_time_internal(Duration::from_secs(5)));
    assert_eq!(read(&blo, PV, None), PropertyValue::Enumerated(0));
    assert_ne!(read(&blo, LCT, None), held);
}

#[test]
fn binary_lighting_output_tracks_each_writer_and_lets_only_it_correct() {
    let mut blo = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    assert_eq!(
        code(blo.write_property(PV, None, PropertyValue::Enumerated(1), Some(8))),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    let start = read(&blo, LCT, None);
    blo.write_property_from(
        PV,
        None,
        PropertyValue::Enumerated(1),
        Some(8),
        &origin(1, 10),
    )
    .unwrap();
    assert_eq!(read(&blo, VS, None), published(10));
    assert_ne!(read(&blo, LCT, None), start);

    // WARN (2) and a STOP (5) with nothing to stop leave the slot's writer.
    for value in [2, 5] {
        blo.write_property_from(
            PV,
            None,
            PropertyValue::Enumerated(value),
            Some(8),
            &origin(2, 20),
        )
        .unwrap();
        assert_eq!(read(&blo, VS, None), published(10), "{value}");
    }
    assert_eq!(
        code(blo.write_property_from(VS, None, published(20), Some(8), &origin(2, 20))),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    blo.write_property_from(VS, None, published(11), Some(8), &origin(1, 10))
        .unwrap();
    assert_eq!(read(&blo, VS, None), published(11));
    // A higher priority writer takes over the published source.
    blo.write_property_from(
        PV,
        None,
        PropertyValue::Enumerated(0),
        Some(4),
        &origin(2, 20),
    )
    .unwrap();
    assert_eq!(read(&blo, VS, None), published(20));
}

/// A Lighting_Command with `operation` at priority 8, encoded.
fn at_eight(operation: LightingOperation) -> PropertyValue {
    let mut command = BACnetLightingCommand::new(operation);
    command.priority = Some(8);
    command::encode(&command)
}

/// An ignored step and a STOP with nothing to stop act on no slot, so the
/// slot keeps its writer; a STOP that ends a fade writes the slot and
/// records the STOP's writer.
#[test]
fn ignored_steps_and_idle_stops_keep_the_writer_and_a_stop_that_ends_a_fade_records_its_own() {
    let mut f = super::engine_tests::Fixture::new();
    f.lo.write_property_from(PV, None, PropertyValue::Real(0.0), Some(8), &origin(1, 10))
        .unwrap();
    // STEP_UP does nothing from off, and nothing runs to STOP.
    for operation in [LightingOperation::STEP_UP, LightingOperation::STOP] {
        f.lo.write_property_from(LC, None, at_eight(operation), None, &origin(2, 20))
            .unwrap();
        assert_eq!(read(&f.lo, VS, None), published(10), "{operation:?}");
    }

    let mut fade = BACnetLightingCommand::new(LightingOperation::FADE_TO);
    fade.target_level = Some(80.0);
    fade.fade_time = Some(10_000);
    fade.priority = Some(8);
    f.lo.write_property_from(LC, None, command::encode(&fade), None, &origin(1, 10))
        .unwrap();
    f.at(1_000);
    f.lo.write_property_from(
        LC,
        None,
        at_eight(LightingOperation::STOP),
        None,
        &origin(2, 20),
    )
    .unwrap();
    assert_eq!(read(&f.lo, VS, None), published(20));
}

/// A trim change advances the engine first, so an egress that ran out
/// before it ends its command there and moves Last_Command_Time on.
#[test]
fn an_egress_ending_inside_a_trim_change_moves_last_command_time_on() {
    let mut f = super::engine_tests::Fixture::new();
    f.write(
        PropertyIdentifier::BLINK_WARN_ENABLE,
        PropertyValue::Boolean(true),
    );
    f.write(PropertyIdentifier::EGRESS_TIME, PropertyValue::Unsigned(5));
    let writer = origin(1, 10);
    f.lo.write_property_from(PV, None, PropertyValue::Real(60.0), Some(8), &writer)
        .unwrap();
    f.lo.write_property_from(PV, None, PropertyValue::Real(-2.0), Some(8), &writer)
        .unwrap();
    let held = read(&f.lo, LCT, None);
    f.at(6_000);
    f.lo.set_high_end_trim(Some(90.0)).unwrap();
    assert_eq!(read(&f.lo, PV, None), PropertyValue::Real(0.0));
    assert_ne!(read(&f.lo, LCT, None), held);
}

//! Write validation on the Staging object's configuration rows: a refused
//! write leaves the object and its queued plan as they were, and an accepted
//! one restarts evaluation (#2183).
use super::*;
use bacnet_types::error::RemoteError;

fn config() -> StagingConfig {
    StagingConfig {
        present_value: 15.0,
        min_present_value: -1.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![
            BACnetStageLimitValue {
                limit: 10.0,
                values: vec![false],
                deadband: 1.0,
            },
            BACnetStageLimitValue {
                limit: 20.0,
                values: vec![true],
                deadband: 2.0,
            },
        ],
        target_references: vec![BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
        }],
        stage_names: None,
    }
}

/// A Staging object at stage 2 with its first plan already taken.
fn settled() -> StagingObject {
    let mut object = StagingObject::new(1, "STG-1", config()).unwrap();
    assert!(object.take_staging_write_plan().is_some());
    object
}

fn assert_code(error: Error, code: ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: actual,
        }) if actual == code => {}
        other => panic!("expected PROPERTY / {code:?}, got {other:?}"),
    }
}

fn write(
    object: &mut StagingObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

fn read(object: &StagingObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

/// What a refused write must leave alone.
fn state(object: &StagingObject) -> (PropertyValue, PropertyValue, PropertyValue, u64) {
    (
        read(object, PropertyIdentifier::PRIORITY_FOR_WRITING),
        read(object, PropertyIdentifier::MIN_PRES_VALUE),
        read(object, PropertyIdentifier::PRESENT_STAGE),
        object.staging_generation(),
    )
}

#[test]
fn priority_for_writing_refuses_a_wrong_type_and_any_value_outside_one_to_sixteen() {
    let mut object = settled();
    let before = state(&object);
    for value in [
        PropertyValue::Real(3.0),
        PropertyValue::Enumerated(3),
        PropertyValue::Signed(3),
    ] {
        assert_code(
            write(&mut object, PropertyIdentifier::PRIORITY_FOR_WRITING, value).unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    // 256 and u64::MAX don't fit the octet the priority is held in; 0 and 17
    // fit but aren't priorities.
    for raw in [0, 17, 256, u64::MAX] {
        assert_code(
            write(
                &mut object,
                PropertyIdentifier::PRIORITY_FOR_WRITING,
                PropertyValue::Unsigned(raw),
            )
            .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(state(&object), before);
    assert!(object.take_staging_write_plan().is_none());
}

#[test]
fn a_new_priority_for_writing_reapplies_the_stage_at_that_priority() {
    let mut object = settled();
    let generation = object.staging_generation();
    // The same priority changes nothing.
    write(
        &mut object,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyValue::Unsigned(8),
    )
    .unwrap();
    assert_eq!(object.staging_generation(), generation);
    assert!(object.take_staging_write_plan().is_none());

    for priority in [1, 16] {
        write(
            &mut object,
            PropertyIdentifier::PRIORITY_FOR_WRITING,
            PropertyValue::Unsigned(u64::from(priority)),
        )
        .unwrap();
        assert_eq!(
            read(&object, PropertyIdentifier::PRIORITY_FOR_WRITING),
            PropertyValue::Unsigned(u64::from(priority))
        );
        let plan = object.take_staging_write_plan().unwrap();
        assert_eq!(plan.priority, priority);
        assert_eq!(plan.generation, object.staging_generation());
        assert!(plan.generation > generation);
        assert_eq!(
            read(&object, PropertyIdentifier::PRESENT_STAGE),
            PropertyValue::Unsigned(2)
        );
    }
}

#[test]
fn min_pres_value_refuses_a_wrong_type_a_non_finite_value_and_one_inside_the_ladder() {
    let mut object = settled();
    let before = state(&object);
    assert_code(
        write(
            &mut object,
            PropertyIdentifier::MIN_PRES_VALUE,
            PropertyValue::Unsigned(0),
        )
        .unwrap_err(),
        ErrorCode::INVALID_DATA_TYPE,
    );
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_code(
            write(
                &mut object,
                PropertyIdentifier::MIN_PRES_VALUE,
                PropertyValue::Real(value),
            )
            .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    // The first stage's lower deadband edge is 9.0; Min_Pres_Value must stay
    // strictly below it.
    for value in [9.0, 9.5] {
        assert_code(
            write(
                &mut object,
                PropertyIdentifier::MIN_PRES_VALUE,
                PropertyValue::Real(value),
            )
            .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(state(&object), before);
    assert!(object.take_staging_write_plan().is_none());

    // An accepted change restarts evaluation; the same value doesn't.
    write(
        &mut object,
        PropertyIdentifier::MIN_PRES_VALUE,
        PropertyValue::Real(-1.0),
    )
    .unwrap();
    assert!(object.take_staging_write_plan().is_none());
    write(
        &mut object,
        PropertyIdentifier::MIN_PRES_VALUE,
        PropertyValue::Real(8.0),
    )
    .unwrap();
    assert_eq!(
        read(&object, PropertyIdentifier::MIN_PRES_VALUE),
        PropertyValue::Real(8.0)
    );
    assert!(object.take_staging_write_plan().is_some());
}

#[test]
fn writing_out_of_service_to_its_current_value_is_a_no_op() {
    let mut object = settled();
    let generation = object.staging_generation();
    write(
        &mut object,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(object.staging_generation(), generation);
    assert!(object.take_staging_write_plan().is_none());
}

#[test]
fn a_completion_that_changes_no_reliability_reports_no_change() {
    let mut object = settled();
    let generation = object.staging_generation();
    // Already NO_FAULT_DETECTED: a success changes nothing.
    assert!(!object.complete_staging_write_plan(generation, true));
    assert!(object.complete_staging_write_plan(generation, false));
    // A second failure leaves it UNRELIABLE_OTHER, unchanged.
    assert!(!object.complete_staging_write_plan(generation, false));
    assert_eq!(
        read(&object, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw())
    );
}

#[test]
fn a_stage_encoding_with_trailing_octets_is_refused() {
    let mut object = settled();
    let before = read(&object, PropertyIdentifier::STAGES);
    let mut encoded = BytesMut::new();
    encode_stage_limit_value(
        &mut encoded,
        &BACnetStageLimitValue {
            limit: 16.0,
            values: vec![true],
            deadband: 1.0,
        },
    );
    let mut bytes = encoded.to_vec();
    bytes.push(0x00);
    assert_code(
        object
            .write_property(
                PropertyIdentifier::STAGES,
                Some(2),
                PropertyValue::ApplicationData(bytes),
                None,
            )
            .unwrap_err(),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    assert_eq!(read(&object, PropertyIdentifier::STAGES), before);
}

#[test]
fn stage_names_refuse_values_that_are_not_character_strings() {
    let mut config = config();
    config.stage_names = Some(vec!["Low".into(), "High".into()]);
    let mut object = StagingObject::new(1, "STG-1", config).unwrap();
    let before = read(&object, PropertyIdentifier::STAGE_NAMES);
    for (index, value) in [
        (None, PropertyValue::CharacterString("Low".into())),
        (
            None,
            PropertyValue::List(vec![
                PropertyValue::CharacterString("Low".into()),
                PropertyValue::Unsigned(2),
            ]),
        ),
        (Some(2), PropertyValue::Unsigned(2)),
    ] {
        assert_code(
            object
                .write_property(PropertyIdentifier::STAGE_NAMES, index, value, None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_code(
        object
            .write_property(
                PropertyIdentifier::STAGE_NAMES,
                Some(3),
                PropertyValue::CharacterString("Top".into()),
                None,
            )
            .unwrap_err(),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    assert_eq!(read(&object, PropertyIdentifier::STAGE_NAMES), before);
}

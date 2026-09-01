use super::*;

use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, LifeSafetyOperation, ObjectType, PropertyIdentifier, SilencedState,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

fn request(
    operation: LifeSafetyOperation,
    object_identifier: Option<ObjectIdentifier>,
) -> LifeSafetyOperationRequest {
    LifeSafetyOperationRequest {
        requesting_process_identifier: 7,
        requesting_source: "operator".into(),
        request: operation,
        object_identifier,
    }
}

fn read_enumerated(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> u32 {
    match db.get(&oid).unwrap().read_property(property, None).unwrap() {
        PropertyValue::Enumerated(value) => value,
        other => panic!("expected enumerated value, got {other:?}"),
    }
}

fn assert_protocol_error(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(matches!(
        error,
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32
    ));
}

#[test]
fn life_safety_operation_targeted_silence_changes_object() {
    let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();

    let changed =
        handle_life_safety_operation(&mut db, &request(LifeSafetyOperation::SILENCE, Some(oid)))
            .unwrap();

    assert_eq!(changed, vec![oid]);
    assert_eq!(
        read_enumerated(&db, oid, PropertyIdentifier::SILENCED),
        SilencedState::ALL_SILENCED.to_raw()
    );
}

#[test]
fn life_safety_operation_replay_without_response_cache_is_invalid_state() {
    let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();
    let request = request(LifeSafetyOperation::SILENCE, Some(oid));

    assert_eq!(
        handle_life_safety_operation(&mut db, &request).unwrap(),
        vec![oid]
    );
    let error = handle_life_safety_operation(&mut db, &request).unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::INVALID_OPERATION_IN_THIS_STATE,
    );
}

#[test]
fn life_safety_operation_reports_target_errors() {
    let missing = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 44).unwrap();
    let mut db = ObjectDatabase::new();
    let error = handle_life_safety_operation(
        &mut db,
        &request(LifeSafetyOperation::SILENCE, Some(missing)),
    )
    .unwrap_err();
    assert_protocol_error(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);

    let analog_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(Box::new(AnalogInputObject::new(1, "analog", 62).unwrap()))
        .unwrap();
    let error = handle_life_safety_operation(
        &mut db,
        &request(LifeSafetyOperation::SILENCE, Some(analog_oid)),
    )
    .unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

#[test]
fn life_safety_operation_rejects_none_and_reserved_and_reports_missing_reset_executor() {
    let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::RESET);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();

    for operation in [LifeSafetyOperation::NONE, LifeSafetyOperation::from_raw(10)] {
        let error =
            handle_life_safety_operation(&mut db, &request(operation, Some(oid))).unwrap_err();
        assert_protocol_error(error, ErrorClass::OBJECT, ErrorCode::VALUE_OUT_OF_RANGE);
    }

    let error =
        handle_life_safety_operation(&mut db, &request(LifeSafetyOperation::RESET, Some(oid)))
            .unwrap_err();
    assert_protocol_error(
        error,
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

#[test]
fn life_safety_operation_without_target_suppresses_missing_reset_executor() {
    let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::RESET);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();

    let changed =
        handle_life_safety_operation(&mut db, &request(LifeSafetyOperation::RESET, None)).unwrap();
    assert!(changed.is_empty());
    assert_eq!(
        read_enumerated(&db, oid, PropertyIdentifier::OPERATION_EXPECTED),
        LifeSafetyOperation::RESET.to_raw()
    );
}

#[test]
fn life_safety_operation_without_target_attempts_every_object() {
    let point_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let zone_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_operation_expected(LifeSafetyOperation::SILENCE_VISUAL);
    let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_operation_expected(LifeSafetyOperation::SILENCE_VISUAL);
    let mut already_desired = LifeSafetyPointObject::new(2, "already").unwrap();
    already_desired.set_silenced(SilencedState::VISIBLE_SILENCED);
    let invalid_state = LifeSafetyPointObject::new(3, "invalid").unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();
    db.add(Box::new(zone)).unwrap();
    db.add(Box::new(already_desired)).unwrap();
    db.add(Box::new(invalid_state)).unwrap();
    db.add(Box::new(AnalogInputObject::new(1, "analog", 62).unwrap()))
        .unwrap();

    let mut changed =
        handle_life_safety_operation(&mut db, &request(LifeSafetyOperation::SILENCE_VISUAL, None))
            .unwrap();
    changed.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
    let mut expected = vec![point_oid, zone_oid];
    expected.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));

    assert_eq!(changed, expected);
    for oid in [point_oid, zone_oid] {
        assert_eq!(
            read_enumerated(&db, oid, PropertyIdentifier::SILENCED),
            SilencedState::VISIBLE_SILENCED.to_raw()
        );
    }
    assert_eq!(
        read_enumerated(
            &db,
            ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 2).unwrap(),
            PropertyIdentifier::SILENCED,
        ),
        SilencedState::VISIBLE_SILENCED.to_raw()
    );
    assert_eq!(
        read_enumerated(
            &db,
            ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 3).unwrap(),
            PropertyIdentifier::SILENCED,
        ),
        SilencedState::UNSILENCED.to_raw()
    );
}

#[test]
fn targetless_mixed_outcomes_aggregate_only_successful_exact_deltas() {
    let point_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let zone_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_silenced(SilencedState::VISIBLE_SILENCED);
    point.set_operation_expected(LifeSafetyOperation::SILENCE_VISUAL);
    let mut zone = LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_operation_expected(LifeSafetyOperation::SILENCE_VISUAL);
    let failed = LifeSafetyPointObject::new(2, "failed").unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();
    db.add(Box::new(zone)).unwrap();
    db.add(Box::new(failed)).unwrap();

    let result = handle_life_safety_operation_detailed(
        &mut db,
        &request(LifeSafetyOperation::SILENCE_VISUAL, None),
    )
    .unwrap();

    assert_eq!(result.applied_object_identifiers, vec![point_oid, zone_oid]);
    assert_eq!(result.cov_changes.len(), 2);
    assert_eq!(result.cov_changes[0].object_identifier, point_oid);
    assert_eq!(
        result.cov_changes[0].changed_properties,
        vec![PropertyIdentifier::OPERATION_EXPECTED]
    );
    assert_eq!(result.cov_changes[1].object_identifier, zone_oid);
    assert_eq!(
        result.cov_changes[1].changed_properties,
        vec![
            PropertyIdentifier::SILENCED,
            PropertyIdentifier::OPERATION_EXPECTED,
        ]
    );
}

#[test]
fn life_safety_operation_without_target_succeeds_with_empty_database() {
    let mut db = ObjectDatabase::new();
    assert!(
        handle_life_safety_operation(&mut db, &request(LifeSafetyOperation::UNSILENCE, None),)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn life_safety_properties_cannot_be_bypassed_through_write_property_multiple() {
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

    let oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(LifeSafetyPointObject::new(1, "point").unwrap()))
        .unwrap();
    let mut mode = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut mode, 1);
    let mut silenced = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut silenced, 3);
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::MODE,
                    property_array_index: None,
                    value: mode.to_vec(),
                    priority: None,
                },
                BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::SILENCED,
                    property_array_index: None,
                    value: silenced.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);

    let error = handle_write_property_multiple(&mut db, &encoded).unwrap_err();
    assert_protocol_error(error, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(
        read_enumerated(&db, oid, PropertyIdentifier::MODE),
        0,
        "the earlier MODE write must roll back"
    );
    assert_eq!(
        read_enumerated(&db, oid, PropertyIdentifier::SILENCED),
        SilencedState::UNSILENCED.to_raw()
    );
}

use super::*;

fn reference(object_type: ObjectType, instance: u32) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: ObjectIdentifier::new(object_type, instance).unwrap(),
    }
}

fn stage(limit: f32, values: &[bool], deadband: f32) -> BACnetStageLimitValue {
    BACnetStageLimitValue {
        limit,
        values: values.to_vec(),
        deadband,
    }
}

fn config() -> StagingConfig {
    StagingConfig {
        present_value: 5.0,
        min_present_value: -1.0,
        units: 62,
        priority_for_writing: 8,
        stages: vec![
            stage(10.0, &[false, true], 1.0),
            stage(20.0, &[true, false], 2.0),
            stage(30.0, &[true, true], 1.0),
        ],
        target_references: vec![
            reference(ObjectType::BINARY_OUTPUT, 1),
            reference(ObjectType::BINARY_VALUE, 2),
        ],
        stage_names: Some(vec!["Low".into(), "Middle".into(), "High".into()]),
    }
}

fn assert_protocol_error(error: Error, class: ErrorClass, code: ErrorCode) {
    match error {
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } => {
            assert_eq!(actual_class, class.to_raw() as u32);
            assert_eq!(actual_code, code.to_raw() as u32);
        }
        other => panic!("expected {class:?} / {code:?}, got {other:?}"),
    }
}

fn write_real(object: &mut StagingObject, value: f32) {
    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(value),
            None,
        )
        .unwrap();
}

fn present_stage(object: &StagingObject) -> u64 {
    match object
        .read_property(PropertyIdentifier::PRESENT_STAGE, None)
        .unwrap()
    {
        PropertyValue::Unsigned(value) => value,
        other => panic!("expected Unsigned Present_Stage, got {other:?}"),
    }
}

#[test]
fn fresh_stage_is_uninitialized_and_present_value_is_real() {
    let object = StagingObject::new(1, "STG-1", config()).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(5.0)
    );
    assert_protocol_error(
        object
            .read_property(PropertyIdentifier::PRESENT_STAGE, None)
            .unwrap_err(),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_NOT_INITIALIZED,
    );
}

#[test]
fn wrong_present_value_type_and_nonfinite_values_do_not_mutate() {
    let mut object = StagingObject::new(1, "STG-1", config()).unwrap();
    for value in [PropertyValue::Unsigned(10), PropertyValue::Real(f32::NAN)] {
        assert!(object
            .write_property(PropertyIdentifier::PRESENT_VALUE, None, value, None)
            .is_err());
        assert_eq!(
            object
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Real(5.0)
        );
        assert!(object.pending_plan.is_none());
    }
}

#[test]
fn construction_accepts_two_and_three_stages_and_rejects_all_config_boundaries() {
    let mut two = config();
    two.stages.pop();
    two.stage_names.as_mut().unwrap().pop();
    assert!(StagingObject::new(1, "two", two).is_ok());
    assert!(StagingObject::new(2, "three", config()).is_ok());

    let mut invalid = Vec::new();
    let mut one = config();
    one.stages.truncate(1);
    one.stage_names.as_mut().unwrap().truncate(1);
    invalid.push(one);
    let mut zero = config();
    zero.stages.clear();
    zero.stage_names.as_mut().unwrap().clear();
    invalid.push(zero);
    let mut nonfinite_pv = config();
    nonfinite_pv.present_value = f32::INFINITY;
    invalid.push(nonfinite_pv);
    let mut nonfinite_limit = config();
    nonfinite_limit.stages[1].limit = f32::NAN;
    invalid.push(nonfinite_limit);
    let mut bad_priority = config();
    bad_priority.priority_for_writing = 0;
    invalid.push(bad_priority);
    let mut high_priority = config();
    high_priority.priority_for_writing = 17;
    invalid.push(high_priority);
    let mut negative_deadband = config();
    negative_deadband.stages[0].deadband = -0.1;
    invalid.push(negative_deadband);
    let mut nonfinite_deadband = config();
    nonfinite_deadband.stages[0].deadband = f32::INFINITY;
    invalid.push(nonfinite_deadband);
    let mut overlap = config();
    overlap.stages[1].limit = 12.0;
    invalid.push(overlap);
    let mut bad_min = config();
    bad_min.min_present_value = 9.0;
    invalid.push(bad_min);
    let mut nonfinite_min = config();
    nonfinite_min.min_present_value = f32::NEG_INFINITY;
    invalid.push(nonfinite_min);
    let mut bad_bits = config();
    bad_bits.stages[0].values.pop();
    invalid.push(bad_bits);
    let mut bad_names = config();
    bad_names.stage_names.as_mut().unwrap().pop();
    invalid.push(bad_names);
    let mut invalid_target = config();
    invalid_target.target_references[0] = reference(ObjectType::ANALOG_OUTPUT, 1);
    invalid.push(invalid_target);

    for candidate in invalid {
        assert!(StagingObject::new(3, "invalid", candidate).is_err());
    }

    let mut remote = config();
    remote.target_references[0].device_identifier =
        Some(ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap());
    assert_protocol_error(
        StagingObject::new(4, "remote", remote).err().unwrap(),
        ErrorClass::PROPERTY,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

#[test]
fn clamp_selection_and_bidirectional_hysteresis_follow_stage_boundaries() {
    let mut object = StagingObject::new(1, "STG-1", config()).unwrap();
    write_real(&mut object, -100.0);
    assert_eq!(present_stage(&object), 1);
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(-1.0)
    );
    object.take_staging_write_plan_internal();

    write_real(&mut object, 10.5);
    assert_eq!(present_stage(&object), 1, "retain through upper deadband");
    assert!(object.take_staging_write_plan_internal().is_none());
    write_real(&mut object, 11.1);
    assert_eq!(present_stage(&object), 2);
    object.take_staging_write_plan_internal();

    write_real(&mut object, 9.1);
    assert_eq!(present_stage(&object), 2, "retain above prior lower edge");
    assert!(object.take_staging_write_plan_internal().is_none());
    write_real(&mut object, 8.9);
    assert_eq!(present_stage(&object), 1);
    object.take_staging_write_plan_internal();

    write_real(&mut object, 100.0);
    assert_eq!(present_stage(&object), 3);
    assert_eq!(
        object
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Real(30.0)
    );
}

#[test]
fn array_reads_are_whole_count_one_based_and_strictly_bounded() {
    let object = StagingObject::new(1, "STG-1", config()).unwrap();
    for (property, count) in [
        (PropertyIdentifier::STAGES, 3),
        (PropertyIdentifier::STAGE_NAMES, 3),
        (PropertyIdentifier::TARGET_REFERENCES, 2),
    ] {
        let PropertyValue::List(all) = object.read_property(property, None).unwrap() else {
            panic!("whole array must be a list");
        };
        assert_eq!(all.len(), count);
        assert_eq!(
            object.read_property(property, Some(0)).unwrap(),
            PropertyValue::Unsigned(count as u64)
        );
        assert_eq!(object.read_property(property, Some(1)).unwrap(), all[0]);
        assert_protocol_error(
            object
                .read_property(property, Some(count as u32 + 1))
                .unwrap_err(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

#[test]
fn supported_configuration_writes_revalidate_atomically_and_reevaluate() {
    let mut object = StagingObject::new(1, "STG-1", config()).unwrap();
    write_real(&mut object, 15.0);
    assert_eq!(present_stage(&object), 2);
    object.take_staging_write_plan_internal();

    let replacement = stage(16.0, &[false, false], 2.0);
    let mut encoded = BytesMut::new();
    encode_stage_limit_value(&mut encoded, &replacement);
    object
        .write_property(
            PropertyIdentifier::STAGES,
            Some(2),
            PropertyValue::ApplicationData(encoded.to_vec()),
            None,
        )
        .unwrap();
    assert_eq!(present_stage(&object), 2);
    assert!(object.take_staging_write_plan_internal().is_some());

    let before = object
        .read_property(PropertyIdentifier::STAGES, None)
        .unwrap();
    let invalid = stage(9.0, &[false, false], 2.0);
    let mut encoded = BytesMut::new();
    encode_stage_limit_value(&mut encoded, &invalid);
    assert!(object
        .write_property(
            PropertyIdentifier::STAGES,
            Some(2),
            PropertyValue::ApplicationData(encoded.to_vec()),
            None,
        )
        .is_err());
    assert_eq!(
        object
            .read_property(PropertyIdentifier::STAGES, None)
            .unwrap(),
        before
    );

    let PropertyValue::List(mut resized) = before else {
        panic!("whole Stages must be a list");
    };
    resized.pop();
    assert_protocol_error(
        object
            .write_property(
                PropertyIdentifier::STAGES,
                None,
                PropertyValue::List(resized),
                None,
            )
            .unwrap_err(),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );

    let remote = BACnetDeviceObjectReference {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 5).unwrap()),
        object_identifier: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 9).unwrap(),
    };
    let mut encoded = BytesMut::new();
    encode_device_object_reference(&mut encoded, &remote);
    let before = object
        .read_property(PropertyIdentifier::TARGET_REFERENCES, None)
        .unwrap();
    assert_protocol_error(
        object
            .write_property(
                PropertyIdentifier::TARGET_REFERENCES,
                Some(1),
                PropertyValue::ApplicationData(encoded.to_vec()),
                None,
            )
            .unwrap_err(),
        ErrorClass::PROPERTY,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::TARGET_REFERENCES, None)
            .unwrap(),
        before
    );
}

#[test]
fn whole_configuration_array_writes_preserve_shape_and_reapply_targets() {
    let mut object = StagingObject::new(1, "STG-1", config()).unwrap();
    object.take_staging_write_plan_internal();

    let stages = vec![
        stage(12.0, &[true, false], 1.0),
        stage(22.0, &[false, true], 2.0),
        stage(32.0, &[true, true], 1.0),
    ];
    let encoded_stages = stages
        .iter()
        .map(|stage| {
            let mut encoded = BytesMut::new();
            encode_stage_limit_value(&mut encoded, stage);
            PropertyValue::ApplicationData(encoded.to_vec())
        })
        .collect();
    object
        .write_property(
            PropertyIdentifier::STAGES,
            None,
            PropertyValue::List(encoded_stages),
            None,
        )
        .unwrap();
    assert_eq!(object.max_present_value(), 32.0);
    assert!(object.take_staging_write_plan_internal().is_some());

    object
        .write_property(
            PropertyIdentifier::STAGE_NAMES,
            None,
            PropertyValue::List(vec![
                PropertyValue::CharacterString("One".into()),
                PropertyValue::CharacterString("Two".into()),
                PropertyValue::CharacterString("Three".into()),
            ]),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::STAGE_NAMES, Some(2))
            .unwrap(),
        PropertyValue::CharacterString("Two".into())
    );

    let references = vec![
        reference(ObjectType::BINARY_OUTPUT, 9),
        reference(ObjectType::BINARY_LIGHTING_OUTPUT, 10),
    ];
    let encoded_references = references
        .iter()
        .map(|reference| {
            let mut encoded = BytesMut::new();
            encode_device_object_reference(&mut encoded, reference);
            PropertyValue::ApplicationData(encoded.to_vec())
        })
        .collect();
    object
        .write_property(
            PropertyIdentifier::TARGET_REFERENCES,
            None,
            PropertyValue::List(encoded_references),
            None,
        )
        .unwrap();
    let plan = object.take_staging_write_plan_internal().unwrap();
    assert_eq!(
        plan.writes[1].object_identifier,
        references[1].object_identifier
    );

    assert_protocol_error(
        object
            .write_property(
                PropertyIdentifier::STAGES,
                Some(0),
                PropertyValue::Unsigned(3),
                None,
            )
            .unwrap_err(),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn out_of_service_suppresses_plans_and_return_to_service_reapplies() {
    let mut object = StagingObject::new(1, "STG-1", config()).unwrap();
    write_real(&mut object, 5.0);
    object.take_staging_write_plan_internal();
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    write_real(&mut object, 25.0);
    assert_eq!(present_stage(&object), 3);
    assert!(object.take_staging_write_plan_internal().is_none());
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert!(object.take_staging_write_plan_internal().is_some());
}

#[test]
fn completion_is_generation_guarded_and_status_flags_track_fault_and_oos() {
    let mut object = StagingObject::new(1, "STG-1", config()).unwrap();
    write_real(&mut object, 5.0);
    let first = object.take_staging_write_plan_internal().unwrap();
    write_real(&mut object, 25.0);
    let second = object.take_staging_write_plan_internal().unwrap();
    assert!(!object.complete_staging_write_plan_internal(first.generation, false));
    assert!(object.complete_staging_write_plan_internal(second.generation, false));
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw())
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![StatusFlags::FAULT.bits() << 4],
        }
    );
    assert_protocol_error(
        object
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw()),
                None,
            )
            .unwrap_err(),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    let PropertyValue::BitString { data, .. } = object
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .unwrap()
    else {
        panic!("status flags must be a bit string");
    };
    assert_eq!(
        data[0],
        (StatusFlags::FAULT | StatusFlags::OUT_OF_SERVICE).bits() << 4
    );

    object
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw()),
            None,
        )
        .unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw())
    );
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    let plan = object.take_staging_write_plan_internal().unwrap();
    assert!(object.complete_staging_write_plan_internal(plan.generation, true));
    assert_eq!(
        object
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}

#[test]
fn metadata_and_property_list_are_truthful_and_non_intrinsic() {
    let object = StagingObject::new(1, "STG-1", config()).unwrap();
    let list = object.property_list();
    for required in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::PRESENT_STAGE,
        PropertyIdentifier::STAGES,
        PropertyIdentifier::TARGET_REFERENCES,
        PropertyIdentifier::EVENT_STATE,
        PropertyIdentifier::UNITS,
        PropertyIdentifier::PRIORITY_FOR_WRITING,
        PropertyIdentifier::MIN_PRES_VALUE,
        PropertyIdentifier::MAX_PRES_VALUE,
    ] {
        assert!(list.contains(&required), "missing {required:?}");
    }
    assert!(object.is_writable_property(PropertyIdentifier::PRESENT_VALUE));
    assert!(!object.is_writable_property(PropertyIdentifier::PRESENT_STAGE));
    assert!(!object.supports_cov());

    let mut unnamed = config();
    unnamed.stage_names = None;
    let unnamed = StagingObject::new(2, "unnamed", unnamed).unwrap();
    assert!(!unnamed
        .property_list()
        .contains(&PropertyIdentifier::STAGE_NAMES));
    assert_protocol_error(
        unnamed
            .read_property(PropertyIdentifier::STAGE_NAMES, None)
            .unwrap_err(),
        ErrorClass::PROPERTY,
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

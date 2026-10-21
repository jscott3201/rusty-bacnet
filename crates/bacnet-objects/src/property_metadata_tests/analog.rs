use super::*;

use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_types::enums::EngineeringUnits;

fn analog_objects(configuration: u8) -> [Box<dyn BACnetObject>; 3] {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let mut av = AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let mut ao = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    if configuration & 1 != 0 {
        ai.configure_fault_out_of_range(-10.0, 100.0).unwrap();
        av.configure_fault_out_of_range(-10.0, 100.0).unwrap();
    }
    macro_rules! bounds {
        ($object:ident) => {
            if configuration & 2 != 0 {
                $object.set_min_pres_value(-20.0);
            }
            if configuration & 4 != 0 {
                $object.set_max_pres_value(120.0);
            }
        };
    }
    bounds!(ai);
    bounds!(av);
    bounds!(ao);
    [Box::new(ai), Box::new(av), Box::new(ao)]
}

#[test]
fn property_metadata_analog_exact_required_and_instance_projections() {
    use PropertyIdentifier as P;

    let base_required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::OUT_OF_SERVICE,
        P::UNITS,
    ];
    // Tables 12-2, 12-3 and 12-4 require these of an object that reports
    // intrinsically; Time_Delay_Normal and Event_Message_Texts they only
    // permit (#1485).
    let intrinsic_required = [
        P::EVENT_DETECTION_ENABLE,
        P::HIGH_LIMIT,
        P::LOW_LIMIT,
        P::DEADBAND,
        P::LIMIT_ENABLE,
        P::EVENT_ENABLE,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::TIME_DELAY,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
    ];
    let intrinsic_permitted = [
        P::TIME_DELAY_NORMAL,
        P::EVENT_MESSAGE_TEXTS,
        P::EVENT_MESSAGE_TEXTS_CONFIG,
        P::EVENT_ALGORITHM_INHIBIT_REF,
        P::EVENT_ALGORITHM_INHIBIT,
    ];
    let base = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::EVENT_DETECTION_ENABLE,
        P::OUT_OF_SERVICE,
        P::UNITS,
        P::COV_INCREMENT,
        P::HIGH_LIMIT,
        P::LOW_LIMIT,
        P::DEADBAND,
        P::LIMIT_ENABLE,
        P::EVENT_ENABLE,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::TIME_DELAY,
        P::TIME_DELAY_NORMAL,
        P::RELIABILITY,
        P::RELIABILITY_EVALUATION_INHIBIT,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::EVENT_MESSAGE_TEXTS,
        P::EVENT_MESSAGE_TEXTS_CONFIG,
        P::EVENT_ALGORITHM_INHIBIT_REF,
        P::EVENT_ALGORITHM_INHIBIT,
    ];
    for configuration in 0..8 {
        for object in analog_objects(configuration) {
            let kind = object.object_identifier().object_type();
            let mut requires = base_required.to_vec();
            requires.extend(intrinsic_required);
            let mut expected = base.to_vec();
            let commandable = [
                P::PRIORITY_ARRAY,
                P::RELINQUISH_DEFAULT,
                P::CURRENT_COMMAND_PRIORITY,
            ];
            if kind != ObjectType::ANALOG_INPUT {
                expected.splice(10..10, commandable);
            }
            if kind == ObjectType::ANALOG_OUTPUT {
                requires.extend(commandable);
            }
            if kind != ObjectType::ANALOG_INPUT {
                expected.extend([P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME]);
                requires.extend([P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME]);
            }
            if configuration & 1 != 0 && kind != ObjectType::ANALOG_OUTPUT {
                expected.extend([P::FAULT_HIGH_LIMIT, P::FAULT_LOW_LIMIT]);
            }
            if configuration & 2 != 0 {
                expected.push(P::MIN_PRES_VALUE);
            }
            if configuration & 4 != 0 {
                expected.push(P::MAX_PRES_VALUE);
            }
            // Metadata order is the list's.
            let mut required: Vec<_> = expected
                .iter()
                .copied()
                .filter(|p| requires.contains(p))
                .collect();
            required.push(P::PROPERTY_LIST);
            assert_unique_and_canonical(object.as_ref());
            assert_eq!(object.required_properties().as_ref(), required);
            assert_eq!(object.property_list().as_ref(), expected);
            let metadata = object.property_metadata();
            assert_eq!(metadata.len(), expected.len() + 1);
            for row in metadata.iter() {
                let p = row.property_identifier;
                if intrinsic_required.contains(&p) {
                    assert_eq!(
                        row.presence_condition,
                        Some(PropertyPresenceCondition::IntrinsicReportingRequired),
                        "{kind:?} {p:?}"
                    );
                } else if intrinsic_permitted.contains(&p) {
                    assert_eq!(
                        row.presence_condition,
                        Some(PropertyPresenceCondition::IntrinsicReportingOptional),
                        "{kind:?} {p:?}"
                    );
                }
                assert_eq!(
                    row.conformance,
                    if kind == ObjectType::ANALOG_OUTPUT && p == P::PRESENT_VALUE {
                        PropertyConformance::RequiredWrite
                    } else if base_required.contains(&p)
                        || p == P::PROPERTY_LIST
                        || (kind == ObjectType::ANALOG_OUTPUT && commandable.contains(&p))
                    {
                        PropertyConformance::RequiredRead
                    } else {
                        PropertyConformance::Optional
                    },
                    "{kind:?} {p:?}"
                );
                if commandable.contains(&row.property_identifier) {
                    assert_eq!(
                        row.presence_condition,
                        (kind == ObjectType::ANALOG_VALUE)
                            .then_some(PropertyPresenceCondition::Commandable)
                    );
                }
                assert!(object.read_property(row.property_identifier, None).is_ok());
            }
            let wire: Vec<_> = expected
                .iter()
                .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
                .map(|p| PropertyValue::Enumerated(p.to_raw()))
                .collect();
            assert_eq!(
                object.read_property(P::PROPERTY_LIST, None).unwrap(),
                PropertyValue::List(wire.clone())
            );
            assert_eq!(
                object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
                PropertyValue::Unsigned(wire.len() as u64)
            );
            for (index, value) in wire.iter().enumerate() {
                assert_eq!(
                    object
                        .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                        .unwrap(),
                    *value
                );
            }
            for p in [
                P::FAULT_HIGH_LIMIT,
                P::FAULT_LOW_LIMIT,
                P::MIN_PRES_VALUE,
                P::MAX_PRES_VALUE,
            ] {
                assert_eq!(object.read_property(p, None).is_ok(), expected.contains(&p));
            }
        }
    }
}

#[test]
fn property_metadata_analog_write_capabilities_match_dispatch() {
    use PropertyIdentifier as P;

    for out_of_service in [false, true] {
        for mut object in analog_objects(7) {
            let commandable = object.object_identifier().object_type() != ObjectType::ANALOG_INPUT;
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let metadata = object.property_metadata().into_owned();
            for row in metadata {
                let p = row.property_identifier;
                let capability = match p {
                    P::VALUE_SOURCE => PropertyWriteCapability::WhenCommandOwner,
                    p if (commandable && crate::common::is_commandable_property_writable(p))
                        || crate::common::is_common_writable(p) =>
                    {
                        PropertyWriteCapability::Always
                    }
                    P::PRESENT_VALUE | P::RELIABILITY => PropertyWriteCapability::WhenOutOfService,
                    P::EVENT_DETECTION_ENABLE
                    | P::COV_INCREMENT
                    | P::HIGH_LIMIT
                    | P::LOW_LIMIT
                    | P::DEADBAND
                    | P::LIMIT_ENABLE
                    | P::EVENT_ENABLE
                    | P::NOTIFY_TYPE
                    | P::NOTIFICATION_CLASS
                    | P::TIME_DELAY
                    | P::TIME_DELAY_NORMAL
                    | P::EVENT_MESSAGE_TEXTS_CONFIG
                    | P::EVENT_ALGORITHM_INHIBIT_REF
                    | P::EVENT_ALGORITHM_INHIBIT
                    | P::RELIABILITY_EVALUATION_INHIBIT => PropertyWriteCapability::Always,
                    _ => PropertyWriteCapability::ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(object.is_writable_property(p), capability.is_writable());
                let index = (p == P::PRIORITY_ARRAY).then_some(8);
                let value = object.read_property(p, index).unwrap();
                let result = object.write_property_from(
                    p,
                    index,
                    value,
                    None,
                    &crate::command_source::test_origin(),
                );
                let expected = capability == PropertyWriteCapability::WhenCommandOwner
                    || capability == PropertyWriteCapability::Always
                    || (capability == PropertyWriteCapability::WhenOutOfService && out_of_service);
                assert_eq!(
                    result.is_ok(),
                    expected,
                    "{p:?}, OOS={out_of_service}: {result:?}"
                );
                if !expected {
                    assert!(matches!(result, Err(Error::Protocol { class, code })
                    if class == bacnet_types::enums::ErrorClass::PROPERTY
                        && code == bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED));
                }
            }
            if commandable {
                for p in [P::PRESENT_VALUE, P::RELINQUISH_DEFAULT] {
                    let index = (p == P::PRIORITY_ARRAY).then_some(8);
                    object
                        .write_property_from(
                            p,
                            index,
                            PropertyValue::Real(12.5),
                            Some(8),
                            &crate::command_source::test_origin(),
                        )
                        .unwrap();
                }
                assert!(
                    matches!(object.write_property(P::PRIORITY_ARRAY, None, PropertyValue::Null, None),
                Err(Error::Protocol { class, code })
                    if class == bacnet_types::enums::ErrorClass::PROPERTY
                        && code == bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED)
                );
            }
        }
    }
}

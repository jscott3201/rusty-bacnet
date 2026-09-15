use super::*;

use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};

fn analog_objects(configuration: u8) -> [Box<dyn BACnetObject>; 3] {
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
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

    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::OUT_OF_SERVICE,
        P::UNITS,
        P::PROPERTY_LIST,
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
    ];
    for configuration in 0..8 {
        for object in analog_objects(configuration) {
            let kind = object.object_identifier().object_type();
            let mut required = required.to_vec();
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
                required.splice(8..8, commandable);
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
            assert_unique_and_canonical(object.as_ref());
            assert_eq!(object.required_properties().as_ref(), required);
            assert_eq!(object.property_list().as_ref(), expected);
            let metadata = object.property_metadata();
            assert_eq!(metadata.len(), expected.len() + 1);
            for row in metadata.iter() {
                assert_eq!(
                    row.conformance,
                    if kind == ObjectType::ANALOG_OUTPUT
                        && row.property_identifier == P::PRESENT_VALUE
                    {
                        PropertyConformance::RequiredWrite
                    } else if required.contains(&row.property_identifier) {
                        PropertyConformance::RequiredRead
                    } else {
                        PropertyConformance::Optional
                    }
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
                    | P::RELIABILITY_EVALUATION_INHIBIT => PropertyWriteCapability::Always,
                    _ => PropertyWriteCapability::ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(object.is_writable_property(p), capability.is_writable());
                let index = (p == P::PRIORITY_ARRAY).then_some(8);
                let value = object.read_property(p, index).unwrap();
                let result = object.write_property(p, index, value, None);
                let expected = capability == PropertyWriteCapability::Always
                    || (capability == PropertyWriteCapability::WhenOutOfService && out_of_service);
                assert_eq!(
                    result.is_ok(),
                    expected,
                    "{p:?}, OOS={out_of_service}: {result:?}"
                );
                if !expected {
                    assert!(matches!(result, Err(Error::Protocol { class, code })
                    if class == bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
                        && code == bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32));
                }
            }
            if commandable {
                for p in [P::PRESENT_VALUE, P::PRIORITY_ARRAY, P::RELINQUISH_DEFAULT] {
                    let index = (p == P::PRIORITY_ARRAY).then_some(8);
                    object
                        .write_property(p, index, PropertyValue::Real(12.5), Some(8))
                        .unwrap();
                }
                assert!(
                    matches!(object.write_property(P::PRIORITY_ARRAY, None, PropertyValue::Null, None),
                Err(Error::Protocol { class, code })
                    if class == bacnet_types::enums::ErrorClass::PROPERTY.to_raw() as u32
                        && code == bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32)
                );
            }
        }
    }
}

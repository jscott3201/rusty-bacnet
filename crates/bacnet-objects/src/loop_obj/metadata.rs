use super::LoopObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Describe only the implemented rows, preserving their legacy order. Base
// conformance is separate from actual write capability: output remains app-owned.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SETPOINT, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPORTIONAL_CONSTANT, Optional, None, Always),
    PropertyMetadata::new(P::INTEGRAL_CONSTANT, Optional, None, Always),
    PropertyMetadata::new(P::DERIVATIVE_CONSTANT, Optional, None, Always),
    PropertyMetadata::new(P::OUTPUT_UNITS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::UPDATE_INTERVAL, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::CONTROLLED_VARIABLE_REFERENCE, RequiredRead, None, Always),
    PropertyMetadata::new(
        P::MANIPULATED_VARIABLE_REFERENCE,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(P::SETPOINT_REFERENCE, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &LoopObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

    #[test]
    fn property_metadata_loop_exact_sets_and_indexed_list() {
        let object = LoopObject::new(1, "LOOP-1", 62).unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::SETPOINT,
            P::PROPORTIONAL_CONSTANT,
            P::INTEGRAL_CONSTANT,
            P::DERIVATIVE_CONSTANT,
            P::OUTPUT_UNITS,
            P::UPDATE_INTERVAL,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::RELIABILITY,
            P::OUT_OF_SERVICE,
            P::CONTROLLED_VARIABLE_REFERENCE,
            P::MANIPULATED_VARIABLE_REFERENCE,
            P::SETPOINT_REFERENCE,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::SETPOINT,
            P::OUTPUT_UNITS,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::CONTROLLED_VARIABLE_REFERENCE,
            P::MANIPULATED_VARIABLE_REFERENCE,
            P::SETPOINT_REFERENCE,
            P::PROPERTY_LIST,
        ];
        assert!(matches!(object.property_metadata(), Cow::Borrowed(_)));
        assert_eq!(object.property_metadata().len(), all.len() + 1);
        assert_eq!(object.property_list().as_ref(), all);
        assert_eq!(object.required_properties().as_ref(), required);
        assert!(object.supports_cov());
        assert!(!object.is_createable());
        for row in object.property_metadata().iter() {
            assert_eq!(row.presence_condition, None);
            assert_eq!(
                row.conformance,
                if required.contains(&row.property_identifier) {
                    RequiredRead
                } else {
                    Optional
                }
            );
        }
        let wire: Vec<_> = all
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
            PropertyValue::Unsigned(15)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        assert_error(
            object
                .read_property(P::PROPERTY_LIST, Some(16))
                .unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_loop_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = LoopObject::new(1, "LOOP-1", 62).unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let original = object.property_metadata().into_owned();
            for row in &original {
                let p = row.property_identifier;
                let capability = match p {
                    P::SETPOINT
                    | P::PROPORTIONAL_CONSTANT
                    | P::INTEGRAL_CONSTANT
                    | P::DERIVATIVE_CONSTANT
                    | P::UPDATE_INTERVAL
                    | P::OUT_OF_SERVICE
                    | P::DESCRIPTION
                    | P::CONTROLLED_VARIABLE_REFERENCE
                    | P::MANIPULATED_VARIABLE_REFERENCE
                    | P::SETPOINT_REFERENCE => Always,
                    P::RELIABILITY => WhenOutOfService,
                    _ => ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(object.is_writable_property(p), capability.is_writable());
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if capability == Always || (capability == WhenOutOfService && out_of_service) {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            for p in [
                P::CONTROLLED_VARIABLE_REFERENCE,
                P::MANIPULATED_VARIABLE_REFERENCE,
                P::SETPOINT_REFERENCE,
            ] {
                let value = PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(
                        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap(),
                    ),
                    PropertyValue::Enumerated(P::PRESENT_VALUE.to_raw()),
                    PropertyValue::Unsigned(3),
                ]);
                object.write_property(p, None, value.clone(), None).unwrap();
                assert_eq!(object.read_property(p, None).unwrap(), value);
            }
            for p in [
                P::PRIORITY_ARRAY,
                P::RELINQUISH_DEFAULT,
                P::CURRENT_COMMAND_PRIORITY,
                P::ALL,
            ] {
                assert!(!object.is_writable_property(p));
                assert_error(
                    object.read_property(p, None).unwrap_err(),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
                assert_error(
                    object
                        .write_property(p, None, PropertyValue::Null, None)
                        .unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
            }
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }
}

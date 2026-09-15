use super::TimerObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Preserve the legacy 13-property order, then append the already-readable
// Event_State. Present_Value and Initial_Timeout keep their unconditional
// network write routes; Timer_State/Timer_Running stay application-managed
// with no behavior fix. Only Description, Present_Value, Initial_Timeout,
// and Out_Of_Service have network write routes.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::TIMER_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::TIMER_RUNNING, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::INITIAL_TIMEOUT, Optional, None, Always),
    PropertyMetadata::new(P::UPDATE_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EXPIRATION_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &TimerObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;
    use std::collections::HashSet;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_timer_exact_sets_readable_rows_and_indexed_list() {
        let object = TimerObject::new(1, "TMR-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TIMER_STATE,
            P::TIMER_RUNNING,
            P::INITIAL_TIMEOUT,
            P::UPDATE_TIME,
            P::EXPIRATION_TIME,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::EVENT_STATE,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::TIMER_STATE,
            P::TIMER_RUNNING,
            P::UPDATE_TIME,
            P::EXPIRATION_TIME,
            P::STATUS_FLAGS,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 15);
        assert_eq!(object.property_list().as_ref(), all);
        assert_eq!(object.required_properties().as_ref(), required);
        assert_eq!(
            metadata
                .iter()
                .map(|row| row.property_identifier)
                .collect::<HashSet<_>>()
                .len(),
            metadata.len()
        );
        assert!(!object.is_createable());
        assert!(object.is_deleteable());
        assert!(!object.supports_cov());
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            assert_eq!(
                row.conformance,
                if required.contains(&row.property_identifier) {
                    RequiredRead
                } else {
                    Optional
                }
            );
            object.read_property(row.property_identifier, None).unwrap();
        }
        // Event_State is readable although it was absent from the legacy list.
        assert_eq!(
            object.read_property(P::EVENT_STATE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 11);
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.clone())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(11)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [12, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_timer_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = TimerObject::new(1, "TMR-1").unwrap();
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
                    P::DESCRIPTION | P::PRESENT_VALUE | P::INITIAL_TIMEOUT | P::OUT_OF_SERVICE => {
                        Always
                    }
                    _ => ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    object.is_writable_property(p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if capability.is_writable() {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Timer_State/Timer_Running/Event_State have no network write route.
            for p in [P::TIMER_STATE, P::TIMER_RUNNING, P::EVENT_STATE] {
                let value = object.read_property(p, None).unwrap();
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!object.is_writable_property(p));
            }
            // Present_Value keeps its unconditional, enumerated write route.
            for value in [
                PropertyValue::Enumerated(0),
                PropertyValue::Enumerated(1),
                PropertyValue::Enumerated(2),
            ] {
                object
                    .write_property(P::PRESENT_VALUE, None, value.clone(), None)
                    .unwrap();
                assert_eq!(object.read_property(P::PRESENT_VALUE, None).unwrap(), value);
                assert_eq!(object.read_property(P::TIMER_STATE, None).unwrap(), value);
            }
            assert_error(
                object
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(99), None)
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_error(
                object
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(1), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            object
                .write_property(
                    P::INITIAL_TIMEOUT,
                    None,
                    PropertyValue::Unsigned(10000),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::INITIAL_TIMEOUT, None).unwrap(),
                PropertyValue::Unsigned(10000)
            );
            assert_error(
                object
                    .write_property(P::INITIAL_TIMEOUT, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            for p in [P::DESCRIPTION, P::OUT_OF_SERVICE] {
                assert_error(
                    object
                        .write_property(p, None, PropertyValue::Null, None)
                        .unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            for p in [
                P::DEFAULT_TIMEOUT,
                P::RESOLUTION,
                P::LAST_STATE_CHANGE,
                P::PRIORITY_ARRAY,
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

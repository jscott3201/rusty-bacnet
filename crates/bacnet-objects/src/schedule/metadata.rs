use super::ScheduleObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Preserve the legacy order, then append the already-readable writing priority.
// Weekly/exception schedules keep their optional base codes; both are implemented.
// The constant Event_State does not imply a complete intrinsic-reporting feature.
// Capabilities describe existing dispatch, not additional simulation write support.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SCHEDULE_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::WEEKLY_SCHEDULE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::EXCEPTION_SCHEDULE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::EFFECTIVE_PERIOD, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::PRIORITY_FOR_WRITING, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &ScheduleObject) -> Cow<'_, [PropertyMetadata]> {
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
    fn property_metadata_schedule_exact_sets_and_indexed_list() {
        let object = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(72.0)).unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::SCHEDULE_DEFAULT,
            P::WEEKLY_SCHEDULE,
            P::EXCEPTION_SCHEDULE,
            P::EFFECTIVE_PERIOD,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::RELIABILITY,
            P::OUT_OF_SERVICE,
            P::PRIORITY_FOR_WRITING,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::SCHEDULE_DEFAULT,
            P::EFFECTIVE_PERIOD,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            P::STATUS_FLAGS,
            P::RELIABILITY,
            P::OUT_OF_SERVICE,
            P::PRIORITY_FOR_WRITING,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), 16);
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
            PropertyValue::Unsigned(12)
        );
        for (index, value) in wire.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                    .unwrap(),
                *value
            );
        }
        for index in [13, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_schedule_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(72.0)).unwrap();
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
                    P::SCHEDULE_DEFAULT | P::OUT_OF_SERVICE | P::DESCRIPTION => Always,
                    P::RELIABILITY => WhenOutOfService,
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
                if capability == Always || (capability == WhenOutOfService && out_of_service) {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Schedule_Default retains its unconditional, untyped write route.
            for value in [
                PropertyValue::Null,
                PropertyValue::Boolean(true),
                PropertyValue::Real(65.0),
            ] {
                object
                    .write_property(P::SCHEDULE_DEFAULT, None, value.clone(), None)
                    .unwrap();
                assert_eq!(
                    object.read_property(P::SCHEDULE_DEFAULT, None).unwrap(),
                    value
                );
            }
            for p in [P::DESCRIPTION, P::OUT_OF_SERVICE] {
                assert_error(
                    object
                        .write_property(p, None, PropertyValue::Null, None)
                        .unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            for (value, error) in [
                (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
                (
                    PropertyValue::Enumerated(u32::MAX),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                ),
            ] {
                assert_error(
                    object
                        .write_property(P::RELIABILITY, None, value, None)
                        .unwrap_err(),
                    if out_of_service {
                        error
                    } else {
                        ErrorCode::WRITE_ACCESS_DENIED
                    },
                );
            }
            assert_eq!(
                object.read_property(P::RELIABILITY, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
            for p in [
                P::PRIORITY_ARRAY,
                P::RELINQUISH_DEFAULT,
                P::RELIABILITY_EVALUATION_INHIBIT,
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

    #[test]
    fn property_metadata_schedule_stays_stable_across_reliability_simulation() {
        use bacnet_types::enums::Reliability;

        let mut object = ScheduleObject::new(1, "SCH-1", PropertyValue::Null).unwrap();
        let original = object.property_metadata().into_owned();
        let evaluated = Reliability::CONFIGURATION_ERROR.to_raw();
        let simulated = Reliability::NO_FAULT_DETECTED.to_raw();
        object.set_reliability_internal(evaluated).unwrap();
        object
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        object
            .write_property(
                P::RELIABILITY,
                None,
                PropertyValue::Enumerated(simulated),
                None,
            )
            .unwrap();
        assert_eq!(
            object.read_property(P::RELIABILITY, None).unwrap(),
            PropertyValue::Enumerated(simulated)
        );
        assert_error(
            object.set_reliability_internal(evaluated).unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        // Repeated OOS writes must not replace the saved evaluated Reliability.
        object
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        for p in [P::WEEKLY_SCHEDULE, P::EXCEPTION_SCHEDULE] {
            assert!(object.is_array_property(p));
            for index in [0, 1, u32::MAX] {
                assert_error(
                    object
                        .write_property(p, Some(index), PropertyValue::List(vec![]), None)
                        .unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
            }
        }
        assert_eq!(object.property_metadata().as_ref(), original);
        object
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(false), None)
            .unwrap();
        assert_eq!(
            object.read_property(P::RELIABILITY, None).unwrap(),
            PropertyValue::Enumerated(evaluated)
        );
        assert_error(
            object
                .write_property(
                    P::RELIABILITY,
                    None,
                    PropertyValue::Enumerated(simulated),
                    None,
                )
                .unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(object.property_metadata().as_ref(), original);
    }
}

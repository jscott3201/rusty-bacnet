use super::TimerObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyPresenceCondition::PresentValueWritableInService,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Table 12-75, in its order. Present_Value is writable in service, so
// footnote 2 makes Timer_State and Timer_Running writable too (both keep
// the table's R), and footnote 4 requires the Min_Pres_Value and
// Max_Pres_Value range (O, with that condition). Footnote 1 makes
// Reliability writable out of service. Update_Time's footnote applies only
// with intrinsic reporting, which this Timer lacks (#2174). Initial_Timeout
// is set by each start, so a client can't write it; Default_Timeout takes
// writes within the range (#2192).
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::TIMER_STATE, RequiredRead, None, Always),
    PropertyMetadata::new(P::TIMER_RUNNING, RequiredRead, None, Always),
    PropertyMetadata::new(P::UPDATE_TIME, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LAST_STATE_CHANGE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::EXPIRATION_TIME, Optional, None, ReadOnly),
    PropertyMetadata::new(P::INITIAL_TIMEOUT, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DEFAULT_TIMEOUT, Optional, None, Always),
    PropertyMetadata::new(
        P::MIN_PRES_VALUE,
        Optional,
        Some(PresentValueWritableInService),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::MAX_PRES_VALUE,
        Optional,
        Some(PresentValueWritableInService),
        ReadOnly,
    ),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &TimerObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::HashSet;
    use crate::property_metadata::PropertyWriteCapability;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::{Error, RemoteError};
    use bacnet_types::primitives::PropertyValue;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY
                    && code == expected),
            "expected {expected:?}, got {error:?}"
        );
    }

    const ALL: [P; 18] = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::TIMER_STATE,
        P::TIMER_RUNNING,
        P::UPDATE_TIME,
        P::LAST_STATE_CHANGE,
        P::EXPIRATION_TIME,
        P::INITIAL_TIMEOUT,
        P::DEFAULT_TIMEOUT,
        P::MIN_PRES_VALUE,
        P::MAX_PRES_VALUE,
    ];

    /// Table 12-75's R rows, and the range footnote 4 requires.
    const REQUIRED: [P; 10] = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::TIMER_STATE,
        P::TIMER_RUNNING,
        P::MIN_PRES_VALUE,
        P::MAX_PRES_VALUE,
        P::PROPERTY_LIST,
    ];

    #[test]
    fn property_metadata_timer_exact_sets_readable_rows_and_indexed_list() {
        let object = TimerObject::new(1, "TMR-1").unwrap();
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), ALL.len() + 1);
        assert_eq!(object.property_list().as_ref(), ALL);
        assert_eq!(object.required_properties().as_ref(), REQUIRED);
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
            let p = row.property_identifier;
            let ranged = matches!(p, P::MIN_PRES_VALUE | P::MAX_PRES_VALUE);
            assert_eq!(
                row.presence_condition,
                ranged.then_some(PresentValueWritableInService),
                "{p:?}"
            );
            assert_eq!(
                row.conformance,
                if REQUIRED.contains(&p) && !ranged {
                    RequiredRead
                } else {
                    Optional
                },
                "{p:?}"
            );
            object.read_property(p, None).unwrap();
        }
        let wire: Vec<_> = ALL
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_eq!(wire.len(), 15);
        assert!(object.is_array_property(P::PROPERTY_LIST));
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
        for index in [16, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    /// Each row's write capability against the write dispatch: a row the
    /// metadata calls writable takes a write of a value it accepts, and every
    /// other row refuses one with WRITE_ACCESS_DENIED.
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
                    P::DESCRIPTION
                    | P::PRESENT_VALUE
                    | P::OUT_OF_SERVICE
                    | P::TIMER_STATE
                    | P::TIMER_RUNNING
                    | P::DEFAULT_TIMEOUT => Always,
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
                let allowed = match capability {
                    PropertyWriteCapability::Always => true,
                    PropertyWriteCapability::WhenOutOfService => out_of_service,
                    _ => false,
                };
                if allowed {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            for (p, wrong) in [
                (P::PRESENT_VALUE, PropertyValue::Enumerated(1)),
                (P::TIMER_RUNNING, PropertyValue::Unsigned(1)),
                (P::TIMER_STATE, PropertyValue::Unsigned(0)),
                (P::DEFAULT_TIMEOUT, PropertyValue::Null),
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    object.write_property(p, None, wrong, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            for p in [
                P::RESOLUTION,
                P::STATE_CHANGE_VALUES,
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
                    ErrorCode::UNKNOWN_PROPERTY,
                );
            }
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }
}

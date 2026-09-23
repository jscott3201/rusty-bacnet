use super::TrendLogObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::log_buffer::{BUFFER_SIZE_METADATA, LOG_BUFFER_METADATA, TOTAL_RECORD_COUNT_METADATA};
use crate::log_lifecycle::{LOG_ENABLE_METADATA, RECORD_COUNT_METADATA, STOP_WHEN_FULL_METADATA};
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Preserve legacy order and the implemented surface. The monitored-reference
// and interval rows retain their base optional classification; no new presence
// or logging-mode write gates are introduced. Out_Of_Service is a compatibility
// row, not an additional required property. Reliability stays read-only.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    LOG_ENABLE_METADATA,
    PropertyMetadata::new(P::LOG_INTERVAL, Optional, None, Always),
    STOP_WHEN_FULL_METADATA,
    BUFFER_SIZE_METADATA,
    LOG_BUFFER_METADATA,
    RECORD_COUNT_METADATA,
    TOTAL_RECORD_COUNT_METADATA,
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::LOGGING_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LOG_DEVICE_OBJECT_PROPERTY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &TrendLogObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{ClockFrame, ClockReader};
    use crate::event_log::EventLogObject;
    use crate::property_metadata::PropertyConformance::RequiredWrite;
    use crate::traits::BACnetObject;
    use crate::trend::TrendLogMultipleObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{Date, PropertyValue, Time};
    use std::sync::Arc;

    struct FixedClock;

    impl ClockReader for FixedClock {
        fn read_clock(&self) -> Option<ClockFrame> {
            Some(ClockFrame {
                local_date: Date {
                    year: 126,
                    month: 9,
                    day: 13,
                    day_of_week: 7,
                },
                local_time: Time {
                    hour: 12,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                utc_offset: 0,
                daylight_savings_status: false,
            })
        }
    }

    fn objects(capacity: u32, logging_type: u32, oos: bool) -> [Box<dyn BACnetObject>; 3] {
        let mut trend = TrendLogObject::new(1, "TL-1", capacity).unwrap();
        let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", capacity).unwrap();
        let mut event = EventLogObject::new(1, "EL-1", capacity).unwrap();
        trend.set_logging_type(logging_type);
        multiple.set_logging_type(logging_type);
        trend.out_of_service = oos;
        // Exercise both internal states without adding a network write route.
        multiple.out_of_service = oos;
        event
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(oos), None)
            .unwrap();
        [Box::new(trend), Box::new(multiple), Box::new(event)]
    }

    fn assert_error(error: Error, class: ErrorClass, code: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class: c, code: e }
            if c == class.to_raw() as u32 && e == code.to_raw() as u32),
            "{error:?}"
        );
    }

    #[test]
    fn property_metadata_log_family_exact_sets_and_indexed_list() {
        // Independent fixtures in the pre-migration property-list order.
        let base = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::LOG_ENABLE,
            P::LOG_INTERVAL,
            P::STOP_WHEN_FULL,
            P::BUFFER_SIZE,
            P::LOG_BUFFER,
            P::RECORD_COUNT,
            P::TOTAL_RECORD_COUNT,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let base_required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::LOG_ENABLE,
            P::STOP_WHEN_FULL,
            P::BUFFER_SIZE,
            P::LOG_BUFFER,
            P::RECORD_COUNT,
            P::TOTAL_RECORD_COUNT,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
        ];
        for capacity in [0, 1, 3] {
            for logging_type in [0, 1, 2] {
                for object in objects(capacity, logging_type, false) {
                    let kind = object.object_identifier().object_type();
                    let mut all = base.to_vec();
                    let mut required = base_required.to_vec();
                    if kind == ObjectType::TREND_LOG {
                        all.swap(13, 14);
                    }
                    if kind != ObjectType::EVENT_LOG {
                        all.extend([P::LOGGING_TYPE, P::LOG_DEVICE_OBJECT_PROPERTY]);
                        required.push(P::LOGGING_TYPE);
                    }
                    if kind == ObjectType::TREND_LOG_MULTIPLE {
                        required.insert(4, P::LOG_INTERVAL);
                        required.push(P::LOG_DEVICE_OBJECT_PROPERTY);
                    }
                    required.push(P::PROPERTY_LIST);
                    assert_eq!(object.property_list().as_ref(), all);
                    assert_eq!(object.required_properties().as_ref(), required);
                    assert!(!object.is_createable());
                    assert!(object.is_deleteable());
                    assert!(!object.supports_cov());
                    assert!(!object.is_array_property(P::LOG_BUFFER));
                    let metadata = object.property_metadata();
                    assert!(matches!(metadata, Cow::Borrowed(_)));
                    assert_eq!(metadata.len(), all.len() + 1);
                    for row in metadata.iter() {
                        let p = row.property_identifier;
                        assert_eq!(row.presence_condition, None);
                        let conformance = if matches!(p, P::LOG_ENABLE | P::RECORD_COUNT) {
                            RequiredWrite
                        } else if required.contains(&p) {
                            RequiredRead
                        } else {
                            Optional
                        };
                        assert_eq!(row.conformance, conformance, "{kind:?} {p:?}");
                        assert!(object.read_property(p, None).is_ok(), "{kind:?} {p:?}");
                    }
                    let wire: Vec<_> = all
                        .iter()
                        .filter(|&&p| {
                            !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE)
                        })
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
                    for (i, value) in wire.iter().enumerate() {
                        assert_eq!(
                            object
                                .read_property(P::PROPERTY_LIST, Some(i as u32 + 1))
                                .unwrap(),
                            *value
                        );
                    }
                    assert_error(
                        object
                            .read_property(P::PROPERTY_LIST, Some(wire.len() as u32 + 1))
                            .unwrap_err(),
                        ErrorClass::PROPERTY,
                        ErrorCode::INVALID_ARRAY_INDEX,
                    );
                    assert_eq!(
                        object.read_property(P::LOG_BUFFER, None).unwrap(),
                        PropertyValue::List(vec![])
                    );
                }
            }
        }
    }

    #[test]
    fn property_metadata_log_family_write_capabilities_match_dispatch() {
        for oos in [false, true] {
            for logging_type in [0, 1, 2] {
                for mut object in objects(8, logging_type, oos) {
                    object.bind_clock_internal(Some(Arc::new(FixedClock)));
                    let kind = object.object_identifier().object_type();
                    let metadata = object.property_metadata().into_owned();
                    for row in &metadata {
                        let p = row.property_identifier;
                        let capability = match p {
                            P::LOG_ENABLE
                            | P::LOG_INTERVAL
                            | P::STOP_WHEN_FULL
                            | P::RECORD_COUNT
                            | P::DESCRIPTION => Always,
                            P::OUT_OF_SERVICE if kind != ObjectType::TREND_LOG_MULTIPLE => Always,
                            _ => ReadOnly,
                        };
                        assert_eq!(row.write_capability, capability, "{kind:?} {p:?}");
                        assert_eq!(object.is_writable_property(p), capability.is_writable());
                        let value = match p {
                            P::LOG_ENABLE => PropertyValue::Boolean(false),
                            P::LOG_INTERVAL => PropertyValue::Unsigned(17),
                            P::STOP_WHEN_FULL => PropertyValue::Boolean(true),
                            P::RECORD_COUNT => PropertyValue::Unsigned(0),
                            _ => object.read_property(p, None).unwrap(),
                        };
                        let result = object.write_property(p, None, value, None);
                        if capability == Always {
                            result.unwrap();
                        } else {
                            assert_error(
                                result.unwrap_err(),
                                ErrorClass::PROPERTY,
                                ErrorCode::WRITE_ACCESS_DENIED,
                            );
                        }
                        let before = object.read_property(p, None).unwrap();
                        if capability == Always && matches!(p, P::DESCRIPTION | P::OUT_OF_SERVICE) {
                            object
                                .write_property(p, None, PropertyValue::Null, None)
                                .unwrap();
                            assert_eq!(object.read_property(p, None).unwrap(), before);
                            assert_error(
                                object
                                    .write_property(p, None, PropertyValue::Unsigned(1), None)
                                    .unwrap_err(),
                                ErrorClass::PROPERTY,
                                ErrorCode::INVALID_DATA_TYPE,
                            );
                            continue;
                        }
                        assert_error(
                            object
                                .write_property(p, None, PropertyValue::Null, None)
                                .unwrap_err(),
                            ErrorClass::PROPERTY,
                            if capability == Always {
                                ErrorCode::INVALID_DATA_TYPE
                            } else {
                                ErrorCode::WRITE_ACCESS_DENIED
                            },
                        );
                    }
                    assert_error(
                        object
                            .write_property(P::RECORD_COUNT, None, PropertyValue::Unsigned(1), None)
                            .unwrap_err(),
                        ErrorClass::PROPERTY,
                        ErrorCode::INVALID_DATA_TYPE,
                    );
                    for p in [
                        P::PRESENT_VALUE,
                        P::PRIORITY_ARRAY,
                        P::START_TIME,
                        P::STOP_TIME,
                        P::ALL,
                    ] {
                        assert!(!object.is_writable_property(p));
                        assert_error(
                            object.read_property(p, None).unwrap_err(),
                            ErrorClass::PROPERTY,
                            ErrorCode::UNKNOWN_PROPERTY,
                        );
                        assert_error(
                            object
                                .write_property(p, None, PropertyValue::Null, None)
                                .unwrap_err(),
                            ErrorClass::PROPERTY,
                            ErrorCode::WRITE_ACCESS_DENIED,
                        );
                    }
                    assert_eq!(object.property_metadata().as_ref(), metadata);
                }
            }
        }
    }

    #[test]
    fn property_metadata_log_capability_does_not_bypass_clock_validation() {
        for mut object in objects(3, 0, false) {
            let metadata = object.property_metadata().into_owned();
            for (p, value) in [
                (P::LOG_ENABLE, PropertyValue::Boolean(false)),
                (P::RECORD_COUNT, PropertyValue::Unsigned(0)),
            ] {
                assert!(object.is_writable_property(p));
                assert_error(
                    object.write_property(p, None, value, None).unwrap_err(),
                    ErrorClass::DEVICE,
                    ErrorCode::OPERATIONAL_PROBLEM,
                );
                assert_eq!(
                    object.read_property(P::LOG_ENABLE, None).unwrap(),
                    PropertyValue::Boolean(true)
                );
                assert_eq!(
                    object.read_property(P::LOG_BUFFER, None).unwrap(),
                    PropertyValue::List(vec![])
                );
                assert_eq!(object.property_metadata().as_ref(), metadata);
            }
        }
    }
}

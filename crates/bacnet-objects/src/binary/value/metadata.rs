use super::BinaryValueObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyPresenceCondition::{Commandable, IntrinsicReporting, PairedText},
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Base conformance is independent of implemented writability. Commandable and
// intrinsic rows retain their optional base code. Preserve legacy list order.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::EVENT_DETECTION_ENABLE,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::EVENT_ENABLE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::TIME_DELAY, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::NOTIFY_TYPE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::NOTIFICATION_CLASS,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(
        P::ACKED_TRANSITIONS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_TIME_STAMPS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
    PropertyMetadata::new(
        P::CURRENT_COMMAND_PRIORITY,
        Optional,
        Some(Commandable),
        ReadOnly,
    ),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::RELIABILITY_EVALUATION_INHIBIT, Optional, None, Always),
    PropertyMetadata::new(P::ACTIVE_TEXT, Optional, Some(PairedText), Always),
    PropertyMetadata::new(P::INACTIVE_TEXT, Optional, Some(PairedText), Always),
    PropertyMetadata::new(P::ALARM_VALUE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(object: &BinaryValueObject) -> Cow<'_, [PropertyMetadata]> {
    // Optional Audit rows belong to this particular value instance.
    let mut rows = Cow::Borrowed(BASE);
    if object.audit_policy != crate::audit::ObjectAuditPolicy::default() {
        rows.to_mut().extend(object.audit_policy.metadata());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary::BinaryOutputObject;
    use crate::property_metadata::PropertyConformance::RequiredWrite;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    fn objects() -> [Box<dyn BACnetObject>; 2] {
        [
            Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
            Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
        ]
    }

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_binary_legacy_order_and_indexed_list() {
        // Independent legacy-list fixture shared by the two commandable types.
        let base = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::EVENT_DETECTION_ENABLE,
            P::EVENT_ENABLE,
            P::TIME_DELAY,
            P::TIME_DELAY_NORMAL,
            P::NOTIFY_TYPE,
            P::NOTIFICATION_CLASS,
            P::ACKED_TRANSITIONS,
            P::EVENT_TIME_STAMPS,
            P::EVENT_MESSAGE_TEXTS,
            P::OUT_OF_SERVICE,
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
            P::CURRENT_COMMAND_PRIORITY,
            P::RELIABILITY,
            P::RELIABILITY_EVALUATION_INHIBIT,
            P::ACTIVE_TEXT,
            P::INACTIVE_TEXT,
        ];
        for mut object in objects() {
            let output = object.object_identifier().object_type() == ObjectType::BINARY_OUTPUT;
            let mut expected = base.to_vec();
            if output {
                expected.insert(20, P::POLARITY);
                expected.insert(5, P::FEEDBACK_VALUE);
            } else {
                expected.push(P::ALARM_VALUE);
            }
            let original = object.property_metadata().into_owned();
            for enabled in [false, true] {
                object
                    .write_property(
                        P::EVENT_DETECTION_ENABLE,
                        None,
                        PropertyValue::Boolean(enabled),
                        None,
                    )
                    .unwrap();
                assert_eq!(object.property_metadata().as_ref(), original);
                assert_eq!(object.property_list().as_ref(), expected);
                assert_eq!(original.len(), expected.len() + 1);
                let required = object.required_properties();
                for row in &original {
                    let p = row.property_identifier;
                    let conformance = if output && p == P::PRESENT_VALUE {
                        RequiredWrite
                    } else if required.contains(&p) {
                        RequiredRead
                    } else {
                        Optional
                    };
                    assert_eq!(row.conformance, conformance, "{p:?}");
                    let condition = match p {
                        P::PRIORITY_ARRAY | P::RELINQUISH_DEFAULT | P::CURRENT_COMMAND_PRIORITY => {
                            (!output).then_some(Commandable)
                        }
                        P::ACTIVE_TEXT | P::INACTIVE_TEXT => Some(PairedText),
                        P::EVENT_DETECTION_ENABLE
                        | P::EVENT_ENABLE
                        | P::TIME_DELAY
                        | P::TIME_DELAY_NORMAL
                        | P::NOTIFY_TYPE
                        | P::NOTIFICATION_CLASS
                        | P::ACKED_TRANSITIONS
                        | P::EVENT_TIME_STAMPS
                        | P::EVENT_MESSAGE_TEXTS
                        | P::ALARM_VALUE
                        | P::FEEDBACK_VALUE => Some(IntrinsicReporting),
                        _ => None,
                    };
                    assert_eq!(row.presence_condition, condition, "{p:?}");
                    assert!(object.read_property(p, None).is_ok(), "{p:?}");
                }
                let wire: Vec<_> = expected
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
                        .read_property(P::PROPERTY_LIST, Some(wire.len() as u32 + 1))
                        .unwrap_err(),
                    ErrorCode::INVALID_ARRAY_INDEX,
                );
                for p in [
                    P::MIN_PRES_VALUE,
                    P::MAX_PRES_VALUE,
                    P::FAULT_LOW_LIMIT,
                    P::FAULT_HIGH_LIMIT,
                ] {
                    assert!(!expected.contains(&p));
                    assert_error(
                        object.read_property(p, None).unwrap_err(),
                        ErrorCode::UNKNOWN_PROPERTY,
                    );
                }
            }
        }
    }

    #[test]
    fn property_metadata_binary_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            for mut object in objects() {
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
                        P::OBJECT_NAME
                        | P::DESCRIPTION
                        | P::PRESENT_VALUE
                        | P::OUT_OF_SERVICE
                        | P::PRIORITY_ARRAY
                        | P::RELINQUISH_DEFAULT
                        | P::EVENT_DETECTION_ENABLE
                        | P::EVENT_ENABLE
                        | P::TIME_DELAY
                        | P::TIME_DELAY_NORMAL
                        | P::NOTIFY_TYPE
                        | P::NOTIFICATION_CLASS
                        | P::RELIABILITY_EVALUATION_INHIBIT
                        | P::ACTIVE_TEXT
                        | P::INACTIVE_TEXT
                        | P::ALARM_VALUE
                        | P::FEEDBACK_VALUE => Always,
                        P::RELIABILITY => WhenOutOfService,
                        _ => ReadOnly,
                    };
                    assert_eq!(row.write_capability, capability, "{p:?}");
                    assert_eq!(
                        object.is_writable_property(p),
                        capability.is_writable(),
                        "{p:?}"
                    );
                    let index = (p == P::PRIORITY_ARRAY).then_some(8);
                    let value = object.read_property(p, index).unwrap();
                    let result = object.write_property(p, index, value, None);
                    if capability == Always || (capability == WhenOutOfService && out_of_service) {
                        result.unwrap();
                    } else {
                        assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                    }
                }
                for p in [P::PRESENT_VALUE, P::PRIORITY_ARRAY, P::RELINQUISH_DEFAULT] {
                    let index = (p == P::PRIORITY_ARRAY).then_some(8);
                    let before = object.read_property(p, index).unwrap();
                    assert_error(
                        object
                            .write_property(p, index, PropertyValue::Enumerated(2), Some(8))
                            .unwrap_err(),
                        ErrorCode::VALUE_OUT_OF_RANGE,
                    );
                    assert_eq!(object.read_property(p, index).unwrap(), before);
                    object
                        .write_property(p, index, PropertyValue::Enumerated(1), Some(8))
                        .unwrap();
                    assert_eq!(
                        object.read_property(P::PRESENT_VALUE, None).unwrap(),
                        PropertyValue::Enumerated(1)
                    );
                }
                assert_error(
                    object
                        .write_property(P::PRIORITY_ARRAY, None, PropertyValue::Null, None)
                        .unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                // Relinquish both written slots: the configured default takes over.
                object
                    .write_property(P::PRESENT_VALUE, None, PropertyValue::Null, Some(8))
                    .unwrap();
                object
                    .write_property(P::PRIORITY_ARRAY, Some(16), PropertyValue::Null, None)
                    .unwrap();
                object
                    .write_property(
                        P::RELINQUISH_DEFAULT,
                        None,
                        PropertyValue::Enumerated(0),
                        None,
                    )
                    .unwrap();
                assert_eq!(
                    object.read_property(P::PRESENT_VALUE, None).unwrap(),
                    PropertyValue::Enumerated(0)
                );
            }
        }
    }
}

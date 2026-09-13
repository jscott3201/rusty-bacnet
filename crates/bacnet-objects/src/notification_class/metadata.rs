use super::NotificationClass;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Preserve the implemented rows and their legacy order. The compatibility
// status rows are optional; base conformance does not imply a write route.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::NOTIFICATION_CLASS, RequiredRead, None, Always),
    PropertyMetadata::new(P::PRIORITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACK_REQUIRED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RECIPIENT_LIST, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &NotificationClass) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    const ALL: &[P] = &[
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::NOTIFICATION_CLASS,
        P::PRIORITY,
        P::ACK_REQUIRED,
        P::RECIPIENT_LIST,
    ];
    const REQUIRED: &[P] = &[
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::NOTIFICATION_CLASS,
        P::PRIORITY,
        P::ACK_REQUIRED,
        P::RECIPIENT_LIST,
        P::PROPERTY_LIST,
    ];

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_notification_class_exact_sets_and_indexed_list() {
        let object = NotificationClass::new(7, "NC-7").unwrap();
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        let expected: Vec<_> = ALL.iter().copied().chain([P::PROPERTY_LIST]).collect();
        assert_eq!(
            metadata
                .iter()
                .map(|row| row.property_identifier)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(object.property_list().as_ref(), ALL);
        assert_eq!(object.required_properties().as_ref(), REQUIRED);
        assert!(!object.supports_cov());
        assert!(!object.is_createable());
        assert!(object.is_deleteable());
        for row in metadata.iter() {
            assert_eq!(row.presence_condition, None);
            assert_eq!(
                row.conformance,
                if REQUIRED.contains(&row.property_identifier) {
                    RequiredRead
                } else {
                    Optional
                }
            );
        }
        let wire = [
            P::DESCRIPTION,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::NOTIFICATION_CLASS,
            P::PRIORITY,
            P::ACK_REQUIRED,
            P::RECIPIENT_LIST,
        ]
        .map(|p| PropertyValue::Enumerated(p.to_raw()));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.to_vec())
        );
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
            PropertyValue::Unsigned(9)
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
                .read_property(P::PROPERTY_LIST, Some(10))
                .unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }

    #[test]
    fn property_metadata_notification_class_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = NotificationClass::new(7, "NC-7").unwrap();
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
                    | P::OUT_OF_SERVICE
                    | P::NOTIFICATION_CLASS
                    | P::RECIPIENT_LIST => Always,
                    _ => ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(object.is_writable_property(p), capability.is_writable());
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if capability == Always {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            object
                .write_property(
                    P::NOTIFICATION_CLASS,
                    None,
                    PropertyValue::Unsigned(99),
                    None,
                )
                .unwrap();
            object.set_description("configured class");
            object.priority = [12, 34, 56];
            object.ack_required = [true, false, true];
            assert_eq!(object.object_identifier().instance_number(), 7);
            assert_eq!(
                object.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
                PropertyValue::Unsigned(99)
            );
            assert!(object.is_array_property(P::PRIORITY));
            assert!(!object.is_array_property(P::ACK_REQUIRED));
            assert!(!object.is_array_property(P::RECIPIENT_LIST));
            assert_error(
                object
                    .write_property(
                        P::RECIPIENT_LIST,
                        Some(0),
                        PropertyValue::ApplicationData(vec![]),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            );
            for p in [P::PRESENT_VALUE, P::PRIORITY_ARRAY, P::ALL] {
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
            assert_eq!(object.property_list().as_ref(), ALL);
            assert_eq!(object.required_properties().as_ref(), REQUIRED);
        }
    }
}

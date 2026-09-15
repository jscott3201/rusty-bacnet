use super::{LifeSafetyPointObject, LifeSafetyZoneObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for Life Safety Point (type 21, Clause 12.15
// Table 12-18) and Life Safety Zone (type 22, Clause 12.16 Table 12-19).
// Order preserves the legacy property_list projection; PROPERTY_LIST is
// appended so the projection helper omits it while required_properties keeps
// it. Only implemented rows are described: table rows the objects do not
// serve (Accepted_Modes, the Zone Tracking_Value and Member_Of, Device_Type,
// Units, Setting, intrinsic-reporting/event rows, Reliability_Evaluation_Inhibit,
// Value_Source/audit/tags/profile rows) stay absent until dispatch exists.
// Mode carries the table W code. Tracking_Value and Reliability carry the
// table R1 OOS-writable footnote, but dispatch currently has no write arm for
// either property, so both rows mirror dispatch as ReadOnly rather than
// advertising a route write_property rejects.
const POINT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MODE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::SILENCED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OPERATION_EXPECTED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::TRACKING_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MEMBER_OF, Optional, None, ReadOnly),
    PropertyMetadata::new(P::DIRECT_READING, Optional, None, Always),
    PropertyMetadata::new(P::MAINTENANCE_REQUIRED, Optional, None, Always),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ZONE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MODE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::SILENCED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OPERATION_EXPECTED, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ZONE_MEMBERS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_point(_object: &LifeSafetyPointObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(POINT_BASE)
}

pub(super) fn for_zone(_object: &LifeSafetyZoneObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ZONE_BASE)
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

    fn assert_conformance(metadata: &[PropertyMetadata], required: &[P]) {
        for row in metadata {
            let expected = if row.property_identifier == P::MODE {
                RequiredWrite
            } else if required.contains(&row.property_identifier) {
                RequiredRead
            } else {
                Optional
            };
            assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
            assert_eq!(
                row.presence_condition, None,
                "{:?}",
                row.property_identifier
            );
        }
    }

    fn assert_indexed_property_list(object: &dyn BACnetObject, wire: &[PropertyValue]) {
        assert!(object.is_array_property(P::PROPERTY_LIST));
        assert_eq!(
            object.read_property(P::PROPERTY_LIST, None).unwrap(),
            PropertyValue::List(wire.to_vec())
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
        for index in [wire.len() as u32 + 1, u32::MAX] {
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(index))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }

    #[test]
    fn property_metadata_point_exact_sets_and_indexed_list() {
        let object = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::DESCRIPTION,
            P::PRESENT_VALUE,
            P::MODE,
            P::SILENCED,
            P::OPERATION_EXPECTED,
            P::TRACKING_VALUE,
            P::MEMBER_OF,
            P::DIRECT_READING,
            P::MAINTENANCE_REQUIRED,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::MODE,
            P::SILENCED,
            P::OPERATION_EXPECTED,
            P::TRACKING_VALUE,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), all.len() + 1);
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
        assert!(object.supports_cov());
        assert_conformance(&metadata, &required);
        for row in metadata.iter() {
            object.read_property(row.property_identifier, None).unwrap();
        }
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_indexed_property_list(&object, &wire);
    }

    #[test]
    fn property_metadata_zone_exact_sets_and_indexed_list() {
        let object = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::DESCRIPTION,
            P::PRESENT_VALUE,
            P::MODE,
            P::SILENCED,
            P::OPERATION_EXPECTED,
            P::ZONE_MEMBERS,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::MODE,
            P::SILENCED,
            P::OPERATION_EXPECTED,
            P::ZONE_MEMBERS,
            P::EVENT_STATE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        let metadata = object.property_metadata();
        assert!(matches!(metadata, Cow::Borrowed(_)));
        assert_eq!(metadata.len(), all.len() + 1);
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
        assert!(object.supports_cov());
        assert_conformance(&metadata, &required);
        for row in metadata.iter() {
            object.read_property(row.property_identifier, None).unwrap();
        }
        let wire: Vec<_> = all
            .iter()
            .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
            .map(|p| PropertyValue::Enumerated(p.to_raw()))
            .collect();
        assert_indexed_property_list(&object, &wire);
    }

    #[test]
    fn property_metadata_point_write_capabilities_match_dispatch() {
        let mut object = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
        object.set_description("original");
        let metadata = object.property_metadata().into_owned();
        for row in &metadata {
            let p = row.property_identifier;
            let writable = matches!(
                p,
                P::MODE
                    | P::DIRECT_READING
                    | P::MAINTENANCE_REQUIRED
                    | P::DESCRIPTION
                    | P::OUT_OF_SERVICE
            );
            assert_eq!(
                row.write_capability,
                if writable { Always } else { ReadOnly },
                "{p:?}"
            );
            assert_eq!(object.is_writable_property(p), writable, "{p:?}");
            let before = object.read_property(p, None).unwrap();
            let result = object.write_property(p, None, before.clone(), None);
            if writable {
                result.unwrap();
            } else {
                assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert_eq!(object.read_property(p, None).unwrap(), before);
        }
        for (property, value, error) in [
            (
                P::MODE,
                PropertyValue::Real(1.0),
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                P::DIRECT_READING,
                PropertyValue::Null,
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                P::DIRECT_READING,
                PropertyValue::Real(f32::INFINITY),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                P::MAINTENANCE_REQUIRED,
                PropertyValue::Null,
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                P::DESCRIPTION,
                PropertyValue::Null,
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                P::OUT_OF_SERVICE,
                PropertyValue::Null,
                ErrorCode::INVALID_DATA_TYPE,
            ),
        ] {
            assert_error(
                object
                    .write_property(property, None, value, None)
                    .unwrap_err(),
                error,
            );
        }
        for p in [P::ACCEPTED_MODES, P::RELIABILITY_EVALUATION_INHIBIT, P::ALL] {
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
        assert_eq!(object.property_metadata().as_ref(), metadata);
    }

    #[test]
    fn property_metadata_zone_write_capabilities_match_dispatch() {
        let mut object = LifeSafetyZoneObject::new(1, "LSZ-1").unwrap();
        object.set_description("original");
        let metadata = object.property_metadata().into_owned();
        for row in &metadata {
            let p = row.property_identifier;
            let writable = matches!(p, P::MODE | P::DESCRIPTION | P::OUT_OF_SERVICE);
            assert_eq!(
                row.write_capability,
                if writable { Always } else { ReadOnly },
                "{p:?}"
            );
            assert_eq!(object.is_writable_property(p), writable, "{p:?}");
            let before = object.read_property(p, None).unwrap();
            let result = object.write_property(p, None, before.clone(), None);
            if writable {
                result.unwrap();
            } else {
                assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert_eq!(object.read_property(p, None).unwrap(), before);
        }
        for (property, value, error) in [
            (
                P::MODE,
                PropertyValue::Real(1.0),
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                P::DESCRIPTION,
                PropertyValue::Null,
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                P::OUT_OF_SERVICE,
                PropertyValue::Null,
                ErrorCode::INVALID_DATA_TYPE,
            ),
        ] {
            assert_error(
                object
                    .write_property(property, None, value, None)
                    .unwrap_err(),
                error,
            );
        }
        for p in [P::TRACKING_VALUE, P::MEMBER_OF, P::ACCEPTED_MODES, P::ALL] {
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
        assert_eq!(object.property_metadata().as_ref(), metadata);
    }
}

use super::ProgramObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Preserve the implemented surface and legacy order. Program_State's existing
// write route is deliberately described, not changed into a lifecycle guard.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROGRAM_STATE, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROGRAM_CHANGE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::REASON_FOR_HALT, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &ProgramObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_program_exact_sets_and_indexed_list() {
        let object = ProgramObject::new(1, "PRG-1").unwrap();
        let all = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::DESCRIPTION,
            P::OBJECT_TYPE,
            P::PROGRAM_STATE,
            P::PROGRAM_CHANGE,
            P::REASON_FOR_HALT,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PROGRAM_STATE,
            P::PROGRAM_CHANGE,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::PROPERTY_LIST,
        ];
        assert!(matches!(object.property_metadata(), Cow::Borrowed(_)));
        assert_eq!(object.property_metadata().len(), all.len() + 1);
        assert_eq!(object.property_list().as_ref(), all);
        assert_eq!(object.required_properties().as_ref(), required);
        assert!(!object.is_createable());
        assert!(!object.supports_cov());
        for row in object.property_metadata().iter() {
            assert_eq!(row.presence_condition, None);
            let p = row.property_identifier;
            assert_eq!(
                row.conformance,
                if p == P::PROGRAM_CHANGE {
                    RequiredWrite
                } else if required.contains(&p) {
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
            PropertyValue::Unsigned(7)
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
            object.read_property(P::PROPERTY_LIST, Some(8)).unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }

    #[test]
    fn property_metadata_program_write_capabilities_match_dispatch() {
        for out_of_service in [false, true] {
            let mut object = ProgramObject::new(1, "PRG-1").unwrap();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            for row in object.property_metadata().into_owned() {
                let p = row.property_identifier;
                let capability = match p {
                    P::PROGRAM_CHANGE | P::PROGRAM_STATE | P::DESCRIPTION | P::OUT_OF_SERVICE => {
                        Always
                    }
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
            for p in [
                P::PRESENT_VALUE,
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
        }
    }

    #[test]
    fn property_metadata_program_preserves_state_and_request_compatibility() {
        let mut object = ProgramObject::new(1, "PRG-1").unwrap();
        let metadata = object.property_metadata().into_owned();
        // No transition validation is introduced: every in-range state is still
        // writable from every other state, in or out of service.
        for out_of_service in [false, true] {
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            for initial in 0..=5 {
                for next in 0..=5 {
                    object.set_program_state(initial);
                    object
                        .write_property(
                            P::PROGRAM_STATE,
                            None,
                            PropertyValue::Enumerated(next),
                            None,
                        )
                        .unwrap();
                    assert_eq!(
                        object.read_property(P::PROGRAM_STATE, None).unwrap(),
                        PropertyValue::Enumerated(next)
                    );
                }
            }
            for invalid in [6, u32::MAX] {
                assert_error(
                    object
                        .write_property(
                            P::PROGRAM_STATE,
                            None,
                            PropertyValue::Enumerated(invalid),
                            None,
                        )
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
                assert_eq!(
                    object.read_property(P::PROGRAM_STATE, None).unwrap(),
                    PropertyValue::Enumerated(5)
                );
            }
            assert_error(
                object
                    .write_property(P::PROGRAM_STATE, None, PropertyValue::Unsigned(0), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_eq!(
                object.read_property(P::PROGRAM_STATE, None).unwrap(),
                PropertyValue::Enumerated(5)
            );
            // Requests retain their unrestricted enum route and do not execute
            // a program or implicitly change Program_State.
            for request in [1, 5, 6, u32::MAX, 0] {
                object
                    .write_property(
                        P::PROGRAM_CHANGE,
                        None,
                        PropertyValue::Enumerated(request),
                        None,
                    )
                    .unwrap();
                assert_eq!(
                    object.read_property(P::PROGRAM_CHANGE, None).unwrap(),
                    PropertyValue::Enumerated(request)
                );
                assert_eq!(
                    object.read_property(P::PROGRAM_STATE, None).unwrap(),
                    PropertyValue::Enumerated(5)
                );
            }
            assert_error(
                object
                    .write_property(P::PROGRAM_CHANGE, None, PropertyValue::Unsigned(1), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_eq!(
                object.read_property(P::PROGRAM_CHANGE, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
        }
        object.set_program_state(u32::MAX);
        assert_eq!(
            object.read_property(P::PROGRAM_STATE, None).unwrap(),
            PropertyValue::Enumerated(u32::MAX)
        );
        assert_eq!(object.property_metadata().as_ref(), metadata);
    }
}

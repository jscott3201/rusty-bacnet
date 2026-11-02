use super::ProgramObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Preserve the implemented surface and legacy order. Program_State is
// read-only (Clause 12.22.4): clients request changes through Program_Change,
// and the application reports the state with `set_program_state` (#2258).
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROGRAM_STATE, RequiredRead, None, ReadOnly),
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
    use bacnet_types::error::{Error, RemoteError};
    use bacnet_types::primitives::PropertyValue;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY && code == expected),
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
        assert_eq!(
            crate::test_rows::listed(&object.property_metadata()),
            crate::test_rows::listed_from(&all)
        );
        assert_eq!(
            crate::test_rows::required(&object.property_metadata()),
            required
        );
        assert!(!object.cov().is_some_and(|cov| cov.supports_subscribe_cov()));
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
                    P::PROGRAM_CHANGE | P::DESCRIPTION | P::OUT_OF_SERVICE => Always,
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
                    ErrorCode::UNKNOWN_PROPERTY,
                );
            }
        }
    }

    #[test]
    fn property_metadata_program_state_is_read_only_and_requests_do_not_change_it() {
        let mut object = ProgramObject::new(1, "PRG-1").unwrap();
        let metadata = object.property_metadata().into_owned();
        for out_of_service in [false, true] {
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // The application reports any state; a network write of any
            // value, in range or not, is refused and changes nothing.
            for state in 0..=5 {
                object.set_program_state(state);
                for value in [
                    PropertyValue::Enumerated((state + 1) % 6),
                    PropertyValue::Enumerated(6),
                    PropertyValue::Unsigned(0),
                ] {
                    assert_error(
                        object
                            .write_property(P::PROGRAM_STATE, None, value, None)
                            .unwrap_err(),
                        ErrorCode::WRITE_ACCESS_DENIED,
                    );
                    assert_eq!(
                        object.read_property(P::PROGRAM_STATE, None).unwrap(),
                        PropertyValue::Enumerated(state)
                    );
                }
            }
            // Requests keep their unrestricted enum route and do not execute
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

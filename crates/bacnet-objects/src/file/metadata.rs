use super::{resize, FileObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Preserve the readable surface and legacy order. Record_Count retains its
// optional base code and is included only when the object's read route has it.
// The implemented status properties are optional, not additional required rows.
const BASE: [PropertyMetadata; 15] = [
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::FILE_TYPE, RequiredRead, None, Always),
    PropertyMetadata::new(P::FILE_SIZE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MODIFICATION_DATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ARCHIVE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::READ_ONLY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::FILE_ACCESS_METHOD, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RECORD_COUNT, Optional, None, ReadOnly),
];

const FILE_SIZE_ROW: usize = 5;
const RECORD_COUNT_ROW: usize = BASE.len() - 1;

const fn with_resize(index: usize) -> [PropertyMetadata; BASE.len()] {
    let mut rows = BASE;
    rows[index].write_capability = Always;
    rows
}

const STREAM_WRITABLE: [PropertyMetadata; BASE.len()] = with_resize(FILE_SIZE_ROW);
const RECORD_WRITABLE: [PropertyMetadata; BASE.len()] = with_resize(RECORD_COUNT_ROW);

pub(super) fn for_object(object: &FileObject) -> Cow<'_, [PropertyMetadata]> {
    // Use the retained resize authority, including its exact access-method and
    // Read_Only gates. No payload, cap, clock, or snapshot behavior lives here.
    let rows = if resize::is_writable(object, P::FILE_SIZE) {
        &STREAM_WRITABLE
    } else if resize::is_writable(object, P::RECORD_COUNT) {
        &RECORD_WRITABLE
    } else {
        &BASE
    };
    let len = if object.record_count.is_some() {
        BASE.len()
    } else {
        RECORD_COUNT_ROW
    };
    Cow::Borrowed(&rows[..len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::BACnetObject;
    use bacnet_types::enums::{ErrorClass, ErrorCode, FileAccessMethod};
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;
    use std::collections::HashSet;

    fn assert_error(error: Error, expected: ErrorCode) {
        assert!(
            matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32),
            "expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn property_metadata_file_exact_sets_readable_rows_and_indexed_list() {
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::FILE_TYPE,
            P::FILE_SIZE,
            P::MODIFICATION_DATE,
            P::ARCHIVE,
            P::READ_ONLY,
            P::FILE_ACCESS_METHOD,
            P::PROPERTY_LIST,
        ];
        let base = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::DESCRIPTION,
            P::FILE_TYPE,
            P::FILE_SIZE,
            P::MODIFICATION_DATE,
            P::ARCHIVE,
            P::READ_ONLY,
            P::FILE_ACCESS_METHOD,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let mut object = FileObject::new(1, "FILE-1", "raw").unwrap();
        object.set_data(vec![1, 2, 3]);
        object.set_records(vec![vec![4, 5], vec![6]]);
        // Reconfigure the same instance to detect stale presence/capabilities.
        for method in [1, 0, u32::MAX, 0, 1] {
            object.set_file_access_method(method);
            for read_only in [false, true, false] {
                object.set_read_only(read_only);
                let mut all = base.to_vec();
                if method == FileAccessMethod::RECORD_ACCESS.to_raw() {
                    all.push(P::RECORD_COUNT);
                }
                let metadata = object.property_metadata();
                assert!(matches!(metadata, Cow::Borrowed(_)));
                assert_eq!(metadata.len(), all.len() + 1);
                let unique: HashSet<_> =
                    metadata.iter().map(|row| row.property_identifier).collect();
                assert_eq!(unique.len(), metadata.len());
                assert_eq!(object.property_list().as_ref(), all);
                assert_eq!(object.required_properties().as_ref(), required);
                assert!(!object.is_createable());
                assert!(object.is_deleteable());
                assert!(!object.supports_cov());
                for row in metadata.iter() {
                    let p = row.property_identifier;
                    assert_eq!(row.presence_condition, None);
                    assert_eq!(
                        row.conformance,
                        if p == P::ARCHIVE {
                            RequiredWrite
                        } else if required.contains(&p) {
                            RequiredRead
                        } else {
                            Optional
                        }
                    );
                    assert!(object.read_property(p, None).is_ok(), "{p:?}");
                }
                let wire: Vec<_> = all[3..]
                    .iter()
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
                if !all.contains(&P::RECORD_COUNT) {
                    assert_error(
                        object.read_property(P::RECORD_COUNT, None).unwrap_err(),
                        ErrorCode::UNKNOWN_PROPERTY,
                    );
                }
            }
        }
    }

    #[test]
    fn property_metadata_file_write_capabilities_match_dispatch() {
        for method in [1, 0, u32::MAX] {
            for read_only in [false, true] {
                let mut object = FileObject::new(1, "FILE-1", "raw").unwrap();
                object.set_file_access_method(method);
                object.set_read_only(read_only);
                for row in object.property_metadata().into_owned() {
                    let p = row.property_identifier;
                    let writable = matches!(
                        p,
                        P::DESCRIPTION | P::OUT_OF_SERVICE | P::ARCHIVE | P::FILE_TYPE
                    ) || (!read_only
                        && ((p == P::FILE_SIZE && method == 1)
                            || (p == P::RECORD_COUNT && method == 0)));
                    assert_eq!(
                        row.write_capability,
                        if writable { Always } else { ReadOnly },
                        "{p:?}"
                    );
                    assert_eq!(object.is_writable_property(p), writable, "{p:?}");
                    let value = if matches!(p, P::FILE_SIZE | P::RECORD_COUNT) {
                        PropertyValue::Unsigned(2)
                    } else {
                        object.read_property(p, None).unwrap()
                    };
                    let result = object.write_property(p, None, value.clone(), None);
                    if writable {
                        result.unwrap();
                        assert_eq!(object.read_property(p, None).unwrap(), value);
                    } else {
                        assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                    }
                }
                for p in [P::PRESENT_VALUE, P::ALL, P::RECORD_COUNT] {
                    if object.property_list().contains(&p) {
                        continue;
                    }
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
    }
}

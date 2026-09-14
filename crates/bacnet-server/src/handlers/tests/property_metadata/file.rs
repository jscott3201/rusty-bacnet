use super::assert_rpm_selector_bytes;
use bacnet_objects::{database::ObjectDatabase, file::FileObject, traits::BACnetObject};
use bacnet_types::enums::PropertyIdentifier as P;

#[test]
fn rpm_metadata_file_selectors_preserve_bytes_and_budgets() {
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
    ];
    for method in [1, 0, u32::MAX] {
        for read_only in [false, true] {
            let mut object = FileObject::new(1, "FILE-1", "raw").unwrap();
            object.set_file_access_method(method);
            object.set_read_only(read_only);
            object.set_description("long file description".repeat(100));
            object.set_data(vec![0xAB; 257]);
            object.set_records(vec![vec![0xCD; 9], vec![]]);
            let oid = object.object_identifier();
            let mut all = base.to_vec();
            let mut optional = vec![
                P::DESCRIPTION,
                P::STATUS_FLAGS,
                P::OUT_OF_SERVICE,
                P::RELIABILITY,
            ];
            if method == 0 {
                all.push(P::RECORD_COUNT);
                optional.push(P::RECORD_COUNT);
            }
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            for (selector, expected) in [
                (P::ALL, all.as_slice()),
                (P::REQUIRED, required.as_slice()),
                (P::OPTIONAL, optional.as_slice()),
                (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}

// Colocate File's server projections under the bounded File child module;
// keep this suite selectable by the existing `pics::` gate.
mod pics {
    use super::*;
    use crate::pics::{generate_pics, PicsConfig};
    use crate::server::ServerConfig;

    #[test]
    fn file_property_metadata_is_exact_for_each_representative() {
        for method in [1, 0, u32::MAX] {
            for read_only in [false, true] {
                let mut object = FileObject::new(1, "FILE-1", "raw").unwrap();
                object.set_file_access_method(method);
                object.set_read_only(read_only);
                // Independent (identifier, optional, writable) fixture. Never
                // combine representatives: PICS intentionally selects one.
                let mut expected = vec![
                    (P::OBJECT_IDENTIFIER, false, false),
                    (P::OBJECT_NAME, false, false),
                    (P::OBJECT_TYPE, false, false),
                    (P::DESCRIPTION, true, true),
                    (P::FILE_TYPE, false, true),
                    (P::FILE_SIZE, false, method == 1 && !read_only),
                    (P::MODIFICATION_DATE, false, false),
                    (P::ARCHIVE, false, true),
                    (P::READ_ONLY, false, false),
                    (P::FILE_ACCESS_METHOD, false, false),
                    (P::STATUS_FLAGS, true, false),
                    (P::OUT_OF_SERVICE, true, true),
                    (P::RELIABILITY, true, false),
                    (P::PROPERTY_LIST, false, false),
                ];
                if method == 0 {
                    expected.push((P::RECORD_COUNT, true, !read_only));
                }
                let required = object.required_properties();
                let mut db = ObjectDatabase::new();
                db.add(Box::new(object)).unwrap();
                let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
                assert_eq!(pics.supported_object_types.len(), 1);
                let support = &pics.supported_object_types[0];
                assert_eq!(support.object_type, bacnet_types::enums::ObjectType::FILE);
                assert!(!support.createable);
                assert!(support.deleteable);
                let rows: Vec<_> = support
                    .supported_properties
                    .iter()
                    .map(|row| {
                        assert!(row.access.readable);
                        (row.property_id, row.access.optional, row.access.writable)
                    })
                    .collect();
                assert_eq!(rows, expected, "method={method}, read_only={read_only}");
                assert_eq!(
                    rows.iter()
                        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                        .collect::<Vec<_>>(),
                    required.as_ref()
                );
            }
        }
    }
}

use super::*;
use bacnet_objects::file::FileObject;
use bacnet_objects::property_metadata::PropertyConformance;
use bacnet_types::enums::FileAccessMethod;

fn file(instance: u32, record: bool, read_only: bool) -> FileObject {
    let mut file = FileObject::new(instance, format!("file-{instance}"), "raw").unwrap();
    if record {
        file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    }
    file.set_read_only(read_only);
    file
}

fn file_pics(files: Vec<FileObject>) -> Pics {
    let mut db = ObjectDatabase::new();
    for file in files {
        db.add(Box::new(file)).unwrap();
    }
    generate_pics(&db, &ServerConfig::default(), &PicsConfig::default())
}

fn access(pics: &Pics, property: PropertyIdentifier) -> Option<PropertyAccess> {
    pics.supported_object_types
        .iter()
        .find(|row| row.object_type == ObjectType::FILE)
        .unwrap()
        .supported_properties
        .iter()
        .find(|row| row.property_id == property)
        .map(|row| row.access)
}

#[test]
fn pics_mixed_file_modes_union_writable_resize_capabilities() {
    let pics = file_pics(vec![file(1, false, false), file(2, true, false)]);
    // Neither individual File can satisfy both assertions, independent of the
    // database's HashMap traversal: stream resizes File_Size; record resizes Record_Count.
    assert_eq!(
        access(&pics, PropertyIdentifier::FILE_SIZE),
        Some(PropertyAccess {
            readable: true,
            writable: true,
            optional: false,
        })
    );
    assert_eq!(
        access(&pics, PropertyIdentifier::RECORD_COUNT),
        Some(PropertyAccess {
            readable: true,
            writable: true,
            optional: true,
        })
    );
}

fn rows(properties: &[PropertySupport]) -> Vec<(u32, PropertyAccess)> {
    properties
        .iter()
        .map(|row| (row.property_id.to_raw(), row.access))
        .collect()
}

#[test]
fn pics_file_union_is_ordered_stable_and_retains_read_only_controls() {
    use bacnet_objects::traits::BACnetObject;
    let stream = file(1, false, false);
    let record = file(2, true, false);
    let objects: [&dyn BACnetObject; 2] = [&stream, &record];
    let forward = PicsGenerator::union_property_support(
        objects
            .iter()
            .flat_map(|object| PicsGenerator::object_property_support(*object)),
    );
    let reverse = PicsGenerator::union_property_support(
        objects
            .iter()
            .rev()
            .flat_map(|object| PicsGenerator::object_property_support(*object)),
    );
    assert_eq!(rows(&forward), rows(&reverse));
    let expected = file_pics(vec![stream, record]);
    for files in [
        vec![file(2, true, false), file(1, false, false)],
        vec![
            file(3, false, true),
            file(2, true, false),
            file(4, true, true),
            file(1, false, false),
        ],
    ] {
        let actual = file_pics(files);
        assert_eq!(
            rows(&actual.supported_object_types[0].supported_properties),
            rows(&forward)
        );
        assert_eq!(actual.generate_text(), expected.generate_text());
        assert_eq!(actual.generate_markdown(), expected.generate_markdown());
    }
    for (files, size_writable, count_access) in [
        (vec![file(1, false, false)], true, None),
        (vec![file(1, false, true)], false, None),
        (vec![file(1, true, false)], false, Some(true)),
        (vec![file(1, true, true)], false, Some(false)),
        (
            vec![file(1, false, true), file(2, true, true)],
            false,
            Some(false),
        ),
    ] {
        let pics = file_pics(files);
        assert_eq!(
            access(&pics, PropertyIdentifier::FILE_SIZE),
            Some(PropertyAccess {
                readable: true,
                writable: size_writable,
                optional: false
            })
        );
        assert_eq!(
            access(&pics, PropertyIdentifier::RECORD_COUNT),
            count_access.map(|writable| PropertyAccess {
                readable: true,
                writable,
                optional: true
            })
        );
        let support = &pics.supported_object_types[0];
        assert!(!support.createable);
        assert!(support.deleteable);
        let properties = rows(&support.supported_properties);
        assert!(properties.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for document in [pics.generate_text(), pics.generate_markdown()] {
            assert!(
                document.contains("a row or access flag means at least one instance supports it")
            );
            assert!(document.contains("absent rows do not vote"));
            let positions: Vec<_> = support
                .supported_properties
                .iter()
                .map(|row| document.find(&row.property_id.to_string()).unwrap())
                .collect();
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}

#[test]
fn pics_property_union_required_row_wins_and_access_is_a_union() {
    let declarations = vec![
        PropertySupport {
            property_id: PropertyIdentifier::DESCRIPTION,
            access: PropertyAccess {
                readable: false,
                writable: true,
                optional: true,
            },
            written_through: None,
        },
        PropertySupport {
            property_id: PropertyIdentifier::RECORD_COUNT,
            access: PropertyAccess {
                readable: true,
                writable: false,
                optional: true,
            },
            written_through: None,
        },
        PropertySupport {
            property_id: PropertyIdentifier::DESCRIPTION,
            access: PropertyAccess {
                readable: true,
                writable: false,
                optional: false,
            },
            written_through: None,
        },
    ];
    let expected = vec![
        (
            PropertyIdentifier::DESCRIPTION.to_raw(),
            PropertyAccess {
                readable: true,
                writable: true,
                optional: false,
            },
        ),
        (
            PropertyIdentifier::RECORD_COUNT.to_raw(),
            PropertyAccess {
                readable: true,
                writable: false,
                optional: true,
            },
        ),
    ];
    assert_eq!(
        rows(&PicsGenerator::union_property_support(declarations.clone())),
        expected
    );
    assert_eq!(
        rows(&PicsGenerator::union_property_support(
            declarations.into_iter().rev()
        )),
        expected
    );
    assert!(PicsGenerator::union_property_support([]).is_empty());
}

#[test]
fn pics_device_union_applies_execution_view_to_every_instance() {
    use PropertyIdentifier as P;
    use bacnet_objects::{
        property_metadata::{PropertyMetadata, PropertyWriteCapability},
        traits::BACnetObject,
    };
    use bacnet_types::{error::Error, primitives::ObjectIdentifier};
    use std::borrow::Cow;

    struct DeclaredDevice {
        instance: u32,
        extra: P,
    }
    impl BACnetObject for DeclaredDevice {
        fn object_identifier(&self) -> ObjectIdentifier {
            ObjectIdentifier::new(ObjectType::DEVICE, self.instance).unwrap()
        }
        fn object_name(&self) -> &str {
            if self.instance == 1 {
                "first"
            } else {
                "second"
            }
        }
        fn property_list(&self) -> Cow<'static, [P]> {
            Cow::Owned(vec![
                P::PROTOCOL_SERVICES_SUPPORTED,
                P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
                self.extra,
            ])
        }
        fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
            Cow::Owned(
                self.property_list()
                    .iter()
                    .map(|p| {
                        PropertyMetadata::new(
                            *p,
                            PropertyConformance::RequiredWrite,
                            None,
                            PropertyWriteCapability::Always,
                        )
                    })
                    .collect(),
            )
        }
        fn read_property(&self, _: P, _: Option<u32>) -> Result<PropertyValue, Error> {
            Ok(PropertyValue::Null)
        }
        fn write_property(
            &mut self,
            _: P,
            _: Option<u32>,
            _: PropertyValue,
            _: Option<u8>,
        ) -> Result<(), Error> {
            panic!("PICS must not mutate declarations")
        }
    }
    let mut db = ObjectDatabase::new();
    db.add(Box::new(DeclaredDevice {
        instance: 1,
        extra: P::DESCRIPTION,
    }))
    .unwrap();
    db.add(Box::new(DeclaredDevice {
        instance: 2,
        extra: P::LOCATION,
    }))
    .unwrap();
    let config = ServerConfig::default();
    let pics_config = PicsConfig::default();
    let raw = PicsGenerator::new(&db, &config, &pics_config).generate();
    let served = PicsGenerator::new(&db, &config, &pics_config)
        .for_server()
        .generate();
    let property = |pics: &Pics, p| {
        pics.supported_object_types[0]
            .supported_properties
            .iter()
            .find(|row| row.property_id == p)
            .map(|row| row.access)
    };
    for p in [P::DESCRIPTION, P::LOCATION] {
        let expected = Some(PropertyAccess {
            readable: true,
            writable: true,
            optional: false,
        });
        assert_eq!(property(&raw, p), expected);
        assert_eq!(property(&served, p), expected);
    }
    for p in [
        P::PROTOCOL_SERVICES_SUPPORTED,
        P::PROPERTY_LIST,
        P::ACTIVE_COV_SUBSCRIPTIONS,
        P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
    ] {
        assert_eq!(
            property(&served, p),
            Some(PropertyAccess {
                readable: true,
                writable: false,
                optional: matches!(
                    p,
                    P::ACTIVE_COV_SUBSCRIPTIONS | P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
                )
            })
        );
    }
    assert_eq!(property(&raw, P::ACTIVE_COV_SUBSCRIPTIONS), None);
    assert_eq!(property(&raw, P::PROPERTY_LIST), None);
    for p in [
        P::PROTOCOL_SERVICES_SUPPORTED,
        P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
    ] {
        assert_eq!(
            property(&raw, p),
            Some(PropertyAccess {
                readable: true,
                writable: true,
                optional: false
            })
        );
    }
}

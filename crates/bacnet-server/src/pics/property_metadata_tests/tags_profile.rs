//! The PICS lists the Tags, Profile_Location and Profile_Name rows an object
//! provisions (#1553): all three optional, Tags writable and the profile rows
//! read-only, the local choice their O code allows. Unprovisioned objects
//! list none.
use super::*;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetNameValue;
use std::sync::Arc;
use PropertyIdentifier as P;

fn profile() -> ObjectProfile {
    ObjectProfile {
        tags: Some(vec![BACnetNameValue::semantic("exhaust")]),
        profile_location: Some("https://example.com/p.xdd".into()),
        profile_name: Some("555-lighting".into()),
    }
}

fn pics(provisioned: bool) -> Pics {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    let mut temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    let mut lighting = LightingOutputObject::new(1, "LO-1").unwrap();
    let mut binary = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    if provisioned {
        color.set_profile(profile()).unwrap();
        temperature.set_profile(profile()).unwrap();
        lighting.set_profile(profile()).unwrap();
        binary.set_profile(profile()).unwrap();
    }
    let objects: [Box<dyn BACnetObject>; 4] = [
        Box::new(color),
        Box::new(temperature),
        Box::new(lighting),
        Box::new(binary),
    ];
    let mut db = ObjectDatabase::new();
    for object in objects {
        db.add(object).unwrap();
    }
    generate_pics(&db, &ServerConfig::default(), &PicsConfig::default())
}

#[test]
fn pics_lists_provisioned_tags_and_read_only_profile_rows() {
    let rows = [
        (P::TAGS, true),
        (P::PROFILE_LOCATION, false),
        (P::PROFILE_NAME, false),
    ];
    let unprovisioned = pics(false);
    let provisioned = pics(true);
    assert_eq!(provisioned.supported_object_types.len(), 4);
    for support in &unprovisioned.supported_object_types {
        assert!(support
            .supported_properties
            .iter()
            .all(|row| rows.iter().all(|(p, _)| row.property_id != *p)));
    }
    for support in &provisioned.supported_object_types {
        for (property, writable) in rows {
            let row = property_support(&provisioned, support.object_type, property);
            assert!(row.access.readable, "{property:?}");
            assert_eq!(row.access.writable, writable, "{property:?}");
            assert!(row.access.optional, "{property:?}");
        }
    }
    let text = provisioned.generate_text();
    for (property, _) in rows {
        assert!(text.contains(&property.to_string()), "{property:?}");
    }
}

struct SavedEmpty;

impl TagsPersistence for SavedEmpty {
    fn load(
        &self,
        _: ObjectIdentifier,
    ) -> Result<Option<TagsSnapshot>, bacnet_types::error::Error> {
        Ok(Some(TagsSnapshot { tags: Some(vec![]) }))
    }

    fn save(
        &self,
        _: ObjectIdentifier,
        _: &TagsSnapshot,
    ) -> Result<(), bacnet_types::error::Error> {
        panic!("provisioning and PICS must not save configuration")
    }
}

#[test]
fn saved_tags_do_not_provision_rows_and_each_profile_row_remains_independent() {
    for mask in 0..8 {
        let configured = ObjectProfile {
            tags: (mask & 1 != 0).then(|| vec![BACnetNameValue::semantic("configured")]),
            profile_location: (mask & 2 != 0).then(|| "https://example.com/p.xdd".into()),
            profile_name: (mask & 4 != 0).then(|| "555-profile".into()),
        };
        macro_rules! build {
            ($ty:ty) => {{
                let mut object =
                    <$ty>::with_tags_persistence(1, stringify!($ty), Arc::new(SavedEmpty)).unwrap();
                object.set_profile(configured.clone()).unwrap();
                // Removing and reprovisioning must retain the saved-empty override.
                object.set_profile(ObjectProfile::default()).unwrap();
                object.set_profile(configured.clone()).unwrap();
                Box::new(object) as Box<dyn BACnetObject>
            }};
        }
        let mut db = ObjectDatabase::new();
        for mut object in [
            build!(ColorObject),
            build!(ColorTemperatureObject),
            build!(LightingOutputObject),
            build!(BinaryLightingOutputObject),
        ] {
            for (bit, property) in [(1, P::TAGS), (2, P::PROFILE_LOCATION), (4, P::PROFILE_NAME)] {
                assert_eq!(object.property_list().contains(&property), mask & bit != 0);
                let read = object.read_property(property, None);
                if mask & bit == 0 {
                    assert!(
                        matches!(read, Err(bacnet_types::error::Error::Protocol { code, .. }) if code == bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
                    );
                } else if property == P::TAGS {
                    assert_eq!(read.unwrap(), PropertyValue::List(vec![]));
                } else {
                    let result = object.write_property(
                        property,
                        None,
                        PropertyValue::CharacterString("555-other".into()),
                        None,
                    );
                    assert!(
                        matches!(result, Err(bacnet_types::error::Error::Protocol { code, .. }) if code == bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32)
                    );
                }
            }
            db.add(object).unwrap();
        }
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        for support in &pics.supported_object_types {
            for (bit, property) in [(1, P::TAGS), (2, P::PROFILE_LOCATION), (4, P::PROFILE_NAME)] {
                let row = support
                    .supported_properties
                    .iter()
                    .find(|row| row.property_id == property);
                assert_eq!(row.is_some(), mask & bit != 0);
                if let Some(row) = row {
                    assert!(row.access.optional && row.access.readable);
                    assert_eq!(row.access.writable, property == P::TAGS);
                }
            }
        }
    }
}

#[test]
fn value_profile_masks_and_saved_empty_are_independent_of_present_value_mode() {
    use bacnet_objects::analog::AnalogValueObject;
    use bacnet_objects::binary::BinaryValueObject;
    use bacnet_objects::multistate::MultiStateValueObject;
    use bacnet_objects::present_value_access::PresentValueAccess as Access;
    for access in [Access::Commandable, Access::Writable, Access::ReadOnly] {
        for mask in 0..8 {
            let configured = ObjectProfile {
                tags: (mask & 1 != 0).then(|| vec![BACnetNameValue::semantic("configured")]),
                profile_location: (mask & 2 != 0).then(|| "https://example.com/p.xdd".into()),
                profile_name: (mask & 4 != 0).then(|| "555-value".into()),
            };
            macro_rules! build {
                ($ctor:expr) => {{
                    let mut object = $ctor.unwrap();
                    object.set_profile(configured.clone()).unwrap();
                    object.set_profile(ObjectProfile::default()).unwrap();
                    object.set_profile(configured.clone()).unwrap();
                    Box::new(object) as Box<dyn BACnetObject>
                }};
            }
            // Separate PICS per mode prevents type-level union from hiding a gap.
            let mut db = ObjectDatabase::new();
            for object in [
                build!(AnalogValueObject::with_tags_persistence(
                    1,
                    "AV",
                    95,
                    access,
                    Arc::new(SavedEmpty)
                )),
                build!(BinaryValueObject::with_tags_persistence(
                    1,
                    "BV",
                    access,
                    Arc::new(SavedEmpty)
                )),
                build!(MultiStateValueObject::with_tags_persistence(
                    1,
                    "MSV",
                    3,
                    access,
                    Arc::new(SavedEmpty)
                )),
            ] {
                for (bit, property) in
                    [(1, P::TAGS), (2, P::PROFILE_LOCATION), (4, P::PROFILE_NAME)]
                {
                    assert_eq!(object.property_list().contains(&property), mask & bit != 0);
                    let read = object.read_property(property, None);
                    if mask & bit == 0 {
                        assert!(
                            matches!(read, Err(bacnet_types::error::Error::Protocol { code, .. }) if code == bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
                        );
                    } else if property == P::TAGS {
                        assert_eq!(read.unwrap(), PropertyValue::List(vec![]));
                    }
                }
                db.add(object).unwrap();
            }
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 3);
            for support in &pics.supported_object_types {
                for (bit, property) in
                    [(1, P::TAGS), (2, P::PROFILE_LOCATION), (4, P::PROFILE_NAME)]
                {
                    let rows: Vec<_> = support
                        .supported_properties
                        .iter()
                        .filter(|row| row.property_id == property)
                        .collect();
                    assert_eq!(rows.len(), usize::from(mask & bit != 0));
                    if let Some(row) = rows.first() {
                        assert!(row.access.optional && row.access.readable);
                        assert_eq!(row.access.writable, property == P::TAGS);
                    }
                }
            }
        }
    }
}

//! The PICS lists the Tags, Profile_Location and Profile_Name rows an object
//! provisions (#1553): all three optional, Tags writable and the profile rows
//! read-only, the local choice their O code allows. Unprovisioned objects
//! list none.
use super::*;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetNameValue;
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

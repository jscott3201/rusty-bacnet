//! Tags, Profile_Location and Profile_Name over ReadProperty and
//! WriteProperty (#1553), the replies as the handlers encode them.
//!
//! A tag is its name as a CharacterString under context tag 0, then any
//! value application-tagged: `exhaust` is semantic, `floor` is Unsigned 3.
//! Tags is property 486 (`92 01 E6` as an Enumerated), Profile_Location 485
//! (`92 01 E5`) and Profile_Name 168 (`91 A8`).

use super::lighting_required_rows::{assert_refused, db_with, read_wire};
use super::*;
use bacnet_objects::color::ColorObject;
use bacnet_objects::lighting::BinaryLightingOutputObject;
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_objects::object_profile::{TagsPersistence, TagsSnapshot};
use bacnet_types::constructed::BACnetNameValue;
use std::sync::{Arc, Mutex};

const TAGS: PropertyIdentifier = PropertyIdentifier::TAGS;
const EXHAUST: [u8; 10] = [0x0D, 0x08, 0x00, 0x65, 0x78, 0x68, 0x61, 0x75, 0x73, 0x74];
const FLOOR: [u8; 10] = [0x0D, 0x06, 0x00, 0x66, 0x6C, 0x6F, 0x6F, 0x72, 0x21, 0x03];
const LOCATION: &str = "https://example.com/p.xdd";
const NAME: &str = "555-lighting";

fn provisioned_color() -> (ObjectDatabase, ObjectIdentifier) {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    color
        .set_profile(ObjectProfile {
            tags: Some(vec![
                BACnetNameValue::semantic("exhaust"),
                BACnetNameValue::valued("floor", PropertyValue::Unsigned(3)),
            ]),
            profile_location: Some(LOCATION.into()),
            profile_name: Some(NAME.into()),
        })
        .unwrap();
    db_with(Box::new(color))
}

fn read_at(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response)?;
    Ok(ReadPropertyACK::decode(&response).unwrap().property_value)
}

fn write_at(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    octets: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: octets.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

/// An application CharacterString in UTF-8.
fn text(value: &str) -> Vec<u8> {
    let mut octets = vec![0x75, value.len() as u8 + 1, 0x00];
    octets.extend(value.as_bytes());
    octets
}

#[test]
fn provisioned_rows_read_with_exact_bytes() {
    let (db, oid) = provisioned_color();
    assert_eq!(read_wire(&db, oid, TAGS), [EXHAUST, FLOOR].concat());
    assert_eq!(read_at(&db, oid, TAGS, Some(0)).unwrap(), [0x21, 0x02]);
    assert_eq!(read_at(&db, oid, TAGS, Some(2)).unwrap(), FLOOR);
    assert_refused(
        read_at(&db, oid, TAGS, Some(3)).map(|_| ()),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PROFILE_LOCATION),
        text(LOCATION)
    );
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PROFILE_NAME),
        text(NAME)
    );
    // Property_List: Transition (385), then the three rows in table order.
    let list = read_wire(&db, oid, PropertyIdentifier::PROPERTY_LIST);
    let tail = [
        0x92, 0x01, 0x81, 0x92, 0x01, 0xE6, 0x92, 0x01, 0xE5, 0x91, 0xA8,
    ];
    assert!(list.ends_with(&tail), "{list:02X?}");

    // An object that provisions none serves none, and lists none.
    let (db, blo) = db_with(Box::new(
        BinaryLightingOutputObject::new(1, "BLO-1").unwrap(),
    ));
    for property in [
        TAGS,
        PropertyIdentifier::PROFILE_LOCATION,
        PropertyIdentifier::PROFILE_NAME,
    ] {
        assert_refused(
            read_at(&db, blo, property, None).map(|_| ()),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    let list = read_wire(&db, blo, PropertyIdentifier::PROPERTY_LIST);
    assert!(!list.windows(3).any(|w| w == [0x92, 0x01, 0xE6]));
}

#[derive(Default)]
struct TagStore(Mutex<Option<TagsSnapshot>>);

impl TagsPersistence for TagStore {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, _: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn persistent_color(store: &Arc<TagStore>) -> (ObjectDatabase, ObjectIdentifier) {
    let mut color = ColorObject::with_tags_persistence(1, "CLR-1", store.clone()).unwrap();
    color
        .set_profile(ObjectProfile {
            tags: Some(vec![BACnetNameValue::semantic("exhaust")]),
            ..ObjectProfile::default()
        })
        .unwrap();
    db_with(Box::new(color))
}

/// The pre-fix memory-only witness failed on reconstruction. Attached
/// storage now makes this an explicit opt-in durability contract.
#[test]
fn tags_network_write_survives_reconstruction() {
    let store = Arc::new(TagStore::default());
    let (mut db, oid) = persistent_color(&store);
    write_at(&mut db, oid, TAGS, None, &FLOOR).unwrap();
    assert_eq!(read_wire(&db, oid, TAGS), FLOOR);
    drop(db);
    let (rebuilt, oid) = persistent_color(&store);
    assert_eq!(read_wire(&rebuilt, oid, TAGS), FLOOR);
}

#[test]
fn tags_take_array_writes_and_the_profile_rows_none() {
    let (mut db, oid) = provisioned_color();
    // Whole: floor, then exhaust.
    write_at(&mut db, oid, TAGS, None, &[FLOOR, EXHAUST].concat()).unwrap();
    assert_eq!(read_wire(&db, oid, TAGS), [FLOOR, EXHAUST].concat());
    // Index 0 grows the array with an empty semantic tag, `09 00`.
    write_at(&mut db, oid, TAGS, Some(0), &[0x21, 0x03]).unwrap();
    assert_eq!(read_at(&db, oid, TAGS, Some(3)).unwrap(), [0x09, 0x00]);
    // One element.
    write_at(&mut db, oid, TAGS, Some(3), &EXHAUST).unwrap();
    assert_eq!(
        read_wire(&db, oid, TAGS),
        [FLOOR, EXHAUST, EXHAUST].concat()
    );
    // Index 0 shrinks it.
    write_at(&mut db, oid, TAGS, Some(0), &[0x21, 0x01]).unwrap();
    assert_eq!(read_wire(&db, oid, TAGS), FLOOR);

    // Refusals leave it as it is.
    assert_refused(
        write_at(&mut db, oid, TAGS, Some(2), &EXHAUST),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    let real = [0x44, 0x3F, 0x80, 0x00, 0x00];
    for index in [None, Some(0), Some(1)] {
        assert_refused(
            write_at(&mut db, oid, TAGS, index, &real),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    // A name in UCS-4 (character set 3), which isn't decoded.
    assert_refused(
        write_at(&mut db, oid, TAGS, None, &[0x0A, 0x03, 0x61]),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    // Past the resource cap: RESOURCES / NO_SPACE_TO_WRITE_PROPERTY.
    let too_many = write_at(&mut db, oid, TAGS, Some(0), &[0x22, 0x04, 0x01]);
    assert!(
        matches!(too_many, Err(Error::Protocol { class, code })
            if class == ErrorClass::RESOURCES.to_raw() as u32
                && code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY.to_raw() as u32),
        "{too_many:?}"
    );
    // A NULL to a property with no NULL in its datatype changes nothing.
    write_at(&mut db, oid, TAGS, None, &[0x00]).unwrap();
    assert_eq!(read_wire(&db, oid, TAGS), FLOOR);

    for property in [
        PropertyIdentifier::PROFILE_LOCATION,
        PropertyIdentifier::PROFILE_NAME,
    ] {
        assert_refused(
            write_at(&mut db, oid, property, None, &text("555-other")),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PROFILE_NAME),
        text(NAME)
    );

    let (mut db, blo) = db_with(Box::new(
        BinaryLightingOutputObject::new(1, "BLO-1").unwrap(),
    ));
    for index in [None, Some(0)] {
        assert_refused(
            write_at(&mut db, blo, TAGS, index, &EXHAUST),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    // An empty array, too: Tags reaches the object as raw octets.
    assert_refused(
        write_at(&mut db, blo, TAGS, None, &[]),
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

#[test]
fn a_whole_tags_write_takes_up_to_max_tags_elements() {
    let (mut db, oid) = provisioned_color();
    // The semantic tag `a`, three octets.
    let tag = [0x0A, 0x00, 0x61];
    let max = bacnet_objects::object_profile::MAX_TAGS;
    write_at(&mut db, oid, TAGS, None, &tag.repeat(max)).unwrap();
    assert_eq!(
        read_at(&db, oid, TAGS, Some(0)).unwrap(),
        [0x22, 0x04, 0x00]
    );
    let too_many = write_at(&mut db, oid, TAGS, None, &tag.repeat(max + 1));
    assert!(
        matches!(too_many, Err(Error::Protocol { class, code })
            if class == ErrorClass::RESOURCES.to_raw() as u32
                && code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY.to_raw() as u32),
        "{too_many:?}"
    );
    assert_eq!(
        read_at(&db, oid, TAGS, Some(0)).unwrap(),
        [0x22, 0x04, 0x00]
    );
}

// New-feature integration witness: the initial provisioning seam alone did not
// connect these rows to object dispatch. This is not a pre-existing-row regression.
#[test]
fn value_profiles_read_with_exact_wire_bytes() {
    use bacnet_objects::analog::AnalogValueObject;
    use bacnet_objects::binary::BinaryValueObject;
    use bacnet_objects::multistate::MultiStateValueObject;
    use bacnet_objects::present_value_access::PresentValueAccess;
    for access in [
        PresentValueAccess::Commandable,
        PresentValueAccess::Writable,
        PresentValueAccess::ReadOnly,
    ] {
        let profile = ObjectProfile {
            tags: Some(vec![
                BACnetNameValue::semantic("exhaust"),
                BACnetNameValue::valued("floor", PropertyValue::Unsigned(3)),
            ]),
            profile_location: Some(LOCATION.into()),
            profile_name: Some(NAME.into()),
        };
        macro_rules! build {
            ($object:expr) => {{
                let mut object = $object.unwrap();
                // Absent rows refuse both reads and writes before provisioning.
                for property in [
                    TAGS,
                    PropertyIdentifier::PROFILE_LOCATION,
                    PropertyIdentifier::PROFILE_NAME,
                ] {
                    assert_refused(
                        object.read_property(property, None).map(|_| ()),
                        ErrorCode::UNKNOWN_PROPERTY,
                    );
                    assert_refused(
                        object.write_property(property, None, PropertyValue::Null, None),
                        ErrorCode::UNKNOWN_PROPERTY,
                    );
                    assert!(!object.property_list().contains(&property));
                }
                object.set_profile(profile.clone()).unwrap();
                Box::new(object) as Box<dyn bacnet_objects::traits::BACnetObject>
            }};
        }
        for object in [
            build!(AnalogValueObject::with_access(1, "AV", 95, access)),
            build!(BinaryValueObject::with_access(1, "BV", access)),
            build!(MultiStateValueObject::with_access(1, "MSV", 3, access)),
        ] {
            let (mut db, oid) = db_with(object);
            assert_eq!(read_wire(&db, oid, TAGS), [EXHAUST, FLOOR].concat());
            assert_eq!(read_at(&db, oid, TAGS, Some(0)).unwrap(), [0x21, 2]);
            assert_eq!(read_at(&db, oid, TAGS, Some(2)).unwrap(), FLOOR);
            assert_refused(
                read_at(&db, oid, TAGS, Some(3)).map(|_| ()),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
            for (property, expected) in [
                (PropertyIdentifier::PROFILE_LOCATION, LOCATION),
                (PropertyIdentifier::PROFILE_NAME, NAME),
            ] {
                assert_eq!(read_wire(&db, oid, property), text(expected));
                assert_refused(
                    read_at(&db, oid, property, Some(0)).map(|_| ()),
                    ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                );
                assert_refused(
                    write_at(&mut db, oid, property, None, &text("555-other")),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert_refused(
                    write_at(&mut db, oid, property, Some(1), &text("555-other")),
                    ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                );
            }
            let list = read_wire(&db, oid, PropertyIdentifier::PROPERTY_LIST);
            assert!(list.ends_with(&[0x92, 0x01, 0xE6, 0x92, 0x01, 0xE5, 0x91, 0xA8]));
            write_at(&mut db, oid, TAGS, None, &FLOOR).unwrap();
            assert_eq!(read_wire(&db, oid, TAGS), FLOOR);
            write_at(&mut db, oid, TAGS, Some(0), &[0x21, 2]).unwrap();
            assert_eq!(read_at(&db, oid, TAGS, Some(2)).unwrap(), [0x09, 0]);
            write_at(&mut db, oid, TAGS, Some(2), &EXHAUST).unwrap();
            assert_eq!(read_wire(&db, oid, TAGS), [FLOOR, EXHAUST].concat());
            write_at(&mut db, oid, TAGS, None, &[]).unwrap();
            assert_eq!(read_wire(&db, oid, TAGS), Vec::<u8>::new());
            assert_eq!(read_at(&db, oid, TAGS, Some(0)).unwrap(), [0x21, 0]);
        }
    }
}

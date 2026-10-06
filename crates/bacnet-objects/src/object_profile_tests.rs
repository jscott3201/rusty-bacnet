//! The Tags, Profile_Location and Profile_Name rows (#1553): absent until
//! provisioned; Tags written as an array (Clause 12.1.5.1); the profile rows
//! read-only; and where each object lists them.

use super::*;
use crate::color::{ColorObject, ColorTemperatureObject};
use crate::lighting::{BinaryLightingOutputObject, ColorLink, LightingOutputObject};
use crate::property_metadata::PropertyConformance;
use crate::traits::BACnetObject;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

fn assert_code<T: std::fmt::Debug>(result: Result<T, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { code, .. }) => {
            assert_eq!(code, expected.to_raw() as u32, "expected {expected:?}")
        }
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

/// `exhaust` (semantic) and `floor` = 3.
fn two_tags() -> Vec<BACnetNameValue> {
    vec![
        BACnetNameValue::semantic("exhaust"),
        BACnetNameValue::valued("floor", TagValue::Primitive(PropertyValue::Unsigned(3))),
    ]
}

const EXHAUST: [u8; 10] = [0x0D, 0x08, 0x00, 0x65, 0x78, 0x68, 0x61, 0x75, 0x73, 0x74];
const FLOOR: [u8; 9] = [0x0D, 0x06, 0x00, 0x66, 0x6C, 0x6F, 0x6F, 0x72, 0x21];
/// `zone` = "east": a CharacterString value.
const ZONE: [u8; 13] = [
    0x0D, 0x05, 0x00, 0x7A, 0x6F, 0x6E, 0x65, 0x75, 0x05, 0x00, 0x65, 0x61, 0x73,
];

fn floor() -> Vec<u8> {
    [&FLOOR[..], &[0x03]].concat()
}

fn zone() -> Vec<u8> {
    [&ZONE[..], &[0x74]].concat()
}

fn data(octets: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(octets.to_vec())
}

fn tagged() -> ObjectProfile {
    ObjectProfile {
        tags: Some(two_tags()),
        ..ObjectProfile::default()
    }
}

#[test]
fn an_unprovisioned_profile_serves_and_lists_nothing() {
    let mut profile = ObjectProfile::default();
    profile.check().unwrap();
    assert_eq!(profile.metadata().count(), 0);
    for property in [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME] {
        assert_code(
            profile.read(property, None).unwrap(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_code(
            profile.write(property, None, &data(&EXHAUST)).unwrap(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert!(profile.read(P::DESCRIPTION, None).is_none());
    assert!(profile
        .write(P::DESCRIPTION, None, &PropertyValue::Null)
        .is_none());
}

#[test]
fn tags_read_whole_by_size_and_by_element() {
    let profile = tagged();
    assert_eq!(
        profile.read(P::TAGS, None).unwrap().unwrap(),
        PropertyValue::List(vec![data(&EXHAUST), data(&floor())])
    );
    assert_eq!(
        profile.read(P::TAGS, Some(0)).unwrap().unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        profile.read(P::TAGS, Some(2)).unwrap().unwrap(),
        data(&floor())
    );
    assert_code(
        profile.read(P::TAGS, Some(3)).unwrap(),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
}

#[test]
fn tags_are_written_whole_resized_at_index_0_and_by_element() {
    let mut profile = tagged();
    let mut write = |index, value: PropertyValue| profile.write(P::TAGS, index, &value).unwrap();

    // Whole, as one octet string or as the elements a read returns.
    write(None, data(&[zone(), EXHAUST.to_vec()].concat())).unwrap();
    write(
        None,
        PropertyValue::List(vec![data(&zone()), data(&EXHAUST)]),
    )
    .unwrap();
    // Size 3 appends a semantic tag with an empty name, size 1 truncates.
    write(Some(0), PropertyValue::Unsigned(3)).unwrap();
    // An element, in range.
    write(Some(2), data(&floor())).unwrap();
    let expected = vec![
        BACnetNameValue::valued(
            "zone",
            TagValue::Primitive(PropertyValue::CharacterString("east".into())),
        ),
        BACnetNameValue::valued("floor", TagValue::Primitive(PropertyValue::Unsigned(3))),
        BACnetNameValue::semantic(""),
    ];
    assert_eq!(profile.tags.as_deref(), Some(expected.as_slice()));

    let mut profile = tagged();
    let mut write = |index, value: PropertyValue| profile.write(P::TAGS, index, &value).unwrap();
    write(Some(0), PropertyValue::Unsigned(1)).unwrap();
    // Past the end is refused and doesn't grow the array (Clause 12.1.5.1).
    assert_code(
        write(Some(2), data(&floor())),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    assert_code(
        write(Some(0), PropertyValue::Unsigned(MAX_TAGS as u64 + 1)),
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
    );
    // Wrong datatypes: a REAL, a NULL, or a REAL where an element belongs.
    assert_code(
        write(Some(0), PropertyValue::Real(2.0)),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_code(
        write(None, PropertyValue::Null),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_code(
        write(Some(1), data(&[0x44, 0, 0, 0, 0])),
        ErrorCode::INVALID_DATA_TYPE,
    );
    // Encodings: a broken element, or two where one belongs.
    assert_code(
        write(None, data(&EXHAUST[..4])),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    assert_code(
        write(Some(1), data(&[EXHAUST, EXHAUST].concat())),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    // A semicolon in a name (Annex Y.1.4).
    let semicolon = [0x0B, 0x00, 0x61, 0x3B];
    assert_code(
        write(Some(1), data(&semicolon)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_code(write(None, data(&semicolon)), ErrorCode::VALUE_OUT_OF_RANGE);
    assert_eq!(profile.tags.as_deref(), Some(&two_tags()[..1]));

    // Whole, empty: no tags, the row still present.
    profile.write(P::TAGS, None, &data(&[])).unwrap().unwrap();
    assert_eq!(profile.tags.as_deref(), Some(&[][..]));
    assert_eq!(profile.metadata().count(), 1);
}

#[test]
fn profile_rows_are_read_only_and_checked_when_provisioned() {
    let profile = ObjectProfile {
        profile_location: Some("https://example.com/profiles.xdd".into()),
        profile_name: Some("555-lighting".into()),
        ..ObjectProfile::default()
    };
    profile.check().unwrap();
    let mut profile = profile;
    assert_eq!(
        profile.read(P::PROFILE_NAME, None).unwrap().unwrap(),
        PropertyValue::CharacterString("555-lighting".into())
    );
    assert_code(
        profile.read(P::PROFILE_LOCATION, Some(1)).unwrap(),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    for property in [P::PROFILE_LOCATION, P::PROFILE_NAME] {
        let value = PropertyValue::CharacterString("555-other".into());
        assert_code(
            profile.write(property, None, &value).unwrap(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    let rows: Vec<_> = profile.metadata().collect();
    assert_eq!(
        rows,
        [
            PropertyMetadata::new(P::PROFILE_LOCATION, Optional, None, ReadOnly),
            PropertyMetadata::new(P::PROFILE_NAME, Optional, None, ReadOnly),
        ]
    );

    // An empty location is allowed: a client then uses the Device's.
    for location in ["", "HTTP://x", "bacnet://5/device,5", "http:x"] {
        let ok = ObjectProfile {
            profile_location: Some(location.into()),
            ..ObjectProfile::default()
        };
        ok.check().unwrap();
    }
    let refused = [
        ObjectProfile {
            profile_location: Some("ftp://example.com/p.xdd".into()),
            ..ObjectProfile::default()
        },
        ObjectProfile {
            profile_location: Some("example.com".into()),
            ..ObjectProfile::default()
        },
        ObjectProfile {
            profile_name: Some("acme-lighting".into()),
            ..ObjectProfile::default()
        },
        ObjectProfile {
            profile_name: Some("70000-lighting".into()),
            ..ObjectProfile::default()
        },
        ObjectProfile {
            profile_name: Some("555lighting".into()),
            ..ObjectProfile::default()
        },
        ObjectProfile {
            tags: Some(vec![BACnetNameValue::semantic("a;b")]),
            ..ObjectProfile::default()
        },
    ];
    for profile in refused {
        assert_code(profile.check(), ErrorCode::VALUE_OUT_OF_RANGE);
    }
    let constructed = ObjectProfile {
        tags: Some(vec![BACnetNameValue::valued(
            "a",
            TagValue::Primitive(PropertyValue::List(vec![])),
        )]),
        ..ObjectProfile::default()
    };
    assert_code(constructed.check(), ErrorCode::INVALID_DATA_TYPE);
    let too_many = ObjectProfile {
        tags: Some(vec![BACnetNameValue::semantic("a"); MAX_TAGS + 1]),
        ..ObjectProfile::default()
    };
    assert_code(too_many.check(), ErrorCode::NO_SPACE_TO_WRITE_PROPERTY);
}

fn full_profile() -> ObjectProfile {
    ObjectProfile {
        tags: Some(two_tags()),
        profile_location: Some("https://example.com/p.xdd".into()),
        profile_name: Some("555-lighting".into()),
    }
}

/// An object fresh, the same object provisioned with every row, and the row
/// its profile rows come after.
type Case = (Box<dyn BACnetObject>, Box<dyn BACnetObject>, P);

/// Each object's [`Case`].
fn objects() -> Vec<Case> {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    color.set_profile(full_profile()).unwrap();
    let mut temperature = ColorTemperatureObject::new(1, "CT-1").unwrap();
    temperature.set_profile(full_profile()).unwrap();
    // A Lighting Output's tags follow its colour link and precede its trims.
    let reference = ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap();
    let mut lighting = LightingOutputObject::new(1, "LO-1").unwrap();
    lighting
        .set_color_link(Some(ColorLink::new(reference)))
        .unwrap();
    lighting.set_high_end_trim(Some(90.0)).unwrap();
    let mut lit = lighting.clone();
    lit.set_profile(full_profile()).unwrap();
    let mut binary = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
    binary.set_profile(full_profile()).unwrap();
    vec![
        (
            Box::new(ColorObject::new(1, "CLR-1").unwrap()),
            Box::new(color),
            P::TRANSITION,
        ),
        (
            Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()),
            Box::new(temperature),
            P::TRANSITION,
        ),
        (Box::new(lighting), Box::new(lit), P::COLOR_REFERENCE),
        (
            Box::new(BinaryLightingOutputObject::new(1, "BLO-1").unwrap()),
            Box::new(binary),
            P::CURRENT_COMMAND_PRIORITY,
        ),
    ]
}

#[test]
fn objects_list_provisioned_rows_in_table_order_and_serve_them() {
    for (fresh, mut provisioned, after) in objects() {
        let kind = fresh.object_identifier().object_type();
        let before = fresh.property_list().into_owned();
        for property in [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME] {
            assert!(!before.contains(&property), "{kind:?}");
            assert_code(
                fresh.read_property(property, None),
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
        let mut expected = before.clone();
        let at = expected.iter().position(|p| *p == after).unwrap() + 1;
        expected.splice(at..at, [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME]);
        assert_eq!(provisioned.property_list().as_ref(), expected, "{kind:?}");
        let metadata = provisioned.property_metadata();
        for row in metadata.iter().filter(|row| {
            matches!(
                row.property_identifier,
                P::TAGS | P::PROFILE_LOCATION | P::PROFILE_NAME
            )
        }) {
            assert_eq!(row.conformance, PropertyConformance::Optional);
            assert_eq!(row.presence_condition, None);
            assert_eq!(
                row.write_capability.is_writable(),
                row.property_identifier == P::TAGS
            );
        }
        assert!(!provisioned.required_properties().contains(&P::TAGS));
        assert_eq!(
            provisioned.read_property(P::TAGS, Some(1)).unwrap(),
            data(&EXHAUST)
        );
        provisioned
            .write_property(P::TAGS, Some(0), PropertyValue::Unsigned(1), None)
            .unwrap();
        assert_eq!(
            provisioned.read_property(P::TAGS, Some(0)).unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_code(
            provisioned.write_property(
                P::PROFILE_NAME,
                None,
                PropertyValue::CharacterString("1-x".into()),
                None,
            ),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        // A refused profile changes nothing.
        let mut color = ColorObject::new(1, "CLR-1").unwrap();
        let bad = ObjectProfile {
            profile_name: Some("lighting".into()),
            ..full_profile()
        };
        assert_code(color.set_profile(bad), ErrorCode::VALUE_OUT_OF_RANGE);
        assert!(!color.property_list().contains(&P::TAGS));
    }
}

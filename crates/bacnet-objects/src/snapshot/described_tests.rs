//! The one-file formats' versions with a written Description (#2364):
//! Notification Class (`RBNNCL02`), Notification Forwarder (`RBNFWD02`),
//! Access Rights (`RBNACR02`), Tags (`RBNTAG02`) and the colour output
//! (`RBNCLR03`). Each puts the Description between the header
//! and the previous version's body, a file without one keeps the previous
//! version's octets, a build that reads only that version refuses the new
//! one as an unknown version, and damage to the Description is refused.

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::framing::{self, HEADER_LEN};
use super::{
    SnapshotError, SnapshotErrorKind, access_rights, color, notification_class,
    notification_forwarder, tags,
};
use crate::access_control::AccessRightsSnapshot;
use crate::color::{ColorOutputSnapshot, ColorOutputValue};
use crate::notification_class::NotificationClassSnapshot;
use crate::notification_forwarder::ForwarderSnapshot;
use crate::object_profile::TagsSnapshot;
use bacnet_types::constructed::{BACnetNameValue, BACnetXyColor};

fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 7).unwrap()
}

/// One format: its object, its two tags and the codec, over a snapshot
/// with `description` set as given.
struct Format {
    object: ObjectIdentifier,
    magic: [u8; 8],
    described: [u8; 8],
    /// The version `described` names.
    version: u8,
    encode: fn(ObjectIdentifier, Option<&str>) -> Result<Vec<u8>, SnapshotError>,
    decode: fn(ObjectIdentifier, &[u8]) -> Result<Option<String>, SnapshotError>,
}

fn formats() -> [Format; 5] {
    [
        Format {
            object: oid(ObjectType::NOTIFICATION_CLASS),
            magic: notification_class::MAGIC,
            described: notification_class::MAGIC_WITH_DESCRIPTION,
            version: 2,
            encode: |object, description| {
                let snapshot = NotificationClassSnapshot {
                    recipient_list: Some(Vec::new()),
                    description: description.map(Into::into),
                };
                notification_class::encode(object, &snapshot)
            },
            decode: |object, octets| {
                let snapshot = notification_class::decode(object, octets)?;
                assert_eq!(snapshot.recipient_list, Some(Vec::new()));
                Ok(snapshot.description)
            },
        },
        Format {
            object: oid(ObjectType::NOTIFICATION_FORWARDER),
            magic: notification_forwarder::MAGIC,
            described: notification_forwarder::MAGIC_WITH_DESCRIPTION,
            version: 2,
            encode: |object, description| {
                let snapshot = ForwarderSnapshot {
                    recipient_list: Some(Vec::new()),
                    subscribed_recipients: Vec::new(),
                    description: description.map(Into::into),
                };
                notification_forwarder::encode(object, &snapshot)
            },
            decode: |object, octets| {
                let snapshot = notification_forwarder::decode(object, octets)?;
                assert_eq!(snapshot.recipient_list, Some(Vec::new()));
                Ok(snapshot.description)
            },
        },
        Format {
            object: oid(ObjectType::ACCESS_RIGHTS),
            magic: access_rights::MAGIC,
            described: access_rights::MAGIC_WITH_DESCRIPTION,
            version: 2,
            encode: |object, description| {
                let snapshot = AccessRightsSnapshot {
                    enable: Some(true),
                    description: description.map(Into::into),
                    ..AccessRightsSnapshot::default()
                };
                access_rights::encode(object, &snapshot)
            },
            decode: |object, octets| {
                let snapshot = access_rights::decode(object, octets)?;
                assert_eq!(snapshot.enable, Some(true));
                Ok(snapshot.description)
            },
        },
        Format {
            object: oid(ObjectType::LIGHTING_OUTPUT),
            magic: tags::MAGIC,
            described: tags::MAGIC_WITH_DESCRIPTION,
            version: 2,
            encode: |object, description| {
                let snapshot = TagsSnapshot {
                    tags: Some(vec![BACnetNameValue::semantic("exhaust")]),
                    description: description.map(Into::into),
                };
                tags::encode(object, &snapshot)
            },
            decode: |object, octets| {
                let snapshot = tags::decode(object, octets)?;
                let exhaust = vec![BACnetNameValue::semantic("exhaust")];
                assert_eq!(snapshot.tags, Some(exhaust));
                Ok(snapshot.description)
            },
        },
        Format {
            object: oid(ObjectType::COLOR),
            magic: color::MAGIC,
            described: color::MAGIC_WITH_DESCRIPTION,
            version: 3,
            encode: |object, description| {
                let snapshot = ColorOutputSnapshot {
                    present_value: Some(colour()),
                    default: None,
                    description: description.map(Into::into),
                };
                color::encode(object, &snapshot)
            },
            decode: |object, octets| {
                let snapshot = color::decode(object, octets)?;
                assert_eq!(snapshot.present_value, Some(colour()));
                Ok(snapshot.description)
            },
        },
    ]
}

fn colour() -> ColorOutputValue {
    ColorOutputValue::Xy(BACnetXyColor::new(0.25, 0.5))
}

/// The described file is the plain one under the new tag, with the
/// Description's length and octets between the header and the body, and
/// both load.
#[test]
fn a_description_goes_between_the_header_and_the_previous_body() {
    for format in formats() {
        let plain = (format.encode)(format.object, None).unwrap();
        assert_eq!(plain[..8], format.magic);
        assert_eq!((format.decode)(format.object, &plain), Ok(None));
        let described = (format.encode)(format.object, Some("Lobby")).unwrap();
        let mut expected = format.described.to_vec();
        expected.extend_from_slice(&plain[8..HEADER_LEN]);
        expected.extend_from_slice(&[0, 0, 0, 5]);
        expected.extend_from_slice(b"Lobby");
        expected.extend_from_slice(&plain[HEADER_LEN..]);
        assert_eq!(described, expected, "{}", format.object);
        assert_eq!(
            (format.decode)(format.object, &described),
            Ok(Some("Lobby".into()))
        );
        // An empty written Description is kept as one.
        let empty = (format.encode)(format.object, Some("")).unwrap();
        assert_eq!(empty[..8], format.described);
        assert_eq!((format.decode)(format.object, &empty), Ok(Some("".into())));
    }
}

/// A build that reads only the previous version refuses the new one as an
/// unknown version, not as damage or another format.
#[test]
fn a_reader_of_the_previous_version_refuses_the_new_one() {
    for format in formats() {
        let described = (format.encode)(format.object, Some("Lobby")).unwrap();
        let older = framing::open(&described, &format.magic, None, usize::MAX, format.object);
        assert_eq!(
            older.unwrap_err().kind(),
            SnapshotErrorKind::UnknownVersion(format.version.into()),
            "{}",
            format.object
        );
        // And this build refuses the version after it the same way.
        let mut later = described.clone();
        later[7] = b'0' + format.version + 1;
        let error = (format.decode)(format.object, &later).unwrap_err();
        let unknown = SnapshotErrorKind::UnknownVersion(u16::from(format.version) + 1);
        assert_eq!(error.kind(), unknown, "{}", format.object);
    }
}

/// 16 KiB is kept; one octet more is refused on encode, and as damage in a
/// file. So is a Description that isn't UTF-8, and every cut inside one.
#[test]
fn a_damaged_description_is_refused() {
    let longest = "d".repeat(16 * 1024);
    for format in formats() {
        let object = format.object;
        let octets = (format.encode)(object, Some(&longest)).unwrap();
        assert_eq!((format.decode)(object, &octets), Ok(Some(longest.clone())));
        let longer = format!("{longest}d");
        let error = (format.encode)(object, Some(&longer)).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{object}");

        let mut past_cap = octets.clone();
        past_cap[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&(16 * 1024 + 1_u32).to_be_bytes());
        past_cap.insert(HEADER_LEN + 4, b'd');
        assert!((format.decode)(object, &past_cap).unwrap_err().is_damage());

        let mut not_utf8 = (format.encode)(object, Some("Lobby")).unwrap();
        not_utf8[HEADER_LEN + 4] = 0xff;
        assert!((format.decode)(object, &not_utf8).unwrap_err().is_damage());

        // Cut inside the length or the text: each is refused.
        let described = (format.encode)(object, Some("Lobby")).unwrap();
        for end in HEADER_LEN..HEADER_LEN + 4 + 5 {
            let error = (format.decode)(object, &described[..end]).unwrap_err();
            assert!(error.is_damage(), "{object} cut at {end}: {error:?}");
        }
    }
}

/// All three versions of the colour format load: version 1 with no default
/// and no Description, version 2 with no Description. A version 3 file at
/// the cap loads, and one octet past it is too large.
#[test]
fn every_colour_version_loads() {
    let object = oid(ObjectType::COLOR);
    let described = ColorOutputSnapshot {
        present_value: Some(colour()),
        default: Some(colour()),
        description: Some("d".repeat(16 * 1024)),
    };
    let version_3 = color::encode(object, &described).unwrap();
    assert!(version_3.len() <= color::MAX_LEN);
    assert_eq!(color::decode(object, &version_3), Ok(described.clone()));

    let plain = ColorOutputSnapshot {
        description: None,
        ..described
    };
    let version_2 = color::encode(object, &plain).unwrap();
    assert_eq!(version_2[..8], color::MAGIC);
    assert_eq!(color::decode(object, &version_2), Ok(plain.clone()));

    // Version 1: the kind octet and Present_Value alone.
    let mut version_1 = color::MAGIC_V1.to_vec();
    version_1.extend_from_slice(&version_2[8..HEADER_LEN + 1]);
    version_1.extend_from_slice(&version_2[HEADER_LEN + 2..HEADER_LEN + 10]);
    let first = ColorOutputSnapshot {
        default: None,
        ..plain
    };
    assert_eq!(color::decode(object, &version_1), Ok(first));

    let mut too_large = version_3;
    too_large.resize(color::MAX_LEN + 1, 0);
    let error = color::decode(object, &too_large).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::TooLarge);
}

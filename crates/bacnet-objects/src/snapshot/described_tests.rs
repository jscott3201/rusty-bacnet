//! The one-file formats' versions with a written Description (#2364):
//! Notification Class (`RBNNCL02`), Notification Forwarder (`RBNFWD02`) and
//! Access Rights (`RBNACR02`). Each puts the Description between the header
//! and the previous version's body, a file without one keeps the previous
//! version's octets, a build that reads only that version refuses the new
//! one as an unknown version, and damage to the Description is refused.

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::framing::{self, HEADER_LEN};
use super::{
    SnapshotError, SnapshotErrorKind, access_rights, notification_class, notification_forwarder,
};
use crate::access_control::AccessRightsSnapshot;
use crate::notification_class::NotificationClassSnapshot;
use crate::notification_forwarder::ForwarderSnapshot;

fn oid(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 7).unwrap()
}

/// One format: its object, its two tags and the codec, over a snapshot
/// with `description` set as given.
struct Format {
    object: ObjectIdentifier,
    magic: [u8; 8],
    described: [u8; 8],
    encode: fn(ObjectIdentifier, Option<&str>) -> Result<Vec<u8>, SnapshotError>,
    decode: fn(ObjectIdentifier, &[u8]) -> Result<Option<String>, SnapshotError>,
}

fn formats() -> [Format; 3] {
    [
        Format {
            object: oid(ObjectType::NOTIFICATION_CLASS),
            magic: notification_class::MAGIC,
            described: notification_class::MAGIC_WITH_DESCRIPTION,
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
    ]
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
            SnapshotErrorKind::UnknownVersion(2),
            "{}",
            format.object
        );
        // And this build refuses the version after it the same way.
        let mut later = described.clone();
        later[7] = b'3';
        let error = (format.decode)(format.object, &later).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::UnknownVersion(3));
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

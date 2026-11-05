//! The one-file formats (#2047): Notification Class, forwarder, Tags,
//! colour and Access Rights. Each round-trips without std, refuses with the
//! kind it names, and writes the octets its file backend writes.

use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAccessRule, BACnetAddress, BACnetDestination, BACnetDeviceObjectReference,
    BACnetEventNotificationSubscription, BACnetNameValue, BACnetRecipient, BACnetXyColor,
};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};

use super::tests::oid;
use super::{
    SnapshotError, SnapshotErrorKind, access_rights, color, notification_class,
    notification_forwarder, tags,
};
use crate::access_control::AccessRightsSnapshot;
use crate::color::{ColorOutputSnapshot, ColorOutputValue};
use crate::notification_class::NotificationClassSnapshot;
use crate::notification_forwarder::ForwarderSnapshot;
use crate::object_profile::TagsSnapshot;

pub(super) fn destination(instance: u32) -> BACnetDestination {
    let at = |hour, minute| Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    };
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: at(0, 0),
        to_time: at(23, 59),
        recipient: if instance.is_multiple_of(2) {
            BACnetRecipient::Device(oid(ObjectType::DEVICE, instance))
        } else {
            BACnetRecipient::Address(BACnetAddress {
                network_number: 7,
                mac_address: bacnet_types::MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]),
            })
        },
        process_identifier: instance,
        issue_confirmed_notifications: instance.is_multiple_of(2),
        transitions: EventTransitionBits::all(),
    }
}

pub(super) fn class() -> (ObjectIdentifier, NotificationClassSnapshot) {
    let snapshot = NotificationClassSnapshot {
        recipient_list: Some(vec![destination(2), destination(3)]),
        description: None,
    };
    (oid(ObjectType::NOTIFICATION_CLASS, 1), snapshot)
}

pub(super) fn forwarder() -> (ObjectIdentifier, ForwarderSnapshot) {
    let snapshot = ForwarderSnapshot {
        recipient_list: Some(vec![destination(4)]),
        subscribed_recipients: vec![BACnetEventNotificationSubscription {
            recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, 8)),
            process_identifier: 3,
            issue_confirmed_notifications: false,
            time_remaining: 15,
        }],
        description: None,
    };
    (oid(ObjectType::NOTIFICATION_FORWARDER, 1), snapshot)
}

pub(super) fn tagged() -> (ObjectIdentifier, TagsSnapshot) {
    let snapshot = TagsSnapshot {
        tags: Some(vec![
            BACnetNameValue::semantic("site"),
            BACnetNameValue::valued("floor", PropertyValue::Unsigned(3)),
        ]),
    };
    (oid(ObjectType::ANALOG_VALUE, 1), snapshot)
}

pub(super) fn colour() -> (ObjectIdentifier, ColorOutputSnapshot) {
    let snapshot = ColorOutputSnapshot {
        present_value: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.25, 0.5))),
        default: Some(ColorOutputValue::Xy(BACnetXyColor::new(0.3, 0.3))),
    };
    (oid(ObjectType::COLOR, 1), snapshot)
}

pub(super) fn rights() -> (ObjectIdentifier, AccessRightsSnapshot) {
    let point = BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: oid(ObjectType::ACCESS_POINT, 2),
    };
    let snapshot = AccessRightsSnapshot {
        positive_access_rules: Some(vec![
            BACnetAccessRule::new(None, Some(point), true),
            BACnetAccessRule::new(None, None, false),
        ]),
        negative_access_rules: Some(Vec::new()),
        enable: Some(true),
        accompaniment: Some(BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: oid(ObjectType::ACCESS_CREDENTIAL, 3),
        }),
        description: None,
    };
    (oid(ObjectType::ACCESS_RIGHTS, 1), snapshot)
}

/// Round-trip `snapshot`, then check the refusals every one-file format
/// shares: another tag, another object, a file past the cap.
fn round_trips<S: PartialEq + core::fmt::Debug>(
    magic: [u8; 8],
    max_len: usize,
    object: ObjectIdentifier,
    snapshot: &S,
    encode: fn(ObjectIdentifier, &S) -> Result<Vec<u8>, SnapshotError>,
    decode: fn(ObjectIdentifier, &[u8]) -> Result<S, SnapshotError>,
) {
    let octets = encode(object, snapshot).unwrap();
    assert_eq!(octets[..8], magic);
    assert_eq!(octets[8..12], object.encode());
    assert_eq!(decode(object, &octets).as_ref(), Ok(snapshot));
    let kind = |octets: &[u8], object| decode(object, octets).unwrap_err().kind();
    let mut retagged = octets.clone();
    retagged[0] ^= 1;
    assert_eq!(kind(&retagged, object), SnapshotErrorKind::NotThisFormat);
    assert_eq!(
        kind(&octets[..11], object),
        SnapshotErrorKind::NotThisFormat
    );
    let other = ObjectIdentifier::new(object.object_type(), object.instance_number() + 1).unwrap();
    assert_eq!(kind(&octets, other), SnapshotErrorKind::OtherObject);
    let mut huge = octets;
    huge.resize(max_len + 1, 0);
    assert_eq!(kind(&huge, object), SnapshotErrorKind::TooLarge);
}

#[test]
fn each_one_file_format_round_trips_and_refuses_what_they_all_refuse() {
    let (object, snapshot) = class();
    let (magic, max) = (notification_class::MAGIC, notification_class::MAX_LEN);
    round_trips(
        magic,
        max,
        object,
        &snapshot,
        notification_class::encode,
        notification_class::decode,
    );
    let (object, snapshot) = forwarder();
    let (magic, max) = (
        notification_forwarder::MAGIC,
        notification_forwarder::MAX_LEN,
    );
    round_trips(
        magic,
        max,
        object,
        &snapshot,
        notification_forwarder::encode,
        notification_forwarder::decode,
    );
    let (object, snapshot) = tagged();
    round_trips(
        tags::MAGIC,
        tags::MAX_LEN,
        object,
        &snapshot,
        tags::encode,
        tags::decode,
    );
    let (object, snapshot) = colour();
    round_trips(
        color::MAGIC,
        color::MAX_LEN,
        object,
        &snapshot,
        color::encode,
        color::decode,
    );
    let (object, snapshot) = rights();
    let (magic, max) = (access_rights::MAGIC, access_rights::MAX_LEN);
    round_trips(
        magic,
        max,
        object,
        &snapshot,
        access_rights::encode,
        access_rights::decode,
    );
    // Nothing written still has a file: a presence octet of 0.
    let empty = NotificationClassSnapshot::default();
    let octets = notification_class::encode(class().0, &empty).unwrap();
    assert_eq!(octets[12..], [0]);
    assert_eq!(notification_class::decode(class().0, &octets), Ok(empty));
}

/// A version 1 colour file, Present_Value alone, loads with no default and
/// saves back as version 2.
#[test]
fn a_version_1_colour_file_loads_with_no_default() {
    let object = oid(ObjectType::COLOR_TEMPERATURE, 4);
    let mut v1 = color::MAGIC_V1.to_vec();
    v1.extend_from_slice(&object.encode());
    v1.push(color::KIND_KELVIN);
    v1.extend_from_slice(&2_700u32.to_be_bytes());
    let loaded = color::decode(object, &v1).unwrap();
    assert_eq!(loaded.present_value, Some(ColorOutputValue::Kelvin(2_700)));
    assert_eq!(loaded.default, None);
    assert_eq!(color::encode(object, &loaded).unwrap()[..8], color::MAGIC);
    // A version 1 file is the value alone: a presence octet is too much.
    let mut longer = v1.clone();
    longer.push(0);
    assert_eq!(
        color::decode(object, &longer).unwrap_err().kind(),
        SnapshotErrorKind::Damaged
    );
}

/// A newer version of a format's tag, or a newer colour kind, is a sound
/// file this build doesn't read, not damage: a backend keeping two copies
/// must not fall back past it. A tag of another family is another format.
#[test]
fn a_newer_tag_or_kind_is_not_damage() {
    let (object, snapshot) = class();
    let mut octets = notification_class::encode(object, &snapshot).unwrap();
    octets[6..8].copy_from_slice(b"03");
    let error = notification_class::decode(object, &octets).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::UnknownVersion(3));
    assert!(!error.is_damage());
    let (object, snapshot) = colour();
    let mut octets = color::encode(object, &snapshot).unwrap();
    octets[6..8].copy_from_slice(b"07");
    assert_eq!(
        color::decode(object, &octets).unwrap_err().kind(),
        SnapshotErrorKind::UnknownVersion(7)
    );
    let mut octets = color::encode(object, &snapshot).unwrap();
    octets[12] = 3;
    let error = color::decode(object, &octets).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::UnknownKind(3));
    assert!(!error.is_damage());
    // Another family, or a version that isn't two digits: not this format.
    for tag in [*b"RBNFWD01", *b"RBNCLRxx"] {
        let mut octets = color::encode(object, &snapshot).unwrap();
        octets[..8].copy_from_slice(&tag);
        let error = color::decode(object, &octets).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::NotThisFormat);
        assert!(error.is_damage());
    }
}

/// A list longer than the object keeps never encodes.
#[test]
fn an_over_long_list_never_encodes() {
    let (object, mut snapshot) = class();
    snapshot.recipient_list = Some(vec![destination(2); 33]);
    let error = notification_class::encode(object, &snapshot).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    let (object, mut snapshot) = forwarder();
    let entry = snapshot.subscribed_recipients[0].clone();
    snapshot.subscribed_recipients = vec![entry; 33];
    let error = notification_forwarder::encode(object, &snapshot).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    let (object, mut snapshot) = rights();
    snapshot.negative_access_rules = Some(vec![BACnetAccessRule::new(None, None, true); 1025]);
    let error = access_rights::encode(object, &snapshot).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
}

/// Values no save writes are damage on decode and refusals on encode.
#[test]
fn values_out_of_range_are_damage_in_and_refused_out() {
    let (object, snapshot) = colour();
    let mut octets = color::encode(object, &snapshot).unwrap();
    // Present_Value's x past 1.0.
    octets[14..18].copy_from_slice(&2.0f32.to_be_bytes());
    let error = color::decode(object, &octets).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
    assert!(
        error.to_string().starts_with("Color output file "),
        "{error}"
    );
    let bad = ColorOutputSnapshot {
        present_value: Some(ColorOutputValue::Kelvin(5)),
        default: None,
    };
    let mixed = ColorOutputSnapshot {
        default: Some(ColorOutputValue::Kelvin(2_700)),
        ..snapshot
    };
    for refused in [bad, mixed] {
        let error = color::encode(object, &refused).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
        assert!(!error.is_damage());
    }

    let (object, _) = tagged();
    let semicolon = TagsSnapshot {
        tags: Some(vec![BACnetNameValue::semantic("a;b")]),
    };
    let error = tags::encode(object, &semicolon).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    // The same tag in a file, as another writer could leave it.
    let mut octets = tags::encode(
        object,
        &TagsSnapshot {
            tags: Some(vec![BACnetNameValue::semantic("a:b")]),
        },
    )
    .unwrap();
    let at = octets.iter().rposition(|octet| *octet == b':').unwrap();
    octets[at] = b';';
    assert_eq!(
        tags::decode(object, &octets).unwrap_err().kind(),
        SnapshotErrorKind::Damaged
    );
}

/// A presence octet other than 0 or 1, or octets after a 0, is damage; a
/// forwarder's Recipient_List running past the file is truncation.
#[test]
fn a_bad_presence_octet_or_list_length_is_refused() {
    let (object, snapshot) = class();
    let mut octets = notification_class::encode(object, &snapshot).unwrap();
    octets[12] = 2;
    assert_eq!(
        notification_class::decode(object, &octets)
            .unwrap_err()
            .kind(),
        SnapshotErrorKind::Damaged
    );
    let (object, snapshot) = forwarder();
    let mut octets = notification_forwarder::encode(object, &snapshot).unwrap();
    octets[13..17].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        notification_forwarder::decode(object, &octets)
            .unwrap_err()
            .kind(),
        SnapshotErrorKind::Truncated
    );
    let (object, _) = tagged();
    let mut octets = tags::encode(object, &TagsSnapshot::default()).unwrap();
    octets.push(0);
    assert_eq!(
        tags::decode(object, &octets).unwrap_err().kind(),
        SnapshotErrorKind::Damaged
    );
}

/// Each file backend writes what the public `encode` returns and loads what
/// it decodes, octet for octet, and loads a file `encode` wrote.
// std: the file backends.
#[cfg(feature = "std")]
mod file_backends {
    use super::*;
    use crate::access_control::{AccessRightsPersistence, FileAccessRightsPersistence};
    use crate::color::{ColorOutputPersistence, FileColorOutputPersistence};
    use crate::notification_class::{
        FileNotificationClassPersistence, NotificationClassPersistence,
    };
    use crate::notification_forwarder::{
        FileNotificationForwarderPersistence, NotificationForwarderPersistence,
    };
    use crate::object_profile::{FileTagsPersistence, TagsPersistence};
    use crate::snapshot::tests::file_backends::Scratch;

    /// The backend `$store` saves `$fixture`; its file is what `$format`
    /// encodes and decodes back. Then a file `$format` wrote for `$changed`
    /// loads through the backend.
    macro_rules! same_octets {
        ($store:ty, $format:ident, $fixture:expr, $changed:expr) => {{
            let scratch = Scratch::new();
            let path = scratch.path("state");
            let store = <$store>::new(&path).unwrap();
            let (object, snapshot) = $fixture;
            store.save(object, &snapshot).unwrap();
            let written = std::fs::read(&path).unwrap();
            assert_eq!(written, $format::encode(object, &snapshot).unwrap());
            assert_eq!($format::decode(object, &written).as_ref(), Ok(&snapshot));
            let changed = $changed(snapshot);
            std::fs::write(&path, $format::encode(object, &changed).unwrap()).unwrap();
            assert_eq!(store.load(object).unwrap(), Some(changed));
        }};
    }

    #[test]
    fn each_one_file_backend_writes_and_reads_the_public_octets() {
        same_octets!(
            FileNotificationClassPersistence,
            notification_class,
            class(),
            |mut s: NotificationClassSnapshot| {
                s.recipient_list.as_mut().unwrap().pop();
                s
            }
        );
        same_octets!(
            FileNotificationForwarderPersistence,
            notification_forwarder,
            forwarder(),
            |mut s: ForwarderSnapshot| {
                s.subscribed_recipients.clear();
                s
            }
        );
        same_octets!(FileTagsPersistence, tags, tagged(), |_| TagsSnapshot {
            tags: Some(Vec::new())
        });
        same_octets!(
            FileColorOutputPersistence,
            color,
            colour(),
            |mut s: ColorOutputSnapshot| {
                s.default = None;
                s
            }
        );
        same_octets!(
            FileAccessRightsPersistence,
            access_rights,
            rights(),
            |mut s: AccessRightsSnapshot| {
                s.enable = Some(false);
                s
            }
        );
    }

    /// A colour backend refuses a save its own decode would refuse, and
    /// keeps the file it had.
    #[test]
    fn a_colour_save_out_of_range_is_refused_and_keeps_the_file() {
        let scratch = Scratch::new();
        let path = scratch.path("colour");
        let store = FileColorOutputPersistence::new(&path).unwrap();
        let (object, snapshot) = colour();
        store.save(object, &snapshot).unwrap();
        let before = std::fs::read(&path).unwrap();
        let bad = ColorOutputSnapshot {
            present_value: Some(ColorOutputValue::Xy(BACnetXyColor::new(1.5, 0.0))),
            default: None,
        };
        let error = store.save(object, &bad).unwrap_err();
        assert!(
            matches!(&error, bacnet_types::error::Error::OutOfRange(message)
                if message.starts_with("Color output snapshot")),
            "{error:?}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

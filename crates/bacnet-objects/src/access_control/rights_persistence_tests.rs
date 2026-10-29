//! Access Rights rule arrays and Enable across a restart (#1392): what a
//! rebuilt object serves, which value wins over the configuration, refused
//! saves, and the file backend.

use super::test_storage::{
    MemoryPersistence, assert_refused, door_rule, octets, persistent, positive_only, write,
    write_positive, zone_rule,
};
use super::*;
use crate::access_control::rights_writes::grown_rule;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference};
use bacnet_types::enums::PropertyIdentifier as P;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const SAVED: [P; 3] = [
    P::POSITIVE_ACCESS_RULES,
    P::NEGATIVE_ACCESS_RULES,
    P::LOG_ENABLE,
];

#[test]
fn written_rules_and_enable_read_back_after_a_rebuild() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let positive = [zone_rule(1), zone_rule(2)];
    write_positive(&mut rights, &positive).unwrap();
    // Negative_Access_Rules grows at index 0, then one element is written.
    write(
        &mut rights,
        P::NEGATIVE_ACCESS_RULES,
        Some(0),
        PropertyValue::Unsigned(2),
    )
    .unwrap();
    write(
        &mut rights,
        P::NEGATIVE_ACCESS_RULES,
        Some(2),
        octets(&[zone_rule(3)]),
    )
    .unwrap();
    write(
        &mut rights,
        P::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    let negative = [grown_rule(), zone_rule(3)];
    let expected = AccessRightsSnapshot {
        positive_access_rules: Some(positive.to_vec()),
        negative_access_rules: Some(negative.to_vec()),
        enable: Some(false),
        accompaniment: None,
    };
    assert_eq!(storage.snapshot(), Some(expected));
    assert_eq!(storage.saves(), 4);
    let reads: Vec<_> = SAVED
        .iter()
        .map(|property| rights.read_property(*property, None).unwrap())
        .collect();
    drop(rights);

    let rebuilt = persistent(&storage);
    assert_eq!(rebuilt.positive_access_rules(), positive);
    assert_eq!(rebuilt.negative_access_rules(), negative);
    assert!(!rebuilt.enable());
    for (property, read) in SAVED.iter().zip(reads) {
        assert!(rebuilt.property_saved(*property), "{property:?}");
        assert_eq!(rebuilt.read_property(*property, None).unwrap(), read);
    }
    assert!(!rebuilt.property_saved(P::DESCRIPTION));
}

#[test]
fn an_index_zero_shrink_survives_a_rebuild() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    write_positive(&mut rights, &[zone_rule(1), zone_rule(2), zone_rule(3)]).unwrap();
    write(
        &mut rights,
        P::POSITIVE_ACCESS_RULES,
        Some(0),
        PropertyValue::Unsigned(1),
    )
    .unwrap();
    drop(rights);
    let rebuilt = persistent(&storage);
    assert_eq!(rebuilt.positive_access_rules(), [zone_rule(1)]);
    assert_eq!(
        rebuilt
            .read_property(P::POSITIVE_ACCESS_RULES, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn saved_values_win_over_configured_ones() {
    let storage = Arc::new(MemoryPersistence::default());
    let configured = [zone_rule(1)];
    let written = [zone_rule(2)];

    // First start: storage holds nothing, so the configuration applies, and
    // it is not saved.
    let mut first = persistent(&storage);
    first.set_positive_access_rules(configured.clone()).unwrap();
    first.set_enable(false);
    assert_eq!(first.positive_access_rules(), configured);
    first.wait_for_saves();
    assert_eq!(storage.saves(), 0);
    // An operator then writes the positive rules and Enable; both are saved.
    write_positive(&mut first, &written).unwrap();
    write(
        &mut first,
        P::LOG_ENABLE,
        None,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    drop(first);

    // Second start: the same configuration again, but the written values
    // win. The negative rules, never written, take theirs.
    let mut second = persistent(&storage);
    second
        .set_positive_access_rules(configured.clone())
        .unwrap();
    second
        .set_negative_access_rules(configured.clone())
        .unwrap();
    second.set_enable(false);
    assert_eq!(second.positive_access_rules(), written);
    assert_eq!(second.negative_access_rules(), configured);
    assert!(second.enable());
    assert!(second.property_saved(P::POSITIVE_ACCESS_RULES));
    assert!(!second.property_saved(P::NEGATIVE_ACCESS_RULES));
    // A configured rule is still checked.
    assert_refused(
        second.set_positive_access_rules([door_rule()]),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    second.wait_for_saves();
    assert_eq!(storage.saves(), 2);
    assert_eq!(
        storage.snapshot(),
        Some(AccessRightsSnapshot {
            positive_access_rules: Some(written.to_vec()),
            negative_access_rules: None,
            enable: Some(true),
            accompaniment: None,
        })
    );
}

#[test]
fn configured_values_apply_at_every_start_until_a_write() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut first = persistent(&storage);
    first.set_positive_access_rules([zone_rule(1)]).unwrap();
    // Writes to the properties the object doesn't save save nothing.
    first
        .write_property(
            P::DESCRIPTION,
            None,
            PropertyValue::CharacterString("doors".into()),
            None,
        )
        .unwrap();
    write(
        &mut first,
        P::GLOBAL_IDENTIFIER,
        None,
        PropertyValue::Unsigned(9),
    )
    .unwrap();
    first.wait_for_saves();
    assert_eq!(storage.snapshot(), None);
    drop(first);
    // The application's next configuration applies.
    let mut second = persistent(&storage);
    second.set_positive_access_rules([zone_rule(2)]).unwrap();
    second.set_enable(false);
    assert_eq!(second.positive_access_rules(), [zone_rule(2)]);
    assert!(!second.enable());
    // An object without persistence always takes its setters.
    let mut in_memory = AccessRightsObject::new(2, "AR-2").unwrap();
    write_positive(&mut in_memory, &[zone_rule(3)]).unwrap();
    write(
        &mut in_memory,
        P::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    in_memory.set_positive_access_rules([zone_rule(4)]).unwrap();
    in_memory.set_enable(true);
    assert!(SAVED.iter().all(|p| !in_memory.property_saved(*p)));
    assert_eq!(in_memory.positive_access_rules(), [zone_rule(4)]);
    assert!(in_memory.enable());
}

#[test]
fn a_saved_empty_array_still_wins() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut first = persistent(&storage);
    first.set_positive_access_rules([zone_rule(1)]).unwrap();
    write_positive(&mut first, &[]).unwrap();
    drop(first);
    let mut second = persistent(&storage);
    second.set_positive_access_rules([zone_rule(1)]).unwrap();
    assert!(second.positive_access_rules().is_empty());
    assert_eq!(storage.snapshot(), Some(positive_only(&[])));
}

#[test]
fn a_write_that_cannot_be_saved_is_refused_and_changes_nothing() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let kept = [zone_rule(1), zone_rule(2)];
    write_positive(&mut rights, &kept).unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    for (property, index, value) in [
        (P::POSITIVE_ACCESS_RULES, None, octets(&[zone_rule(3)])),
        (P::POSITIVE_ACCESS_RULES, Some(1), octets(&[zone_rule(3)])),
        (
            P::POSITIVE_ACCESS_RULES,
            Some(0),
            PropertyValue::Unsigned(5),
        ),
        (P::NEGATIVE_ACCESS_RULES, None, octets(&[zone_rule(3)])),
        (P::LOG_ENABLE, None, PropertyValue::Boolean(false)),
    ] {
        assert_refused(
            write(&mut rights, property, index, value),
            ErrorClass::DEVICE,
            ErrorCode::OPERATIONAL_PROBLEM,
        );
    }
    assert_eq!(rights.positive_access_rules(), kept);
    assert!(rights.negative_access_rules().is_empty());
    assert!(rights.enable());
    assert!(!rights.property_saved(P::NEGATIVE_ACCESS_RULES));
    assert!(!rights.property_saved(P::LOG_ENABLE));
    assert_eq!(storage.snapshot(), Some(positive_only(&kept)));
    // A write the object refuses itself keeps its own error, with no save.
    storage.fail.store(false, Ordering::SeqCst);
    assert_refused(
        write_positive(&mut rights, &[door_rule()]),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(storage.saves(), 1);
}

#[test]
fn saved_state_the_setters_refuse_refuses_the_object() {
    let rights = ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, 1).unwrap();
    let past_the_cap = vec![zone_rule(1); MAX_ACCESS_RULES + 1];
    let mut odd_specifier = zone_rule(1);
    odd_specifier.location_specifier = AccessRuleLocationSpecifier::from_raw(7);
    for (snapshot, code) in [
        (positive_only(&[door_rule()]), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            AccessRightsSnapshot {
                negative_access_rules: Some(vec![odd_specifier]),
                ..AccessRightsSnapshot::default()
            },
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            positive_only(&past_the_cap),
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ),
    ] {
        let storage = Arc::new(MemoryPersistence::default());
        storage.preload(rights, snapshot);
        let refused = AccessRightsObject::with_persistence(1, "AR", storage).map(|_| ());
        let class = if code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY {
            ErrorClass::RESOURCES
        } else {
            ErrorClass::PROPERTY
        };
        assert_refused(refused, class, code);
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_file() -> PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "rusty-bacnet-access-rights-{}-{serial}",
            std::process::id()
        ))
        .join("rules")
}

fn rights_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, instance).unwrap()
}

#[test]
fn file_persistence_round_trips_each_member_and_refuses_bad_files() {
    let path = temp_file();
    let storage = FileAccessRightsPersistence::new(&path).unwrap();
    assert_eq!(storage.path(), path);
    let rights = rights_oid(1);
    assert_eq!(storage.load(rights).unwrap(), None);
    // Access User 3 in Device 99 (#1393).
    let accompaniment = BACnetDeviceObjectReference {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap()),
        object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_USER, 3).unwrap(),
    };
    let full = AccessRightsSnapshot {
        positive_access_rules: Some(vec![zone_rule(1), grown_rule()]),
        negative_access_rules: Some(vec![zone_rule(2)]),
        enable: Some(false),
        accompaniment: Some(accompaniment.clone()),
    };
    // Each member may be absent, and an array may be empty.
    for saved in [
        full.clone(),
        positive_only(&[]),
        AccessRightsSnapshot {
            negative_access_rules: Some(vec![zone_rule(3)]),
            ..AccessRightsSnapshot::default()
        },
        AccessRightsSnapshot {
            enable: Some(true),
            ..AccessRightsSnapshot::default()
        },
        AccessRightsSnapshot {
            accompaniment: Some(accompaniment.object_identifier.into()),
            ..AccessRightsSnapshot::default()
        },
        AccessRightsSnapshot::default(),
    ] {
        storage.save(rights, &saved).unwrap();
        assert_eq!(storage.load(rights).unwrap(), Some(saved));
    }

    storage.save(rights, &full).unwrap();
    assert!(storage.load(rights_oid(2)).is_err());
    let good = std::fs::read(&path).unwrap();
    let header = &good[..12];
    let body = &good[12..];
    // The body is the BACnet encoding of the four members in order.
    let mut expected = vec![0x0E];
    for rule in [zone_rule(1), grown_rule()] {
        let mut buf = BytesMut::new();
        encode_access_rule(&mut buf, &rule);
        expected.extend_from_slice(&buf);
    }
    expected.extend_from_slice(&[0x0F, 0x1E]);
    let mut buf = BytesMut::new();
    encode_access_rule(&mut buf, &zone_rule(2));
    expected.extend_from_slice(&buf);
    expected.extend_from_slice(&[0x1F, 0x29, 0x00]);
    // Accompaniment framed by context tag 3: device [0], then object [1].
    let accompaniment_frame = [
        0x3E, 0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x08, 0xC0, 0x00, 0x03, 0x3F,
    ];
    expected.extend_from_slice(&accompaniment_frame);
    assert_eq!(body, expected);
    assert_eq!(&header[..8], b"RBNACR01");

    let with_body = |body: &[u8]| [header, body].concat();
    let enable_only = [0x29, 0x01];
    let negative_end = body.len() - 2 - accompaniment_frame.len();
    let negative_frame = &body[negative_end - (buf.len() + 2)..negative_end];
    let refusals = [
        // Cut short, inside a frame and inside a rule.
        with_body(&body[..body.len() - 1]),
        with_body(&body[..3]),
        // A frame that never closes.
        with_body(&[0x0E]),
        // Members out of order, and one repeated.
        with_body(&[&enable_only[..], negative_frame].concat()),
        with_body(&[&enable_only[..], &enable_only[..]].concat()),
        // An Enable that isn't a BOOLEAN of 0 or 1, and octets past the end.
        with_body(&[0x29, 0x02]),
        with_body(&[&enable_only[..], &[0x00]].concat()),
        // Accompaniment before Enable, repeated, cut short, unclosed, or
        // holding something other than one reference.
        with_body(&[&accompaniment_frame[..], &enable_only[..]].concat()),
        with_body(&[accompaniment_frame, accompaniment_frame].concat()),
        with_body(&accompaniment_frame[..8]),
        with_body(&accompaniment_frame[..11]),
        with_body(&[0x3E, 0x21, 0x01, 0x3F]),
        with_body(
            &[
                &accompaniment_frame[..11],
                &[0x1C, 0x08, 0xC0, 0x00, 0x03, 0x3F],
            ]
            .concat(),
        ),
        // Another object's identifier, and no valid header at all.
        [&header[..8], &rights_oid(9).encode()[..], body].concat(),
        b"not an access rights file".to_vec(),
    ];
    for bad in refusals {
        std::fs::write(&path, &bad).unwrap();
        assert!(storage.load(rights).is_err(), "{bad:02X?}");
    }
    // A Notification Class's file is another format.
    let class_file =
        crate::notification_class::FileNotificationClassPersistence::new(&path).unwrap();
    crate::notification_class::NotificationClassPersistence::save(
        &class_file,
        rights,
        &crate::notification_class::NotificationClassSnapshot::default(),
    )
    .unwrap();
    let refusal = storage.load(rights).unwrap_err().to_string();
    assert!(refusal.contains("has no valid header"), "{refusal}");

    assert!(FileAccessRightsPersistence::new("").is_err());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn file_persistence_keeps_rules_across_a_rebuild() {
    let path = temp_file();
    let storage = || {
        Arc::new(FileAccessRightsPersistence::new(&path).unwrap())
            as Arc<dyn AccessRightsPersistence>
    };
    let mut rights = AccessRightsObject::with_persistence(4, "AR", storage()).unwrap();
    rights.set_negative_access_rules([zone_rule(1)]).unwrap();
    write_positive(&mut rights, &[zone_rule(2)]).unwrap();
    write(
        &mut rights,
        P::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    drop(rights);
    let mut rebuilt = AccessRightsObject::with_persistence(4, "AR", storage()).unwrap();
    rebuilt.set_positive_access_rules([zone_rule(1)]).unwrap();
    assert_eq!(rebuilt.positive_access_rules(), [zone_rule(2)]);
    // The configured negative rules were never saved.
    assert!(rebuilt.negative_access_rules().is_empty());
    assert!(!rebuilt.enable());
    // No temporary file is left beside the rules.
    assert!(!path.with_file_name("rules.tmp").exists());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// The longest rule a write accepts: SPECIFIED members whose references
/// carry every optional part, with the widest numbers.
fn longest_rule() -> BACnetAccessRule {
    let widest = |object_type| {
        ObjectIdentifier::new(object_type, ObjectIdentifier::MAX_INSTANCE - 1).unwrap()
    };
    BACnetAccessRule::new(
        Some(BACnetDeviceObjectPropertyReference {
            object_identifier: widest(ObjectType::SCHEDULE),
            property_identifier: u32::MAX,
            property_array_index: Some(u32::MAX),
            device_identifier: Some(widest(ObjectType::DEVICE)),
        }),
        Some(BACnetDeviceObjectReference {
            device_identifier: Some(widest(ObjectType::DEVICE)),
            object_identifier: widest(ObjectType::ACCESS_POINT),
        }),
        true,
    )
}

#[test]
fn file_persistence_refuses_a_file_past_its_size_or_entry_caps() {
    use super::persistence::MAX_FILE_BYTES;
    let path = temp_file();
    let storage = FileAccessRightsPersistence::new(&path).unwrap();
    let rights = rights_oid(1);
    let refusal = |storage: &FileAccessRightsPersistence| storage.load(rights).unwrap_err();
    // Both arrays full of the longest rules fit the size cap, and load.
    let longest = longest_rule();
    check_access_rule(&longest).unwrap();
    let mut one = BytesMut::new();
    encode_access_rule(&mut one, &longest);
    assert_eq!(one.len(), 40);
    let full_array = vec![longest; MAX_ACCESS_RULES];
    // The longest Accompaniment too: one naming its device.
    let full = AccessRightsSnapshot {
        positive_access_rules: Some(full_array.clone()),
        negative_access_rules: Some(full_array.clone()),
        enable: Some(true),
        accompaniment: Some(BACnetDeviceObjectReference {
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap()),
            object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 5).unwrap(),
        }),
    };
    storage.save(rights, &full).unwrap();
    let file_len = std::fs::metadata(&path).unwrap().len();
    assert!(file_len < MAX_FILE_BYTES, "{file_len}");
    assert_eq!(storage.load(rights).unwrap(), Some(full.clone()));

    // One rule past the cap is refused. The backend saves what it is given,
    // so a file like this can only come from elsewhere.
    let mut too_many = full_array;
    too_many.push(zone_rule(1));
    storage
        .save(
            rights,
            &AccessRightsSnapshot {
                negative_access_rules: Some(too_many),
                ..AccessRightsSnapshot::default()
            },
        )
        .unwrap();
    assert!(
        refusal(&storage)
            .to_string()
            .contains("more entries than the cap")
    );

    // A file past the size cap is refused before any of it is decoded; one
    // at the cap is read, and here refused for what it holds.
    storage.save(rights, &full).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.resize(usize::try_from(MAX_FILE_BYTES).unwrap() + 1, 0);
    std::fs::write(&path, &bytes).unwrap();
    assert!(refusal(&storage).to_string().contains("too large"));
    bytes.pop();
    std::fs::write(&path, &bytes).unwrap();
    let at_cap = refusal(&storage).to_string();
    assert!(!at_cap.contains("too large"), "{at_cap}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

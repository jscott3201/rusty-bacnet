//! Access Rights Accompaniment (#1393): left out until the application sets
//! it, then served, listed and writable; the setter's and the writes'
//! refusals; and, with persistence, a written Accompaniment kept across a
//! restart and saved off the database lock like the rule arrays (#1392).

use super::test_storage::{MemoryPersistence, assert_refused, block_on, persistent, write};
use super::*;
use crate::durable::{DurableWrites, StageStep};
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use std::sync::Arc;
use std::sync::atomic::Ordering;

const UNSPECIFIED: u32 = ObjectIdentifier::MAX_INSTANCE;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn local(object_type: ObjectType, instance: u32) -> BACnetDeviceObjectReference {
    oid(object_type, instance).into()
}

fn remote(
    device: ObjectIdentifier,
    object_type: ObjectType,
    instance: u32,
) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(device),
        object_identifier: oid(object_type, instance),
    }
}

/// Access Credential 5 in Device 99: device [0], then object [1].
const REMOTE_CREDENTIAL: &[u8] = &[0x0C, 0x02, 0x00, 0x00, 0x63, 0x1C, 0x08, 0x00, 0x00, 0x05];

fn remote_credential() -> BACnetDeviceObjectReference {
    remote(
        oid(ObjectType::DEVICE, 99),
        ObjectType::ACCESS_CREDENTIAL,
        5,
    )
}

fn octets(bytes: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn assert_property_error<T>(result: Result<T, Error>, code: ErrorCode) {
    assert_refused(result.map(|_| ()), ErrorClass::PROPERTY, code);
}

/// Whether Accompaniment is in Property_List and the metadata, and how the
/// metadata describes it.
fn assert_listed(rights: &AccessRightsObject, listed: bool) {
    assert_eq!(rights.property_list().contains(&P::ACCOMPANIMENT), listed);
    let metadata = rights.property_metadata();
    let row = metadata
        .iter()
        .find(|row| row.property_identifier == P::ACCOMPANIMENT);
    assert_eq!(row.is_some(), listed);
    assert_eq!(rights.is_writable_property(P::ACCOMPANIMENT), listed);
    // Never required, and never an array.
    assert!(!rights.required_properties().contains(&P::ACCOMPANIMENT));
    assert!(!rights.is_array_property(P::ACCOMPANIMENT));
    if let Some(row) = row {
        assert_eq!(row.conformance, PropertyConformance::Optional);
        assert_eq!(row.write_capability, PropertyWriteCapability::Always);
        assert_eq!(row.presence_condition, None);
        // After Enable, just before Property_List.
        let order: Vec<_> = metadata.iter().map(|row| row.property_identifier).collect();
        assert_eq!(
            &order[order.len() - 3..],
            [P::LOG_ENABLE, P::ACCOMPANIMENT, P::PROPERTY_LIST]
        );
    }
}

#[test]
fn accompaniment_is_left_out_until_the_application_sets_it() {
    let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
    assert_eq!(rights.accompaniment(), None);
    assert_listed(&rights, false);
    assert_property_error(
        rights.read_property(P::ACCOMPANIMENT, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    // A write can't add the row, with or without an index.
    for index in [None, Some(1)] {
        assert_property_error(
            write(
                &mut rights,
                P::ACCOMPANIMENT,
                index,
                octets(REMOTE_CREDENTIAL),
            ),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert_eq!(rights.accompaniment(), None);

    rights.set_accompaniment(Some(remote_credential())).unwrap();
    assert_eq!(rights.accompaniment(), Some(&remote_credential()));
    assert_listed(&rights, true);
    assert_eq!(
        rights.read_property(P::ACCOMPANIMENT, None).unwrap(),
        octets(REMOTE_CREDENTIAL)
    );

    // Each object type Clause 12.34.11 names, here or in another device,
    // and an unspecified reference of any type, which asks for none. Each
    // reads back in its Clause 21 form.
    for (reference, read) in [
        (
            local(ObjectType::ACCESS_RIGHTS, 2),
            &[0x1C, 0x08, 0x80, 0x00, 0x02][..],
        ),
        (
            local(ObjectType::ACCESS_USER, 3),
            &[0x1C, 0x08, 0xC0, 0x00, 0x03][..],
        ),
        (
            local(ObjectType::ACCESS_CREDENTIAL, UNSPECIFIED),
            &[0x1C, 0x08, 0x3F, 0xFF, 0xFF][..],
        ),
        (
            remote(
                oid(ObjectType::DEVICE, UNSPECIFIED),
                ObjectType::ANALOG_VALUE,
                UNSPECIFIED,
            ),
            &[0x0C, 0x02, 0x3F, 0xFF, 0xFF, 0x1C, 0x00, 0xBF, 0xFF, 0xFF][..],
        ),
    ] {
        rights.set_accompaniment(Some(reference.clone())).unwrap();
        assert_eq!(rights.accompaniment(), Some(&reference));
        assert_eq!(
            rights.read_property(P::ACCOMPANIMENT, None).unwrap(),
            octets(read),
            "{reference:?}"
        );
    }

    // `None` takes the row out again.
    rights.set_accompaniment(None).unwrap();
    assert_eq!(rights.accompaniment(), None);
    assert_listed(&rights, false);
    assert_property_error(
        rights.read_property(P::ACCOMPANIMENT, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

/// The references the setter and the writes refuse with VALUE_OUT_OF_RANGE.
fn refused_references() -> Vec<BACnetDeviceObjectReference> {
    let not_a_device = oid(ObjectType::ANALOG_VALUE, 99);
    vec![
        // Objects Clause 12.34.11 gives no meaning to.
        local(ObjectType::ACCESS_POINT, 1),
        remote(oid(ObjectType::DEVICE, 99), ObjectType::ACCESS_ZONE, 1),
        // Only half unspecified, so not asking for no accompaniment.
        remote(
            oid(ObjectType::DEVICE, 99),
            ObjectType::ACCESS_POINT,
            UNSPECIFIED,
        ),
        // A device member that isn't a Device (#1285), even unspecified.
        remote(not_a_device, ObjectType::ACCESS_CREDENTIAL, 5),
        remote(
            oid(ObjectType::ANALOG_VALUE, UNSPECIFIED),
            ObjectType::ACCESS_CREDENTIAL,
            UNSPECIFIED,
        ),
    ]
}

#[test]
fn set_accompaniment_refuses_other_objects_and_non_device_devices() {
    let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
    for reference in refused_references() {
        assert_property_error(
            rights.set_accompaniment(Some(reference.clone())),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        // Refused while the row is out, it stays out.
        assert_eq!(rights.accompaniment(), None, "{reference:?}");
    }
    rights.set_accompaniment(Some(remote_credential())).unwrap();
    for reference in refused_references() {
        assert_property_error(
            rights.set_accompaniment(Some(reference.clone())),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            rights.accompaniment(),
            Some(&remote_credential()),
            "{reference:?}"
        );
    }
}

#[test]
fn a_served_accompaniment_takes_writes() {
    let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
    rights
        .set_accompaniment(Some(local(ObjectType::ACCESS_USER, 3)))
        .unwrap();
    // The octets WriteProperty carries.
    write(
        &mut rights,
        P::ACCOMPANIMENT,
        None,
        octets(REMOTE_CREDENTIAL),
    )
    .unwrap();
    assert_eq!(rights.accompaniment(), Some(&remote_credential()));
    // The form a read returns, split at each member, as a local write may
    // hand it back.
    write(
        &mut rights,
        P::ACCOMPANIMENT,
        None,
        PropertyValue::List(vec![
            octets(&REMOTE_CREDENTIAL[..5]),
            octets(&REMOTE_CREDENTIAL[5..]),
        ]),
    )
    .unwrap();
    assert_eq!(rights.accompaniment(), Some(&remote_credential()));
    // An unspecified reference asks for no accompaniment; the row stays.
    write(
        &mut rights,
        P::ACCOMPANIMENT,
        None,
        octets(&[0x1C, 0x08, 0x3F, 0xFF, 0xFF]),
    )
    .unwrap();
    assert_eq!(
        rights.accompaniment(),
        Some(&local(ObjectType::ACCESS_CREDENTIAL, UNSPECIFIED))
    );
    assert_listed(&rights, true);
}

#[test]
fn refused_accompaniment_writes_keep_the_reference() {
    let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
    rights.set_accompaniment(Some(remote_credential())).unwrap();
    let mut cases: Vec<(Option<u32>, PropertyValue, ErrorCode)> = refused_references()
        .into_iter()
        .map(|reference| {
            let mut buf = BytesMut::new();
            bacnet_encoding::constructed::encode_device_object_reference(&mut buf, &reference);
            (None, octets(&buf), ErrorCode::VALUE_OUT_OF_RANGE)
        })
        .collect();
    cases.extend([
        // Not a reference at all: other datatypes, and octets whose first
        // element can't open one. A NULL is refused here too; the server's
        // handlers turn that into a write that changes nothing (#1396).
        (None, PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        (
            None,
            PropertyValue::Boolean(true),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            None,
            PropertyValue::Unsigned(5),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (None, octets(&[0x00]), ErrorCode::INVALID_DATA_TYPE),
        (
            None,
            octets(&[0xC4, 0x08, 0x00, 0x00, 0x05]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // Opens as a reference but isn't exactly one.
        (
            None,
            octets(&REMOTE_CREDENTIAL[..7]),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            None,
            octets(&REMOTE_CREDENTIAL[..5]),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (
            None,
            octets(&[REMOTE_CREDENTIAL, &REMOTE_CREDENTIAL[5..]].concat()),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
        (None, octets(&[]), ErrorCode::INVALID_DATA_ENCODING),
        // Accompaniment is no array.
        (
            Some(0),
            PropertyValue::Unsigned(1),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            Some(1),
            octets(REMOTE_CREDENTIAL),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
    ]);
    for (index, value, code) in cases {
        let shown = format!("{index:?} {value:?}");
        assert_property_error(write(&mut rights, P::ACCOMPANIMENT, index, value), code);
        assert_eq!(
            rights.accompaniment(),
            Some(&remote_credential()),
            "{shown}"
        );
    }
}

#[test]
fn a_written_accompaniment_survives_a_rebuild_and_wins_over_the_setter() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    // Configuration alone is never saved.
    rights
        .set_accompaniment(Some(local(ObjectType::ACCESS_USER, 3)))
        .unwrap();
    assert!(!rights.property_saved(P::ACCOMPANIMENT));
    write(
        &mut rights,
        P::ACCOMPANIMENT,
        None,
        octets(REMOTE_CREDENTIAL),
    )
    .unwrap();
    assert!(rights.property_saved(P::ACCOMPANIMENT));
    assert_eq!(
        storage.snapshot(),
        Some(AccessRightsSnapshot {
            accompaniment: Some(remote_credential()),
            ..AccessRightsSnapshot::default()
        })
    );
    assert_eq!(storage.saves(), 1);
    drop(rights);

    // The saved reference serves the row though nothing configures one.
    let mut rebuilt = persistent(&storage);
    assert_eq!(rebuilt.accompaniment(), Some(&remote_credential()));
    assert!(rebuilt.property_saved(P::ACCOMPANIMENT));
    assert_listed(&rebuilt, true);
    assert_eq!(
        rebuilt.read_property(P::ACCOMPANIMENT, None).unwrap(),
        octets(REMOTE_CREDENTIAL)
    );
    // The setter still checks, but stores nothing, and can't take the row
    // out.
    rebuilt
        .set_accompaniment(Some(local(ObjectType::ACCESS_USER, 3)))
        .unwrap();
    rebuilt.set_accompaniment(None).unwrap();
    assert_property_error(
        rebuilt.set_accompaniment(Some(local(ObjectType::ACCESS_POINT, 1))),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(rebuilt.accompaniment(), Some(&remote_credential()));
    // The other properties were never written, so nothing else is saved.
    for property in [
        P::POSITIVE_ACCESS_RULES,
        P::NEGATIVE_ACCESS_RULES,
        P::LOG_ENABLE,
    ] {
        assert!(!rebuilt.property_saved(property), "{property:?}");
    }
}

#[test]
fn an_accompaniment_write_that_cannot_be_saved_is_refused_and_changes_nothing() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    rights
        .set_accompaniment(Some(local(ObjectType::ACCESS_USER, 3)))
        .unwrap();
    storage.fail.store(true, Ordering::SeqCst);
    assert_refused(
        write(
            &mut rights,
            P::ACCOMPANIMENT,
            None,
            octets(REMOTE_CREDENTIAL),
        ),
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    );
    assert_eq!(
        rights.accompaniment(),
        Some(&local(ObjectType::ACCESS_USER, 3))
    );
    assert!(!rights.property_saved(P::ACCOMPANIMENT));
    assert_eq!(storage.snapshot(), None);
}

#[test]
fn a_saved_accompaniment_the_setter_refuses_refuses_the_object() {
    let rights = oid(ObjectType::ACCESS_RIGHTS, 1);
    for reference in refused_references() {
        let storage = Arc::new(MemoryPersistence::default());
        storage.preload(
            rights,
            AccessRightsSnapshot {
                accompaniment: Some(reference.clone()),
                ..AccessRightsSnapshot::default()
            },
        );
        assert_property_error(
            AccessRightsObject::with_persistence(1, "AR", storage),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
}

#[test]
fn an_accompaniment_write_stages_only_while_the_row_is_served() {
    let storage = Arc::new(MemoryPersistence::default());
    let mut rights = persistent(&storage);
    let value = octets(REMOTE_CREDENTIAL);
    // Left out, the write would be refused, so nothing is staged.
    assert!(matches!(
        rights.stage_write(P::ACCOMPANIMENT, None, &value),
        StageStep::Skip
    ));
    rights
        .set_accompaniment(Some(local(ObjectType::ACCESS_USER, 3)))
        .unwrap();
    // Nor is a write it refuses.
    for (index, refused) in [(None, PropertyValue::Null), (Some(1), value.clone())] {
        assert!(matches!(
            rights.stage_write(P::ACCOMPANIMENT, index, &refused),
            StageStep::Skip
        ));
    }
    assert_eq!(storage.saves(), 0);

    let wait = match rights.stage_write(P::ACCOMPANIMENT, None, &value) {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    };
    block_on(&wait);
    // Saved, but not yet served.
    assert_eq!(
        rights.accompaniment(),
        Some(&local(ObjectType::ACCESS_USER, 3))
    );
    // The write takes the saved state without saving again.
    write(&mut rights, P::ACCOMPANIMENT, None, value).unwrap();
    rights.release_staged_write(&wait);
    rights.wait_for_saves();
    assert_eq!(rights.accompaniment(), Some(&remote_credential()));
    assert_eq!(storage.saves(), 1);
    assert_eq!(
        storage.snapshot().and_then(|saved| saved.accompaniment),
        Some(remote_credential())
    );
}

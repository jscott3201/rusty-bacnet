//! Regression tests for issue #116: WriteProperty / WritePropertyMultiple /
//! local-write routing of `OBJECT_NAME` through the `ObjectDatabase` name
//! index.
//!
//! Before the fix, `write_property(OBJECT_NAME, …)` mutated the object's name
//! field in place without touching the database secondary name index, so:
//!   - lookups still resolved the old name and missed the new name, and
//!   - a duplicate name could be introduced via WriteProperty.
//!
//! These tests pin the index invariants: uniqueness is enforced on write, a
//! successful rename frees the old name and reserves the new one, and a failed
//! WritePropertyMultiple restores the pre-transaction index state.

use super::*;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_types::primitives::PropertyValue;

/// Build a `WritePropertyRequest` body for `OBJECT_NAME` on `oid` set to `name`.
fn encode_name_write(oid: ObjectIdentifier, name: &str) -> Vec<u8> {
    let mut value_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value_buf, name).unwrap();
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
        property_value: value_buf.to_vec(),
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    buf.to_vec()
}

/// Two objects with distinct names live in the database.
fn db_with_two_bvs() -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let a = BinaryValueObject::new(1, "BV-A").unwrap();
    let b = BinaryValueObject::new(2, "BV-B").unwrap();
    db.add(Box::new(a)).unwrap();
    db.add(Box::new(b)).unwrap();
    let oid_a = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
    let oid_b = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 2).unwrap();
    (db, oid_a, oid_b)
}

#[test]
fn write_object_name_rejects_duplicate() {
    let (mut db, oid_a, _oid_b) = db_with_two_bvs();

    // Renaming A to "BV-B" (owned by B) must be rejected up front.
    let buf = encode_name_write(oid_a, "BV-B");
    let result = handle_write_property(&mut db, &buf);
    assert!(result.is_err(), "duplicate Object_Name must be rejected");

    // Index untouched: A is still "BV-A", B still owns "BV-B".
    assert!(db.find_by_name("BV-A").is_some());
    assert!(db.find_by_name("BV-B").is_some());
    assert_eq!(
        db.get(&oid_a).unwrap().object_name(),
        "BV-A",
        "rejected write must not mutate the object name"
    );
}

#[test]
fn write_object_name_rename_refreshes_index() {
    let (mut db, oid_a, oid_b) = db_with_two_bvs();

    // Rename A from "BV-A" to "BV-A2".
    let buf = encode_name_write(oid_a, "BV-A2");
    handle_write_property(&mut db, &buf).unwrap();

    // Old name freed, new name resolves to A.
    assert!(db.find_by_name("BV-A").is_none(), "old name must be freed");
    let found = db.find_by_name("BV-A2").expect("new name must resolve");
    assert_eq!(found.object_identifier(), oid_a);
    assert_eq!(db.get(&oid_a).unwrap().object_name(), "BV-A2");

    // A third object can now reclaim the freed old name.
    let c = BinaryValueObject::new(3, "BV-A").unwrap();
    db.add(Box::new(c)).unwrap();
    assert!(db.find_by_name("BV-A").is_some());

    // B was untouched by the rename.
    assert_eq!(db.get(&oid_b).unwrap().object_name(), "BV-B");
}

#[test]
fn write_object_name_empty_or_wrong_type_still_rejected() {
    let (mut db, oid_a, _oid_b) = db_with_two_bvs();

    // Empty name → VALUE_OUT_OF_RANGE (the object's own write route rejects it).
    let buf = encode_name_write(oid_a, "");
    assert!(handle_write_property(&mut db, &buf).is_err());
    assert_eq!(db.get(&oid_a).unwrap().object_name(), "BV-A");

    // Wrong type (Unsigned) → INVALID_DATA_TYPE, and the name index is left
    // intact (no spurious mapping created).
    let mut value_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut value_buf, 7);
    let request = WritePropertyRequest {
        object_identifier: oid_a,
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
        property_value: value_buf.to_vec(),
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    assert!(handle_write_property(&mut db, &buf).is_err());
    assert_eq!(db.get(&oid_a).unwrap().object_name(), "BV-A");
    assert!(db.find_by_name("BV-A").is_some());
}

#[test]
fn write_property_multiple_name_rename_refreshes_index() {
    let (mut db, oid_a, _oid_b) = db_with_two_bvs();

    // One spec renaming A to "BV-A2". A second spec writes a benign prop on the
    // same object so the multi-write path exercises commit + index refresh.
    let mut name_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut name_buf, "BV-A2").unwrap();
    let mut desc_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut desc_buf, "renamed").unwrap();

    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![bacnet_services::wpm::WriteAccessSpecification {
            object_identifier: oid_a,
            list_of_properties: vec![
                bacnet_services::common::BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: name_buf.to_vec(),
                    priority: None,
                },
                bacnet_services::common::BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    value: desc_buf.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    handle_write_property_multiple(&mut db, &buf).unwrap();

    assert!(db.find_by_name("BV-A").is_none());
    assert_eq!(db.get(&oid_a).unwrap().object_name(), "BV-A2");
    let desc = db
        .get(&oid_a)
        .unwrap()
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    match desc {
        PropertyValue::CharacterString(s) => assert_eq!(s, "renamed"),
        other => panic!("expected CharacterString, got {other:?}"),
    }
}

#[test]
fn write_property_multiple_rollback_restores_name_index() {
    let (mut db, oid_a, oid_b) = db_with_two_bvs();

    // First spec renames A to "BV-B" — but B already owns "BV-B", so the
    // duplicate-name check must reject the *whole* transaction before any
    // mutation. The object name and index are left as before.
    let mut name_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut name_buf, "BV-B").unwrap();

    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![bacnet_services::wpm::WriteAccessSpecification {
            object_identifier: oid_a,
            list_of_properties: vec![bacnet_services::common::BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
                value: name_buf.to_vec(),
                priority: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(
        handle_write_property_multiple(&mut db, &buf).is_err(),
        "duplicate Object_Name in WPM must be rejected"
    );
    assert_eq!(db.get(&oid_a).unwrap().object_name(), "BV-A");
    assert_eq!(db.get(&oid_b).unwrap().object_name(), "BV-B");
    assert!(db.find_by_name("BV-A").is_some());
    assert!(db.find_by_name("BV-B").is_some());

    // Now exercise true rollback: rename A to a *free* name ("BV-A2") followed
    // by an invalid write (wrong type for DESCRIPTION is not invalid, so use a
    // read-only property write to force a mid-transaction failure after the
    // successful rename). The rollback must restore A's name AND the index.
    let mut good_name_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut good_name_buf, "BV-A2").unwrap();
    // OBJECT_TYPE is read-only → write_property returns WRITE_ACCESS_DENIED.
    let mut bad_type_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut bad_type_buf, 5);

    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![bacnet_services::wpm::WriteAccessSpecification {
            object_identifier: oid_a,
            list_of_properties: vec![
                bacnet_services::common::BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: good_name_buf.to_vec(),
                    priority: None,
                },
                bacnet_services::common::BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_TYPE,
                    property_array_index: None,
                    value: bad_type_buf.to_vec(),
                    priority: None,
                },
            ],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    assert!(
        handle_write_property_multiple(&mut db, &buf).is_err(),
        "read-only write must fail the transaction"
    );

    // Rollback restored A's name field AND resynced the index back: "BV-A2"
    // must no longer map to A, and "BV-A" must resolve again.
    assert_eq!(
        db.get(&oid_a).unwrap().object_name(),
        "BV-A",
        "rollback must restore the object name"
    );
    assert!(
        db.find_by_name("BV-A2").is_none(),
        "rollback must free the rolled-back new name in the index"
    );
    assert!(
        db.find_by_name("BV-A").is_some(),
        "rollback must restore the old name in the index"
    );
}

#[test]
fn remove_after_rename_frees_current_name_only() {
    let (mut db, oid_a, _oid_b) = db_with_two_bvs();

    // Rename A → "BV-A2", then remove A. Only the *current* name should be
    // freed (the old name was already freed at rename time).
    let buf = encode_name_write(oid_a, "BV-A2");
    handle_write_property(&mut db, &buf).unwrap();
    assert!(db.find_by_name("BV-A").is_none());
    assert!(db.find_by_name("BV-A2").is_some());

    db.remove(&oid_a);
    assert!(
        db.find_by_name("BV-A2").is_none(),
        "remove frees current name"
    );
    // The freed old name stays free.
    assert!(db.find_by_name("BV-A").is_none());
}

#[test]
fn write_object_name_noop_rename_to_same_name_succeeds() {
    // Renaming an object to its own current name is a no-op that must succeed:
    // check_name_available treats a name already owned by `oid` as available.
    let (mut db, oid_a, _oid_b) = db_with_two_bvs();
    let buf = encode_name_write(oid_a, "BV-A");
    handle_write_property(&mut db, &buf).unwrap();
    assert_eq!(db.get(&oid_a).unwrap().object_name(), "BV-A");
    assert!(db.find_by_name("BV-A").is_some());
    // B is unaffected.
    assert!(db.find_by_name("BV-B").is_some());
}

#[test]
fn write_property_multiple_cross_object_name_move() {
    // A single WPM that renames A → "BV-A2" and B → "BV-A" (B takes A's freed
    // old name). This must succeed because each check runs inside the commit
    // loop AFTER prior writes have refreshed the index, so B's check sees
    // "BV-A" as free.
    let (mut db, oid_a, oid_b) = db_with_two_bvs();

    let mut a_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut a_buf, "BV-A2").unwrap();
    let mut b_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut b_buf, "BV-A").unwrap();

    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![
            bacnet_services::wpm::WriteAccessSpecification {
                object_identifier: oid_a,
                list_of_properties: vec![bacnet_services::common::BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: a_buf.to_vec(),
                    priority: None,
                }],
            },
            bacnet_services::wpm::WriteAccessSpecification {
                object_identifier: oid_b,
                list_of_properties: vec![bacnet_services::common::BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                    value: b_buf.to_vec(),
                    priority: None,
                }],
            },
        ],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);

    handle_write_property_multiple(&mut db, &buf).unwrap();
    assert_eq!(db.get(&oid_a).unwrap().object_name(), "BV-A2");
    assert_eq!(db.get(&oid_b).unwrap().object_name(), "BV-A");
    assert_eq!(db.find_by_name("BV-A").unwrap().object_identifier(), oid_b);
    assert_eq!(db.find_by_name("BV-A2").unwrap().object_identifier(), oid_a);
    // B's old name is freed.
    assert!(db.find_by_name("BV-B").is_none());
}

#[test]
fn create_object_with_object_name_initial_value_refreshes_index() {
    // CreateObject with an OBJECT_NAME initial value must route through the
    // database name index: the created object is added under a default name,
    // then the initial OBJECT_NAME write renames it and the index must follow.
    let mut db = ObjectDatabase::new();

    let mut name_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut name_buf, "Custom-Name").unwrap();

    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
        list_of_initial_values: vec![bacnet_services::common::BACnetPropertyValue {
            property_identifier: PropertyIdentifier::OBJECT_NAME,
            property_array_index: None,
            value: name_buf.to_vec(),
            priority: None,
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    let mut ack_buf = BytesMut::new();
    handle_create_object(&mut db, &buf, &mut ack_buf).unwrap();

    // The created object must be findable by its initial Object_Name, and the
    // default name must be freed.
    let obj = db
        .find_by_name("Custom-Name")
        .expect("initial name resolves");
    let created_oid = obj.object_identifier();
    assert_eq!(db.get(&created_oid).unwrap().object_name(), "Custom-Name");
    assert!(
        db.find_by_name(&format!("{:?}-{}", ObjectType::BINARY_VALUE, 1))
            .is_none(),
        "default name must be freed after rename"
    );
}

#[test]
fn create_object_duplicate_object_name_initial_value_rejected_and_rolled_back() {
    // CreateObject with an OBJECT_NAME initial value that collides with an
    // existing object must be rejected, and the created object removed (so the
    // index and object count are unchanged).
    let (mut db, _oid_a, _oid_b) = db_with_two_bvs();
    let before_len = db.len();

    let mut name_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut name_buf, "BV-B").unwrap();

    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
        list_of_initial_values: vec![bacnet_services::common::BACnetPropertyValue {
            property_identifier: PropertyIdentifier::OBJECT_NAME,
            property_array_index: None,
            value: name_buf.to_vec(),
            priority: None,
        }],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    assert!(
        handle_create_object(&mut db, &buf, &mut BytesMut::new()).is_err(),
        "duplicate Object_Name initial value must be rejected"
    );
    assert_eq!(
        db.len(),
        before_len,
        "created object must be removed on failure"
    );
    // The colliding name still maps only to the original owner.
    assert!(db.find_by_name("BV-B").is_some());
}

#[test]
fn create_object_rename_then_later_failure_removes_renamed_object() {
    // An OBJECT_NAME initial value renames the created object (refreshing the
    // index), then a *subsequent* initial value fails. The created object must
    // be removed, and ObjectDatabase::remove must free the *current* (renamed)
    // name — not the stale default name — so the renamed name is reclaimable.
    let mut db = ObjectDatabase::new();
    let before_len = db.len();

    let mut name_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut name_buf, "Fresh-Name").unwrap();
    // OBJECT_TYPE is read-only → write_property returns WRITE_ACCESS_DENIED,
    // failing the transaction AFTER the OBJECT_NAME rename committed.
    let mut bad_type_buf = BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut bad_type_buf, 5);

    let req = CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
        list_of_initial_values: vec![
            bacnet_services::common::BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
                value: name_buf.to_vec(),
                priority: None,
            },
            bacnet_services::common::BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OBJECT_TYPE,
                property_array_index: None,
                value: bad_type_buf.to_vec(),
                priority: None,
            },
        ],
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);

    assert!(
        handle_create_object(&mut db, &buf, &mut BytesMut::new()).is_err(),
        "read-only initial value must fail the create"
    );
    assert_eq!(
        db.len(),
        before_len,
        "created object must be removed on failure"
    );
    // The renamed name was freed by remove (not stranded in the index), and a
    // new object can reclaim it.
    assert!(
        db.find_by_name("Fresh-Name").is_none(),
        "remove must free the current (renamed) name"
    );
    db.add(Box::new(BinaryValueObject::new(9, "Fresh-Name").unwrap()))
        .unwrap();
    assert!(db.find_by_name("Fresh-Name").is_some());
}

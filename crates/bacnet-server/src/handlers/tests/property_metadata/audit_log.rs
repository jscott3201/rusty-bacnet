use super::*;
use bacnet_objects::{
    audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot},
    traits::BACnetObject,
};
use std::sync::{Arc, Mutex};
use PropertyIdentifier as P;

#[derive(Default)]
struct MemoryPersistence(Mutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn audit_log_object() -> AuditLogObject {
    AuditLogObject::new(7, "AL-7", 4, Arc::new(MemoryPersistence::default())).unwrap()
}

#[test]
fn rpm_audit_log_forwarding_optional_metadata_and_member_of_wire() {
    let mut object = audit_log_object();
    object.set_member_of(Some(
        bacnet_types::constructed::BACnetDeviceObjectReference {
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 20).unwrap()),
            object_identifier: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 8).unwrap(),
        },
    ));
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    assert_rpm_selector_bytes(
        &db,
        oid,
        P::OPTIONAL,
        &[
            P::DESCRIPTION,
            P::MEMBER_OF,
            P::DELETE_ON_FORWARD,
            P::ISSUE_CONFIRMED_NOTIFICATIONS,
            P::RELIABILITY,
        ],
    );
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut value,
        &db.get(&oid)
            .unwrap()
            .read_property(P::MEMBER_OF, None)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(&value[..], &[0x0c, 0x02, 0, 0, 20, 0x1c, 0x0f, 0x40, 0, 8]);
}

#[test]
fn rpm_audit_log_metadata_selectors_preserve_bytes_and_budgets() {
    // Independent fixtures preserve the legacy projection order; the REQUIRED
    // set replaces the universal four with the Clause 12.64 R/W rows.
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::LOG_ENABLE,
        P::BUFFER_SIZE,
        P::RECORD_COUNT,
        P::TOTAL_RECORD_COUNT,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::LOG_ENABLE,
        P::BUFFER_SIZE,
        P::RECORD_COUNT,
        P::TOTAL_RECORD_COUNT,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
    ];
    let optional = [P::DESCRIPTION];
    let object = audit_log_object();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    for (selector, expected) in [
        (P::ALL, all.as_slice()),
        (P::REQUIRED, required.as_slice()),
        (P::OPTIONAL, optional.as_slice()),
        (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
    ] {
        assert_rpm_selector_bytes(&db, oid, selector, expected);
    }
}

#[test]
fn rpm_audit_log_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::AUDIT_LOG, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::AUDIT_LOG),
        ObjectSpecifier::Identifier(oid),
    ] {
        let mut db = ObjectDatabase::new();
        let mut request = BytesMut::new();
        CreateObjectRequest {
            object_specifier,
            list_of_initial_values: vec![],
        }
        .encode(&mut request);
        let mut response = BytesMut::new();
        let result = handle_create_object(&mut db, &request, &mut response);
        assert!(matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::OBJECT.to_raw() as u32
                && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32));
        assert!(response.is_empty());
        assert!(db.is_empty());
    }
}

#[test]
fn audit_log_delete_object_removes_log() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    let mut db = ObjectDatabase::new();
    db.add(Box::new(audit_log_object())).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::AUDIT_LOG, 7).unwrap();
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut request);
    handle_delete_object(&mut db, &request).unwrap();
    assert!(db.get(&oid).is_none());
}

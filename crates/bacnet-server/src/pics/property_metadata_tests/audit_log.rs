use super::*;
use bacnet_objects::{
    audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot},
    traits::BACnetObject,
};
use bacnet_types::error::Error;
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

#[test]
fn pics_audit_log_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in projection order.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::LOG_ENABLE, false, true),
        (P::BUFFER_SIZE, false, false),
        (P::RECORD_COUNT, false, false),
        (P::TOTAL_RECORD_COUNT, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    let object = AuditLogObject::new(7, "AL-7", 4, Arc::new(MemoryPersistence::default())).unwrap();
    let required = object.required_properties();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(pics.supported_object_types.len(), 1);
    let support = &pics.supported_object_types[0];
    assert_eq!(support.object_type, ObjectType::AUDIT_LOG);
    assert!(!support.createable);
    assert!(support.deleteable);
    let rows: Vec<_> = support
        .supported_properties
        .iter()
        .map(|row| {
            assert!(row.access.readable);
            (row.property_id, row.access.optional, row.access.writable)
        })
        .collect();
    assert_eq!(rows, expected);
    assert_eq!(
        rows.iter()
            .filter_map(|&(p, optional, _)| (!optional).then_some(p))
            .collect::<Vec<_>>(),
        required.as_ref()
    );
}

#[test]
fn pics_audit_log_forwarding_metadata_is_optional_and_read_only() {
    let mut object =
        AuditLogObject::new(7, "AL-7", 4, Arc::new(MemoryPersistence::default())).unwrap();
    object.set_member_of(Some(
        bacnet_types::constructed::BACnetDeviceObjectReference {
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 20).unwrap()),
            object_identifier: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 8).unwrap(),
        },
    ));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    for p in [
        P::MEMBER_OF,
        P::DELETE_ON_FORWARD,
        P::ISSUE_CONFIRMED_NOTIFICATIONS,
        P::RELIABILITY,
    ] {
        let row = property_support(&pics, ObjectType::AUDIT_LOG, p);
        assert!(row.access.readable && row.access.optional && !row.access.writable);
    }
}

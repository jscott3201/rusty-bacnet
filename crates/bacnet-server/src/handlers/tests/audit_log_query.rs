use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_objects::audit::{
    AuditLogObject, AuditLogPersistence, AuditLogQueryPage, AuditLogSnapshot, AuditLogStorage,
};
use bacnet_services::audit::{AuditLogQueryRequest, BACnetAuditLogQueryParameters};
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditOperation, ErrorClass, ErrorCode};
use bacnet_types::primitives::{Date, Time};

use super::*;

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

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn audit_record(target: ObjectIdentifier) -> BACnetAuditLogRecord {
    let source = oid(ObjectType::DEVICE, 1);
    BACnetAuditLogRecord {
        timestamp: (
            Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
        ),
        datum: BACnetAuditLogDatum::AuditNotification(BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: None,
            source_device: BACnetRecipient::Device(source),
            source_object: None,
            operation: AuditOperation::READ,
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(target),
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        }),
    }
}

fn request(audit_log: ObjectIdentifier) -> AuditLogQueryRequest {
    AuditLogQueryRequest {
        audit_log,
        query_parameters: BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: oid(ObjectType::DEVICE, 1),
            source_device_address: None,
            source_object_identifier: None,
            operations: None,
            successful_actions_only: false,
        },
        start_at_sequence_number: None,
        requested_count: 10,
    }
}

fn encode_request(request: &AuditLogQueryRequest) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    request.try_encode(&mut encoded).unwrap();
    encoded.to_vec()
}

fn assert_protocol(error: Error, class: ErrorClass, code: ErrorCode) {
    assert!(matches!(
        error,
        Error::Protocol { class: actual_class, code: actual_code }
            if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32
    ));
}

#[test]
fn handler_decodes_then_returns_an_owned_page_from_real_audit_storage() {
    let audit_oid = oid(ObjectType::AUDIT_LOG, 7);
    let target = oid(ObjectType::DEVICE, 2);
    let mut log =
        AuditLogObject::new(7, "Audit-7", 4, Arc::new(MemoryPersistence::default())).unwrap();
    log.add_record(audit_record(target)).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(log)).unwrap();

    let (returned_oid, page) =
        handle_audit_log_query(&db, &encode_request(&request(audit_oid))).unwrap();
    assert_eq!(returned_oid, audit_oid);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].sequence_number, 1);
    assert!(page.no_more_items);
}

#[test]
fn missing_or_non_audit_identifiers_map_to_unknown_object() {
    let db = make_db_with_ai();
    for requested in [
        oid(ObjectType::AUDIT_LOG, 99),
        oid(ObjectType::ANALOG_INPUT, 1),
    ] {
        let error = handle_audit_log_query(&db, &encode_request(&request(requested))).unwrap_err();
        assert_protocol(error, ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    }
}

struct AuditWithoutCapability {
    oid: ObjectIdentifier,
}

impl BACnetObject for AuditWithoutCapability {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "AuditWithoutCapability"
    }

    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
        })
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

#[test]
fn audit_typed_object_without_capability_maps_to_optional_functionality() {
    let audit_oid = oid(ObjectType::AUDIT_LOG, 8);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AuditWithoutCapability { oid: audit_oid }))
        .unwrap();

    let error = handle_audit_log_query(&db, &encode_request(&request(audit_oid))).unwrap_err();
    assert_protocol(
        error,
        ErrorClass::SERVICES,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}

struct SpyAuditStorage {
    query_count: AtomicUsize,
}

impl AuditLogStorage for SpyAuditStorage {
    fn query(
        &self,
        _parameters: &BACnetAuditLogQueryParameters,
        _start_at_sequence_number: Option<u32>,
        _requested_count: u16,
    ) -> AuditLogQueryPage {
        self.query_count.fetch_add(1, Ordering::SeqCst);
        AuditLogQueryPage {
            records: Vec::new(),
            no_more_items: true,
        }
    }
}

struct SpyAuditObject {
    oid: ObjectIdentifier,
    storage: Arc<SpyAuditStorage>,
}

impl BACnetObject for SpyAuditObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "SpyAuditObject"
    }

    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        unreachable!("AuditLogQuery must not inspect properties")
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        unreachable!("AuditLogQuery is read-only")
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }

    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        Some(self.storage.as_ref())
    }
}

#[test]
fn malformed_payload_fails_before_audit_storage_is_inspected() {
    let audit_oid = oid(ObjectType::AUDIT_LOG, 9);
    let storage = Arc::new(SpyAuditStorage {
        query_count: AtomicUsize::new(0),
    });
    let mut db = ObjectDatabase::new();
    db.add(Box::new(SpyAuditObject {
        oid: audit_oid,
        storage: Arc::clone(&storage),
    }))
    .unwrap();
    let mut malformed = encode_request(&request(audit_oid));
    malformed.push(0);

    assert!(handle_audit_log_query(&db, &malformed).is_err());
    assert_eq!(storage.query_count.load(Ordering::SeqCst), 0);
}

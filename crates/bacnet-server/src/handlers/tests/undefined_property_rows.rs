//! Rows the Clause 12 property tables don't define, on the wire (#1064).
//!
//! For each object below, the listed rows are gone from Property_List, from
//! an RPM ALL expansion and from the PICS, and ReadProperty, RPM and
//! WriteProperty on them fail with PROPERTY / UNKNOWN_PROPERTY without
//! changing the object.

use super::*;
use bacnet_objects::access_control::{
    AccessCredentialObject, AccessPointObject, AccessRightsObject, AccessUserObject,
    AccessZoneObject,
};
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::file::FileObject;
use bacnet_objects::group::{GroupObject, StructuredViewObject};
use bacnet_objects::load_control::LoadControlObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use PropertyIdentifier as P;

const STATUS_ROWS: &[P] = &[P::STATUS_FLAGS, P::RELIABILITY, P::OUT_OF_SERVICE];

/// Each object paired with the rows its table doesn't define.
fn undefined_rows() -> Vec<(Box<dyn BACnetObject>, &'static [P])> {
    vec![
        (
            Box::new(EventLogObject::new(1, "EL-1", 8).unwrap()),
            &[P::OUT_OF_SERVICE, P::LOG_INTERVAL],
        ),
        (
            Box::new(CommandObject::new(1, "CMD-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(
                EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap(),
            ),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(NotificationClass::new(1, "NC-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(LoadControlObject::new(1, "LC-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(FileObject::new(1, "FILE-1", "raw").unwrap()),
            STATUS_ROWS,
        ),
        (Box::new(GroupObject::new(1, "G-1").unwrap()), STATUS_ROWS),
        (
            Box::new(StructuredViewObject::new(1, "SV-1").unwrap()),
            STATUS_ROWS,
        ),
        (
            Box::new(AveragingObject::new(1, "AVG-1").unwrap()),
            &[
                P::PRESENT_VALUE,
                P::STATUS_FLAGS,
                P::EVENT_STATE,
                P::RELIABILITY,
                P::OUT_OF_SERVICE,
            ],
        ),
        (
            Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(AccessRightsObject::new(1, "AR-1").unwrap()),
            &[P::OUT_OF_SERVICE],
        ),
        (
            Box::new(AccessUserObject::new(1, "USER-1").unwrap()),
            &[
                P::PRESENT_VALUE,
                P::ASSIGNED_ACCESS_RIGHTS,
                P::OUT_OF_SERVICE,
            ],
        ),
        (
            Box::new(AccessPointObject::new(1, "AP-1").unwrap()),
            &[P::PRESENT_VALUE],
        ),
        (
            Box::new(AccessZoneObject::new(1, "AZ-1").unwrap()),
            &[P::PRESENT_VALUE, P::ACCESS_DOORS],
        ),
    ]
}

fn db_with(object: Box<dyn BACnetObject>) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    (db, oid)
}

fn assert_unknown_property<T: std::fmt::Debug>(result: Result<T, Error>, context: &str) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::UNKNOWN_PROPERTY),
        "{context}: expected PROPERTY/UNKNOWN_PROPERTY, got {result:?}"
    );
}

fn read_property(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response)?;
    Ok(ReadPropertyACK::decode(&response).unwrap().property_value)
}

fn write_property(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    value: &PropertyValue,
) -> Result<(), Error> {
    let mut property_value = BytesMut::new();
    encode_property_value(&mut property_value, value).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: property_value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

/// The RPM results for `references` on `oid`.
fn rpm(db: &ObjectDatabase, oid: ObjectIdentifier, references: &[P]) -> Vec<ReadResultElement> {
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: references
                .iter()
                .map(|&property_identifier| PropertyReference {
                    property_identifier,
                    property_array_index: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut response = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut response).unwrap();
    let mut ack = ReadPropertyMultipleACK::decode(&response).unwrap();
    ack.list_of_read_access_results.remove(0).list_of_results
}

#[test]
fn rp_rpm_and_wp_find_no_undefined_row() {
    for (object, rows) in undefined_rows() {
        let (mut db, oid) = db_with(object);
        let kind = oid.object_type();
        let property_list = read_property(&db, oid, P::PROPERTY_LIST).unwrap();
        for &property in rows {
            let context = format!("{kind:?} {property:?}");
            assert_unknown_property(read_property(&db, oid, property), &context);
            let results = rpm(&db, oid, &[property]);
            assert_eq!(
                results[0].error,
                Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
                "{context} over RPM"
            );
            for value in [
                PropertyValue::Boolean(true),
                PropertyValue::Enumerated(1),
                PropertyValue::Unsigned(1),
                PropertyValue::Real(1.0),
            ] {
                assert_unknown_property(write_property(&mut db, oid, property, &value), &context);
            }
        }
        assert_eq!(
            read_property(&db, oid, P::PROPERTY_LIST).unwrap(),
            property_list,
            "{kind:?}: a refused write changed Property_List"
        );
    }
}

#[test]
fn undefined_rows_are_absent_from_property_list_rpm_all_and_the_pics() {
    use crate::pics::{generate_pics, PicsConfig};
    use crate::server::ServerConfig;

    for (object, rows) in undefined_rows() {
        let (db, oid) = db_with(object);
        let kind = oid.object_type();
        let listed = match db.get(&oid).unwrap().read_property(P::PROPERTY_LIST, None) {
            Ok(PropertyValue::List(items)) => items,
            other => panic!("{kind:?} Property_List: {other:?}"),
        };
        let expanded: Vec<_> = rpm(&db, oid, &[P::ALL])
            .into_iter()
            .map(|result| {
                // An Event Log's Log_Buffer is present but read only by
                // ReadRange (Clause 12.27.13), so ALL reports it inline.
                let refused =
                    kind == ObjectType::EVENT_LOG && result.property_identifier == P::LOG_BUFFER;
                assert_eq!(
                    result.error,
                    refused.then_some((ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED)),
                    "{kind:?} RPM ALL inline error"
                );
                result.property_identifier
            })
            .collect();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let support = pics
            .supported_object_types
            .iter()
            .find(|support| support.object_type == kind)
            .unwrap();
        for &property in rows {
            let context = format!("{kind:?} {property:?}");
            assert!(
                !listed.contains(&PropertyValue::Enumerated(property.to_raw())),
                "{context} in Property_List"
            );
            assert!(!expanded.contains(&property), "{context} in RPM ALL");
            assert!(
                !support
                    .supported_properties
                    .iter()
                    .any(|row| row.property_id == property),
                "{context} in the PICS"
            );
        }
    }
}

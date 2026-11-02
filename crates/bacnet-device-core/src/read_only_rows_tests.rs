//! Every row the catalogue marks read-only refuses a network write (#2258).
//!
//! A row's write capability describes the object's own write route; the
//! generic WriteProperty path doesn't gate on it, and leaves the refusal to
//! the object. So a row that says read-only while its object takes the
//! write (Program_State did) is writable on the wire and listed read-only in
//! the PICS. This test writes each such row's current value back through
//! [`write_property`], with no priority, and expects WRITE_ACCESS_DENIED.

use crate::write::write_property;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::catalogue::representatives;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

fn denied(error: &Error) -> bool {
    matches!(
        error,
        Error::Remote(RemoteError::Error { class, code })
            if *class == ErrorClass::PROPERTY && *code == ErrorCode::WRITE_ACCESS_DENIED
    )
}

/// The octets written to `property`: its current value, so only the
/// capability can refuse it. An empty value, which no single-valued decode
/// takes, and a row with no whole read (a log buffer) get a placeholder.
fn write_back(object: &dyn BACnetObject, property: PropertyIdentifier) -> Vec<u8> {
    let mut octets = BytesMut::new();
    if let Ok(value) = object.read_property(property, None) {
        encode_property_value(&mut octets, &value).expect("served values encode");
    }
    if octets.is_empty() {
        encode_property_value(&mut octets, &PropertyValue::Unsigned(0)).unwrap();
    }
    octets.to_vec()
}

#[test]
fn every_read_only_row_refuses_a_network_write() {
    let mut db = ObjectDatabase::new();
    for object in representatives() {
        db.add(object).unwrap();
    }
    let mut rows = Vec::new();
    for (oid, object) in db.iter_objects() {
        for row in object.property_metadata().iter() {
            if row.write_capability.is_writable() {
                continue;
            }
            rows.push((
                oid,
                row.property_identifier,
                write_back(object, row.property_identifier),
            ));
        }
    }
    let mut failures = Vec::new();
    for (oid, property, octets) in &rows {
        let mut request = BytesMut::new();
        WritePropertyRequest {
            object_identifier: *oid,
            property_identifier: *property,
            property_array_index: None,
            property_value: octets.to_vec(),
            priority: None,
        }
        .encode(&mut request)
        .unwrap();
        match write_property(&mut db, &request, None, None, None) {
            Err(error) if denied(&error) => {}
            outcome => failures.push(format!(
                "{} {}: {property}: {outcome:?}",
                oid.object_type(),
                oid.instance_number()
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(rows.len() > 500, "only {} read-only rows", rows.len());
}

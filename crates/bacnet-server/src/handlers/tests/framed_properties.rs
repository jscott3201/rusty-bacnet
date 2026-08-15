//! Wire-level integration for structured properties (#154, #152): conformant
//! framed writes round-trip identically, while retained legacy flat forms are
//! validated before mutation.

use super::*;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_types::constructed::{
    BACnetDestination, BACnetEventParameter, BACnetRecipient, FaultParameters,
};
use bacnet_types::primitives::Time;

fn framed_event_parameters() -> Vec<u8> {
    let params = BACnetEventParameter::OutOfRange {
        time_delay: 5,
        low_limit: 10.0,
        high_limit: 90.0,
        deadband: 1.0,
    };
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut buf, &params);
    buf.to_vec()
}

fn framed_fault_parameters() -> Vec<u8> {
    let fp = FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 100.0,
    };
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_fault_parameters(&mut buf, &fp).unwrap();
    buf.to_vec()
}

fn framed_recipient_list() -> (Vec<u8>, Vec<BACnetDestination>) {
    let midnight = Time {
        hour: 0,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let end_of_day = Time {
        hour: 23,
        minute: 59,
        second: 59,
        hundredths: 99,
    };
    let device_entry = BACnetDestination {
        valid_days: 0b0111_1111,
        from_time: midnight,
        to_time: end_of_day,
        recipient: BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap()),
        process_identifier: 1,
        issue_confirmed_notifications: true,
        transitions: 0b0000_0111,
    };
    let address_entry = BACnetDestination {
        valid_days: 0b0111_1111,
        from_time: midnight,
        to_time: end_of_day,
        recipient: BACnetRecipient::Address(bacnet_types::constructed::BACnetAddress {
            network_number: 0xBAC0,
            mac_address: bacnet_types::MacAddr::from_slice(&[192, 168, 1, 100, 0xBA, 0xC0]),
        }),
        process_identifier: 42,
        issue_confirmed_notifications: false,
        transitions: 0b0000_0111,
    };
    let destinations = vec![device_entry, address_entry];
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(&mut buf, &destinations);
    (buf.to_vec(), destinations)
}

fn write_framed(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    framed: Vec<u8>,
) -> Result<(), Error> {
    let request = WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: framed,
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_write_property(db, &buf).map(|_| ())
}

fn read_raw(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
    let request = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    let mut ack_buf = BytesMut::new();
    handle_read_property(db, &buf, &mut ack_buf).unwrap();
    ReadPropertyACK::decode(&ack_buf.to_vec())
        .unwrap()
        .property_value
}

#[test]
fn event_parameters_framed_wire_round_trip() {
    let mut db = ObjectDatabase::new();
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let framed = framed_event_parameters();
    write_framed(
        &mut db,
        oid,
        PropertyIdentifier::EVENT_PARAMETERS,
        framed.clone(),
    )
    .unwrap();
    // The identical framed bytes come back over ReadProperty.
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::EVENT_PARAMETERS),
        framed
    );
}

#[test]
fn legacy_event_parameters_wire_write_is_canonicalized() {
    let mut db = ObjectDatabase::new();
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let payload = vec![1, 0xff, 0xff, 0x3f, 2];
    let mut historical = vec![0xfe, 0xff];
    historical.extend_from_slice(&payload);
    historical.extend_from_slice(&[0xff, 0xff]);
    write_framed(
        &mut db,
        oid,
        PropertyIdentifier::EVENT_PARAMETERS,
        historical,
    )
    .unwrap();

    let mut canonical = BytesMut::new();
    bacnet_encoding::primitives::encode_app_octet_string(&mut canonical, &payload);
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::EVENT_PARAMETERS),
        canonical.to_vec()
    );
}

#[test]
fn fault_parameters_framed_wire_round_trip() {
    let mut db = ObjectDatabase::new();
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let framed = framed_fault_parameters();
    write_framed(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_PARAMETERS,
        framed.clone(),
    )
    .unwrap();
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::FAULT_PARAMETERS),
        framed
    );
}

#[test]
fn malformed_legacy_fault_parameters_preserve_existing_value() {
    let mut db = ObjectDatabase::new();
    let mut ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    ee.set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
        min_normal: 0.0,
        max_normal: 100.0,
    }));
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();
    let before = read_raw(&db, oid, PropertyIdentifier::FAULT_PARAMETERS);

    let mut oversized_tag = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut oversized_tag, 256);
    bacnet_encoding::primitives::encode_app_unsigned(&mut oversized_tag, 1);
    assert!(write_framed(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_PARAMETERS,
        oversized_tag.to_vec(),
    )
    .is_err());
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::FAULT_PARAMETERS),
        before
    );

    let mut trailing_value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut trailing_value, 0);
    bacnet_encoding::primitives::encode_app_boolean(&mut trailing_value, true);
    assert!(write_framed(
        &mut db,
        oid,
        PropertyIdentifier::FAULT_PARAMETERS,
        trailing_value.to_vec(),
    )
    .is_err());
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::FAULT_PARAMETERS),
        before
    );
}

#[test]
fn recipient_list_framed_wire_round_trip() {
    let mut db = ObjectDatabase::new();
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let oid = nc.object_identifier();
    db.add(Box::new(nc)).unwrap();

    let (framed, destinations) = framed_recipient_list();
    write_framed(
        &mut db,
        oid,
        PropertyIdentifier::RECIPIENT_LIST,
        framed.clone(),
    )
    .unwrap();
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::RECIPIENT_LIST),
        framed
    );

    // And the stored configuration decodes to exactly the written entries.
    let nc_obj = db.get(&oid).unwrap();
    let val = nc_obj
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    let bacnet_types::primitives::PropertyValue::ApplicationData(bytes) = val else {
        panic!("expected ApplicationData");
    };
    assert_eq!(
        bacnet_encoding::constructed::decode_destination_list(&bytes).unwrap(),
        destinations
    );
}

#[test]
fn recipient_list_framed_wire_round_trip_via_wpm() {
    // Same framed write through WritePropertyMultiple's decode path.
    use bacnet_services::common::BACnetPropertyValue;
    use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

    let mut db = ObjectDatabase::new();
    let nc = NotificationClass::new(1, "NC-1").unwrap();
    let oid = nc.object_identifier();
    db.add(Box::new(nc)).unwrap();

    let (framed, destinations) = framed_recipient_list();
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::RECIPIENT_LIST,
                property_array_index: None,
                value: framed.clone(),
                priority: None,
            }],
        }],
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    handle_write_property_multiple(&mut db, &buf).unwrap();
    assert_eq!(
        read_raw(&db, oid, PropertyIdentifier::RECIPIENT_LIST),
        framed
    );
    let _ = destinations;
}

#[test]
fn malformed_framed_event_parameters_write_rejected() {
    let mut db = ObjectDatabase::new();
    let ee = EventEnrollmentObject::new(1, "EE-1", 0).unwrap();
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // Reserved context tag [6] — rejected by decode, so INVALID_DATA_TYPE.
    let bad = vec![0x6E, 0x09, 0x01, 0x6F];
    let err = write_framed(&mut db, oid, PropertyIdentifier::EVENT_PARAMETERS, bad).unwrap_err();
    match err {
        Error::Protocol { class, .. } => {
            assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

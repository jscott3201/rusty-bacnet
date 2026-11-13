//! The clockless Recipient_List filter (#2303, decision 14): a `Responder`
//! with no clock refuses, on WriteProperty, WritePropertyMultiple and the
//! `Reporter`'s local writes, a Recipient_List holding a destination that
//! isn't open every day, all day; one with a clock stores it.

use super::*;
use crate::clock::{DeviceClock, Zone};
use crate::report::{ReportConfig, Reporter};
use crate::responder::{Inbound, Limits, Responder};
use crate::write::local::{LocalChange, LocalCommandSource};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::constructed::encode_destination_list;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_time::Instant;
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAddress, BACnetDestination, BACnetPropertyValue, BACnetRecipient,
};
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::primitives::Time;
use bytes::{Bytes, BytesMut};

fn class() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap()
}

fn destination(days: DaysOfWeek, to: Time) -> BACnetDestination {
    BACnetDestination {
        valid_days: days,
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: to,
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[10, 0, 0, 7, 0xBA, 0xC0]),
        }),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    }
}

const END: Time = Time {
    hour: 23,
    minute: 59,
    second: 59,
    hundredths: 99,
};

/// Open every day, all day.
fn always() -> BACnetDestination {
    destination(DaysOfWeek::all(), END)
}

/// Open every day until noon: restricted.
fn mornings() -> BACnetDestination {
    let noon = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    destination(DaysOfWeek::all(), noon)
}

/// Mondays only: restricted.
fn mondays() -> BACnetDestination {
    destination(DaysOfWeek::MONDAY, END)
}

fn encoded(list: &[BACnetDestination]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_destination_list(&mut buf, list).unwrap();
    buf.to_vec()
}

/// A Device and a Notification Class with no destination, and a
/// responder for them, with a clock when `clocked`.
fn device(clocked: bool) -> (Responder, ObjectDatabase) {
    let identity = DeviceIdentity::new(9, 0x105).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> =
        vec![Box::new(NotificationClass::new(1, "NC-1").unwrap())];
    let mut db =
        bacnet_objects::device_identity::build_database_with_extra(&identity, objects).unwrap();
    let responder = Responder::new(identity, Limits::default()).unwrap();
    let responder = if clocked {
        let zone = Zone::new(0, false).unwrap();
        let clock = DeviceClock::new(zone, Arc::new(|| Instant::ZERO));
        responder.with_clock(clock, &mut db)
    } else {
        responder
    };
    (responder, db)
}

fn request(service: ConfirmedServiceChoice, body: &[u8]) -> Bytes {
    let mut apdu = BytesMut::new();
    let pdu = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: Bytes::copy_from_slice(body),
    });
    encode_apdu(&mut apdu, &pdu).unwrap();
    apdu.freeze()
}

fn write_list(
    responder: &mut Responder,
    db: &mut ObjectDatabase,
    list: &[BACnetDestination],
) -> Apdu {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: class(),
        property_identifier: PropertyIdentifier::RECIPIENT_LIST,
        property_array_index: None,
        property_value: encoded(list),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    let apdu = request(ConfirmedServiceChoice::WRITE_PROPERTY, &body);
    let mut out = BytesMut::new();
    responder.write(db, None, Inbound::new(apdu, 1476), &mut out);
    decode_apdu(out.freeze()).unwrap()
}

fn stored(db: &ObjectDatabase) -> usize {
    let class = db.get(&class()).unwrap();
    let list = class
        .read_property(PropertyIdentifier::RECIPIENT_LIST, None)
        .unwrap();
    bacnet_objects::notification_class::decode_recipient_list(&list)
        .unwrap()
        .len()
}

fn refused(reply: &Apdu) -> bool {
    matches!(reply, Apdu::Error(error)
        if error.error_class == ErrorClass::PROPERTY
            && error.error_code == ErrorCode::VALUE_OUT_OF_RANGE)
}

#[test]
fn write_property_refuses_a_restricted_entry_without_a_clock() {
    let (mut responder, mut db) = device(false);
    for list in [vec![mornings()], vec![mondays()]] {
        let reply = write_list(&mut responder, &mut db, &list);
        assert!(refused(&reply), "{reply:?}");
        assert_eq!(stored(&db), 0, "nothing committed");
    }
}

#[test]
fn an_unrestricted_entry_is_stored_without_a_clock() {
    let (mut responder, mut db) = device(false);
    let reply = write_list(&mut responder, &mut db, &[always()]);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(stored(&db), 1);
}

#[test]
fn a_mixed_list_is_refused_whole() {
    let (mut responder, mut db) = device(false);
    assert!(matches!(
        write_list(&mut responder, &mut db, &[always()]),
        Apdu::SimpleAck(_)
    ));
    let reply = write_list(&mut responder, &mut db, &[always(), always(), mornings()]);
    assert!(refused(&reply), "{reply:?}");
    assert_eq!(stored(&db), 1, "the list as it was");
}

#[test]
fn a_responder_with_a_clock_stores_a_restricted_entry() {
    let (mut responder, mut db) = device(true);
    let reply = write_list(&mut responder, &mut db, &[mornings(), mondays()]);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(stored(&db), 2);
}

#[test]
fn write_property_multiple_keeps_what_it_wrote_before_the_refusal() {
    let (mut responder, mut db) = device(false);
    let mut description = BytesMut::new();
    encode_property_value(
        &mut description,
        &PropertyValue::CharacterString("first".into()),
    )
    .unwrap();
    let property = |property_identifier, value: Vec<u8>| BACnetPropertyValue {
        property_identifier,
        property_array_index: None,
        value,
        priority: None,
    };
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: class(),
            list_of_properties: vec![
                property(PropertyIdentifier::DESCRIPTION, description.to_vec()),
                property(PropertyIdentifier::RECIPIENT_LIST, encoded(&[mornings()])),
            ],
        }],
    }
    .encode(&mut body)
    .unwrap();
    let apdu = request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, &body);
    let mut out = BytesMut::new();
    responder.write(&mut db, None, Inbound::new(apdu, 1476), &mut out);
    let reply = decode_apdu(out.freeze()).unwrap();
    let Apdu::Error(error) = &reply else {
        panic!("{reply:?}")
    };
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
    );
    // The error data names the failed write and gives PROPERTY /
    // VALUE_OUT_OF_RANGE: [0] class, [1] code, then its reference.
    let class_and_code = [0x0E, 0x91, 0x02, 0x91, 0x25, 0x0F];
    assert!(
        error.error_data.starts_with(&class_and_code),
        "{:02X?}",
        error.error_data
    );
    let read = db
        .get(&class())
        .unwrap()
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(
        read,
        PropertyValue::CharacterString("first".into()),
        "it stays"
    );
    assert_eq!(stored(&db), 0);
}

#[test]
fn the_reporters_local_write_refuses_it_too() {
    let (responder, mut db) = device(false);
    let mut reporter = Reporter::new(responder, ReportConfig::default()).unwrap();
    let change = |list: &[BACnetDestination]| LocalChange::Property {
        property: PropertyIdentifier::RECIPIENT_LIST,
        array_index: None,
        value: PropertyValue::ApplicationData(encoded(list)),
        priority: None,
        source: LocalCommandSource::ServerDevice,
    };
    let applied = reporter.apply_local(&mut db, class(), change(&[mornings()]), Instant::ZERO);
    let error = applied.expect_err("restricted");
    assert_eq!(
        crate::reply::error_fields(&error),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
    );
    assert_eq!(stored(&db), 0);
    let applied = reporter.apply_local(&mut db, class(), change(&[always()]), Instant::ZERO);
    assert!(applied.is_ok(), "{applied:?}");
    assert_eq!(stored(&db), 1);
}

#[test]
fn an_add_list_element_host_checks_its_elements_the_same_way() {
    let elements = PropertyValue::ApplicationData(encoded(&[always(), mondays()]));
    let checked =
        check_clockless_recipients(class(), PropertyIdentifier::RECIPIENT_LIST, &elements);
    let error = checked.expect_err("Mondays only");
    assert_eq!(
        crate::reply::error_fields(&error),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
    );
    let fine = PropertyValue::ApplicationData(encoded(&[always()]));
    assert!(check_clockless_recipients(class(), PropertyIdentifier::RECIPIENT_LIST, &fine).is_ok());
    // Another property, or another object, isn't judged.
    let other = PropertyIdentifier::DESCRIPTION;
    assert!(check_clockless_recipients(class(), other, &elements).is_ok());
}

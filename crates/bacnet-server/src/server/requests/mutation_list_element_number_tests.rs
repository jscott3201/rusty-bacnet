//! AddListElement names the exact element an object refuses (#1048). Clause
//! 15.1.1.3.2 counts the First Failed Element Number among the request's List
//! of Elements, from 1. The object judges the edited list, its stored
//! elements followed by the new ones in request order, and names the position
//! it refused in that list; the handler maps it back to the request. Each
//! codec is checked with a refused second and a refused third element, some
//! behind an element already present, so the two counts differ.
//!
//! PROPERTY 2, RESOURCES 3; INVALID_DATA_TYPE 9, NO_SPACE_TO_ADD_LIST_ELEMENT
//! 19, NO_SPACE_TO_WRITE_PROPERTY 20, VALUE_OUT_OF_RANGE 37.

use super::mutation_list_wire_tests::{
    ADD, REMOVE, change_list_error, fill_msi_alarm_values, list_request, wire,
};
use super::mutation_tests::{Fixture, oid};
use super::*;
use bacnet_encoding::constructed::{
    decode_destination_list, encode_destination, encode_destination_list,
};
use bacnet_objects::elevator::EscalatorObject;
use bacnet_objects::notification_class::{MAX_RECIPIENT_LIST_DESTINATIONS, NotificationClass};
use bacnet_objects::schedule::CalendarObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetDestination, BACnetRecipient};
use bacnet_types::error::ErrorDetail;
use bacnet_types::primitives::{Date, Time};
use std::borrow::Cow;

const FAULT_SIGNALS: PropertyIdentifier = PropertyIdentifier::FAULT_SIGNALS;

async fn add_wire(
    fixture: &Fixture,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    elements: &[u8],
) -> Vec<u8> {
    wire(fixture, ADD, list_request(object, property, None, elements)).await
}

#[tokio::test]
async fn add_list_element_names_the_value_an_object_refuses_as_out_of_range() {
    // Escalator Fault_Signals: faults 9 to 1023 are reserved, so 20 and 21
    // are out of range; 1 is stored.
    let fixture = Fixture::new(None);
    let escalator = oid(ObjectType::ESCALATOR, 1);
    {
        let mut db = fixture.db.write().await;
        db.add(Box::new(EscalatorObject::new(1, "ESC-1").unwrap()))
            .unwrap();
        db.get_mut(&escalator)
            .unwrap()
            .write_property(
                FAULT_SIGNALS,
                None,
                PropertyValue::List(vec![PropertyValue::Enumerated(1)]),
                None,
            )
            .unwrap();
    }
    let stored = fixture.read(escalator, FAULT_SIGNALS).await;
    for (what, elements, element) in [
        (
            "a new fault, then a reserved one",
            &[0x91, 2, 0x91, 20][..],
            2,
        ),
        (
            "the stored fault, a new one, then a reserved one",
            &[0x91, 1, 0x91, 3, 0x91, 21],
            3,
        ),
    ] {
        assert_eq!(
            add_wire(&fixture, escalator, FAULT_SIGNALS, elements).await,
            change_list_error(ADD, 2, 37, element),
            "{what}"
        );
        assert_eq!(
            fixture.read(escalator, FAULT_SIGNALS).await,
            stored,
            "{what}"
        );
    }
}

#[tokio::test]
async fn add_list_element_names_the_value_that_does_not_fit() {
    // Multi-state Input Alarm_Values holds at most 1024 values; 1023 are
    // stored, so the second new value is the one with no room.
    let fixture = Fixture::new(None);
    let msi = oid(ObjectType::MULTI_STATE_INPUT, 1);
    let alarm_values = PropertyIdentifier::ALARM_VALUES;
    fill_msi_alarm_values(&fixture, 100..1123).await;
    let stored = fixture.read(msi, alarm_values).await;
    for (what, elements, element) in [
        ("two new values", &[0x21, 7, 0x21, 8][..], 2),
        (
            "a stored value, then two new ones",
            &[0x21, 100, 0x21, 7, 0x21, 8],
            3,
        ),
    ] {
        assert_eq!(
            add_wire(&fixture, msi, alarm_values, elements).await,
            change_list_error(ADD, 3, 19, element),
            "{what}"
        );
        assert_eq!(fixture.read(msi, alarm_values).await, stored, "{what}");
    }
}

/// date `[0]` holding four octets: an unspecified weekday on the first of
/// the month, the month and year counting up with `index`.
fn first_of_month(index: u16) -> [u8; 5] {
    [0x0C, (index / 12) as u8, (index % 12) as u8 + 1, 1, 0xFF]
}

#[tokio::test]
async fn add_list_element_names_the_calendar_entry_refused() {
    // weekNDay `[2]`: every Monday, every Tuesday, and weekday 8, out of range.
    const MONDAYS: [u8; 4] = [0x2B, 0xFF, 0xFF, 1];
    const TUESDAYS: [u8; 4] = [0x2B, 0xFF, 0xFF, 2];
    const WEEKDAY_8: [u8; 4] = [0x2B, 0xFF, 0xFF, 8];
    let fixture = Fixture::new(None);
    let full = oid(ObjectType::CALENDAR, 1);
    let short = oid(ObjectType::CALENDAR, 2);
    {
        let mut db = fixture.db.write().await;
        // Date_List holds at most 1024 entries; 1023 distinct ones are stored.
        let mut calendar = CalendarObject::new(1, "CAL-1").unwrap();
        for index in 0..1023 {
            let [_, year, month, day, day_of_week] = first_of_month(index);
            calendar
                .add_date_entry(BACnetCalendarEntry::Date(Date {
                    year,
                    month,
                    day,
                    day_of_week,
                }))
                .unwrap();
        }
        db.add(Box::new(calendar)).unwrap();
        let mut calendar = CalendarObject::new(2, "CAL-2").unwrap();
        let [_, year, month, day, day_of_week] = first_of_month(0);
        calendar
            .add_date_entry(BACnetCalendarEntry::Date(Date {
                year,
                month,
                day,
                day_of_week,
            }))
            .unwrap();
        db.add(Box::new(calendar)).unwrap();
    }
    let date_list = PropertyIdentifier::DATE_LIST;
    let stored = first_of_month(0);
    for (what, calendar, elements, class, code, element) in [
        (
            "two new entries, no room for the second",
            full,
            [&MONDAYS[..], &TUESDAYS].concat(),
            3,
            19,
            2,
        ),
        (
            "a stored entry, then two new ones, no room for the third",
            full,
            [&stored[..], &MONDAYS, &TUESDAYS].concat(),
            3,
            19,
            3,
        ),
        (
            "a new entry, then one out of range",
            short,
            [&MONDAYS[..], &WEEKDAY_8].concat(),
            2,
            37,
            2,
        ),
        (
            "a stored entry, a new one, then one out of range",
            short,
            [&stored[..], &MONDAYS, &WEEKDAY_8].concat(),
            2,
            37,
            3,
        ),
    ] {
        let before = fixture.read(calendar, date_list).await;
        assert_eq!(
            add_wire(&fixture, calendar, date_list, &elements).await,
            change_list_error(ADD, class, code, element),
            "{what}"
        );
        assert_eq!(fixture.read(calendar, date_list).await, before, "{what}");
    }
}

fn destination(process_identifier: u32) -> BACnetDestination {
    let time = |hour, minute| Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    };
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: time(0, 0),
        to_time: time(23, 59),
        recipient: BACnetRecipient::Device(oid(ObjectType::DEVICE, 9)),
        process_identifier,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    }
}

fn elements(process_identifiers: &[u32]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for &process_identifier in process_identifiers {
        encode_destination(&mut buf, &destination(process_identifier)).unwrap();
    }
    buf.to_vec()
}

/// A Notification Class-typed custom object whose Recipient_List, a framed
/// BACnetLIST of BACnetDestination, holds at most three destinations. Its
/// writer names the destination it refuses: a process identifier above 1000,
/// or a fourth destination. Process identifier 666 it refuses without naming.
struct Recipients {
    oid: ObjectIdentifier,
    name: &'static str,
    list: Vec<BACnetDestination>,
}

fn protocol(class: ErrorClass, code: ErrorCode, element: Option<usize>) -> Error {
    Error::with_detail(
        class,
        code,
        element.map(|index| ErrorDetail::FirstFailedElementNumber(index as u32 + 1)),
    )
}

impl BACnetObject for Recipients {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        self.name
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property != PropertyIdentifier::RECIPIENT_LIST {
            return Err(protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
                None,
            ));
        }
        let mut buf = BytesMut::new();
        encode_destination_list(&mut buf, &self.list).unwrap();
        Ok(PropertyValue::ApplicationData(buf.to_vec()))
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        let invalid = || protocol(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, None);
        let PropertyValue::ApplicationData(bytes) = value else {
            return Err(invalid());
        };
        let list = decode_destination_list(&bytes).map_err(|_| invalid())?;
        for (index, destination) in list.iter().enumerate() {
            if index == 3 {
                return Err(protocol(
                    ErrorClass::RESOURCES,
                    ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
                    Some(index),
                ));
            }
            match destination.process_identifier {
                666 => {
                    return Err(protocol(
                        ErrorClass::PROPERTY,
                        ErrorCode::VALUE_OUT_OF_RANGE,
                        None,
                    ));
                }
                1001.. => {
                    return Err(protocol(
                        ErrorClass::PROPERTY,
                        ErrorCode::VALUE_OUT_OF_RANGE,
                        Some(index),
                    ));
                }
                _ => {}
            }
        }
        self.list = list;
        Ok(())
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::RECIPIENT_LIST,
        ])
    }
}

#[tokio::test]
async fn add_list_element_names_the_destination_an_object_refuses() {
    let fixture = Fixture::new(None);
    let recipients = oid(ObjectType::NOTIFICATION_CLASS, 7);
    // Holds a destination the writer now refuses, which no request adds.
    let legacy = oid(ObjectType::NOTIFICATION_CLASS, 8);
    {
        let mut db = fixture.db.write().await;
        db.add(Box::new(Recipients {
            oid: recipients,
            name: "Recipients",
            list: vec![destination(1)],
        }))
        .unwrap();
        db.add(Box::new(Recipients {
            oid: legacy,
            name: "Legacy recipients",
            list: vec![destination(1001), destination(10)],
        }))
        .unwrap();
    }
    let recipient_list = PropertyIdentifier::RECIPIENT_LIST;
    for (what, object, service, elements, class, code, element) in [
        (
            "a new destination, then one out of range",
            recipients,
            ADD,
            elements(&[2, 1001]),
            2,
            37,
            2,
        ),
        (
            "the stored destination, a new one, then one out of range",
            recipients,
            ADD,
            elements(&[1, 3, 1002]),
            2,
            37,
            3,
        ),
        (
            "three new destinations, no room for the third",
            recipients,
            ADD,
            elements(&[4, 5, 6]),
            3,
            19,
            3,
        ),
        // A refusal naming no element keeps the estimate: the first new one.
        (
            "an unnamed refusal of the third",
            recipients,
            ADD,
            elements(&[1, 8, 666]),
            2,
            37,
            2,
        ),
        // The object refuses an element the list already held: no element
        // of the request is at fault.
        (
            "a stored destination refused",
            legacy,
            ADD,
            elements(&[9]),
            2,
            37,
            0,
        ),
        (
            "what a removal leaves refused",
            legacy,
            REMOVE,
            elements(&[10]),
            2,
            37,
            0,
        ),
    ] {
        let before = fixture.read(object, recipient_list).await;
        assert_eq!(
            wire(
                &fixture,
                service,
                list_request(object, recipient_list, None, &elements)
            )
            .await,
            change_list_error(service, class, code, element),
            "{what}"
        );
        assert_eq!(fixture.read(object, recipient_list).await, before, "{what}");
    }
}

/// The Recipient_List cap (#1098), as the test's process identifiers count.
const CAP: u32 = MAX_RECIPIENT_LIST_DESTINATIONS as u32;

/// A Notification Class holding `count` destinations, process identifiers 1
/// to `count`.
fn notification_class(instance: u32, count: u32) -> NotificationClass {
    let mut class = NotificationClass::new(instance, format!("NC-{instance}")).unwrap();
    for process_identifier in 1..=count {
        class
            .add_destination(destination(process_identifier))
            .unwrap();
    }
    class
}

#[tokio::test]
async fn add_list_element_names_the_destination_past_the_recipient_list_cap() {
    // One class has room for one more destination, the other for two.
    let fixture = Fixture::new(None);
    let one_left = oid(ObjectType::NOTIFICATION_CLASS, 1);
    let two_left = oid(ObjectType::NOTIFICATION_CLASS, 2);
    {
        let mut db = fixture.db.write().await;
        db.add(Box::new(notification_class(1, CAP - 1))).unwrap();
        db.add(Box::new(notification_class(2, CAP - 2))).unwrap();
    }
    let recipient_list = PropertyIdentifier::RECIPIENT_LIST;
    for (what, object, elements, element) in [
        (
            "two new destinations, no room for the second",
            one_left,
            elements(&[100, 101]),
            2,
        ),
        (
            "a stored destination, then two new ones, no room for the third",
            one_left,
            elements(&[1, 100, 101]),
            3,
        ),
        (
            "three new destinations, no room for the third",
            two_left,
            elements(&[100, 101, 102]),
            3,
        ),
    ] {
        let before = fixture.read(object, recipient_list).await;
        assert_eq!(
            add_wire(&fixture, object, recipient_list, &elements).await,
            change_list_error(ADD, 3, 19, element),
            "{what}"
        );
        assert_eq!(fixture.read(object, recipient_list).await, before, "{what}");
    }
}

#[tokio::test]
async fn add_list_element_fills_the_recipient_list_to_the_cap() {
    let fixture = Fixture::new(None);
    let class = oid(ObjectType::NOTIFICATION_CLASS, 1);
    fixture
        .db
        .write()
        .await
        .add(Box::new(notification_class(1, CAP - 2)))
        .unwrap();
    let recipient_list = PropertyIdentifier::RECIPIENT_LIST;
    let full: Vec<u32> = (1..=CAP - 2).chain([100, 101]).collect();
    let full = PropertyValue::ApplicationData(elements(&full));
    assert_eq!(
        add_wire(&fixture, class, recipient_list, &elements(&[100, 101])).await,
        vec![0x20, 5, ADD.to_raw()]
    );
    assert_eq!(fixture.read(class, recipient_list).await, full);
    // A full list still takes a destination it already holds, which adds
    // nothing, and refuses the first new one.
    assert_eq!(
        add_wire(&fixture, class, recipient_list, &elements(&[1])).await,
        vec![0x20, 5, ADD.to_raw()]
    );
    assert_eq!(
        add_wire(&fixture, class, recipient_list, &elements(&[1, 102])).await,
        change_list_error(ADD, 3, 19, 2)
    );
    assert_eq!(fixture.read(class, recipient_list).await, full);
}

#[tokio::test]
async fn write_property_of_a_recipient_list_past_the_cap_is_no_space() {
    let fixture = Fixture::new(None);
    let class = oid(ObjectType::NOTIFICATION_CLASS, 1);
    fixture
        .db
        .write()
        .await
        .add(Box::new(notification_class(1, 1)))
        .unwrap();
    let recipient_list = PropertyIdentifier::RECIPIENT_LIST;
    let write_property = |count: u32| {
        let mut request = BytesMut::new();
        WritePropertyRequest {
            object_identifier: class,
            property_identifier: recipient_list,
            property_array_index: None,
            property_value: elements(&(1..=count).collect::<Vec<_>>()),
            priority: None,
        }
        .encode(&mut request)
        .unwrap();
        request.freeze()
    };
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let before = fixture.read(class, recipient_list).await;
    // The plain class and code: a WriteProperty error names no element.
    assert_eq!(
        wire(&fixture, service, write_property(CAP + 1)).await,
        vec![0x50, 5, 15, 0x91, 3, 0x91, 20]
    );
    assert_eq!(fixture.read(class, recipient_list).await, before);
    assert_eq!(
        wire(&fixture, service, write_property(CAP)).await,
        vec![0x20, 5, 15]
    );
    assert_eq!(
        fixture.read(class, recipient_list).await,
        PropertyValue::ApplicationData(elements(&(1..=CAP).collect::<Vec<_>>()))
    );
}

#[tokio::test]
async fn write_property_keeps_the_plain_error_when_an_object_names_an_element() {
    // The Multi-state Input names its out-of-range second value; a
    // WriteProperty error still carries only the class and code.
    let fixture = Fixture::new(None);
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::MULTI_STATE_INPUT, 1),
        property_identifier: PropertyIdentifier::ALARM_VALUES,
        property_array_index: None,
        property_value: vec![0x21, 1, 0x25, 0x05, 0x01, 0, 0, 0, 0],
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request.freeze()
        )
        .await,
        vec![0x50, 5, 15, 0x91, 2, 0x91, 37]
    );
}

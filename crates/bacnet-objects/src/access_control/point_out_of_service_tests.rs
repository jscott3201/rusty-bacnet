//! The access events an Access Point records on its Out_Of_Service edges
//! (Clauses 12.31.8 and 12.31.27 to 12.31.30, #1248, #1284).

use std::sync::Arc;

use bacnet_types::enums::{ErrorCode, PropertyIdentifier as P};

use super::credential_data_input_out_of_service_tests::{
    assert_property_error, sequence, stamp, stamped, FixedClock,
};
use super::*;

/// Access Credential 3, the credential behind the GRANTED event.
pub(super) fn credential() -> BACnetDeviceObjectReference {
    ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 3)
        .unwrap()
        .into()
}

/// Access_Event_Credential naming Access Credential 3: the object under [1].
pub(super) const CREDENTIAL_3: [u8; 5] = [0x1C, 0x08, 0x00, 0x00, 0x03];

/// The no-credential reference: Access Credential 4194303 under [1].
pub(super) const NO_CREDENTIAL: [u8; 5] = [0x1C, 0x08, 0x3F, 0xFF, 0xFF];

/// A point whose last event was GRANTED to Access Credential 3 in
/// transaction 7 at 09:30, on a Device clock reading 11:30.
fn point() -> AccessPointObject {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    granted_at(&mut point, 7);
    point.bind_clock_internal(Some(Arc::new(FixedClock(11))));
    point
}

/// Record GRANTED to Access Credential 3 in transaction `tag` at 09:30.
fn granted_at(point: &mut AccessPointObject, tag: u64) {
    point
        .set_access_event(AccessEventReport {
            time: Some(stamp(9)),
            credential: Some(credential()),
            ..AccessEventReport::new(AccessEvent::GRANTED, tag)
        })
        .unwrap();
}

fn write_out_of_service(point: &mut AccessPointObject, value: PropertyValue) -> Result<(), Error> {
    point.write_property(P::OUT_OF_SERVICE, None, value, None)
}

/// Access_Event, Access_Event_Tag, Access_Event_Time, Access_Event_Credential
/// and Status_Flags as served.
fn served(point: &AccessPointObject) -> [PropertyValue; 5] {
    [
        P::ACCESS_EVENT,
        P::ACCESS_EVENT_TAG,
        P::ACCESS_EVENT_TIME,
        P::ACCESS_EVENT_CREDENTIAL,
        P::STATUS_FLAGS,
    ]
    .map(|property| point.read_property(property, None).unwrap())
}

/// An event an Out_Of_Service edge records: no credential belongs to it.
fn event(
    event: AccessEvent,
    tag: u64,
    time: PropertyValue,
    out_of_service: bool,
) -> [PropertyValue; 5] {
    let mut flags = StatusFlags::empty();
    flags.set(StatusFlags::OUT_OF_SERVICE, out_of_service);
    [
        PropertyValue::Enumerated(event.to_raw()),
        PropertyValue::Unsigned(tag),
        time,
        PropertyValue::ApplicationData(NO_CREDENTIAL.to_vec()),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![flags.bits() << 4],
        },
    ]
}

/// The GRANTED event the point starts from, with its credential.
fn granted(out_of_service: bool) -> [PropertyValue; 5] {
    let mut granted = event(AccessEvent::GRANTED, 7, stamped(9), out_of_service);
    granted[3] = PropertyValue::ApplicationData(CREDENTIAL_3.to_vec());
    granted
}

#[test]
fn access_point_entering_out_of_service_records_out_of_service() {
    let mut point = point();
    assert_eq!(served(&point), granted(false));
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    // A new transaction, stamped from the Device clock.
    assert_eq!(
        served(&point),
        event(AccessEvent::OUT_OF_SERVICE, 8, stamped(11), true)
    );
}

#[test]
fn access_point_return_to_service_records_out_of_service_relinquished() {
    let mut point = point();
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    point.bind_clock_internal(Some(Arc::new(FixedClock(12))));
    write_out_of_service(&mut point, PropertyValue::Boolean(false)).unwrap();
    assert_eq!(
        served(&point),
        event(
            AccessEvent::OUT_OF_SERVICE_RELINQUISHED,
            9,
            stamped(12),
            false
        )
    );

    // A second period out of service records a pair of its own.
    point.bind_clock_internal(Some(Arc::new(FixedClock(13))));
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(
        served(&point),
        event(AccessEvent::OUT_OF_SERVICE, 10, stamped(13), true)
    );
}

#[test]
fn access_point_writes_that_keep_out_of_service_record_nothing() {
    let mut point = point();
    // FALSE while in service, NULL and a refused datatype are no edges.
    write_out_of_service(&mut point, PropertyValue::Boolean(false)).unwrap();
    write_out_of_service(&mut point, PropertyValue::Null).unwrap();
    assert_property_error(
        write_out_of_service(&mut point, PropertyValue::Unsigned(1)),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(served(&point), granted(false));

    // TRUE again while out of service records nothing more either.
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    let entered = served(&point);
    point.bind_clock_internal(Some(Arc::new(FixedClock(12))));
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    write_out_of_service(&mut point, PropertyValue::Null).unwrap();
    assert_eq!(served(&point), entered);
}

#[test]
fn access_point_out_of_service_event_without_a_clock_counts_sequence_numbers() {
    let mut point = point();
    point.bind_clock_internal(None);
    // The time served is a date and time, so the count starts at 1.
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(
        served(&point),
        event(AccessEvent::OUT_OF_SERVICE, 8, sequence(1), true)
    );
    write_out_of_service(&mut point, PropertyValue::Boolean(false)).unwrap();
    assert_eq!(
        served(&point),
        event(
            AccessEvent::OUT_OF_SERVICE_RELINQUISHED,
            9,
            sequence(2),
            false
        )
    );
}

#[test]
fn access_point_sequence_number_wraps_back_to_one() {
    let mut point = point();
    point.bind_clock_internal(None);
    // The count follows the sequence number served, whatever the tag, and
    // wraps from 65535 to 1, never the 0 of no update yet.
    point
        .set_access_event(AccessEventReport {
            time: Some(BACnetTimeStamp::SequenceNumber(65_534)),
            ..AccessEventReport::new(AccessEvent::GRANTED, 3)
        })
        .unwrap();
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(
        served(&point),
        event(AccessEvent::OUT_OF_SERVICE, 4, sequence(65_535), true)
    );
    write_out_of_service(&mut point, PropertyValue::Boolean(false)).unwrap();
    assert_eq!(
        served(&point),
        event(
            AccessEvent::OUT_OF_SERVICE_RELINQUISHED,
            5,
            sequence(1),
            false
        )
    );
}

#[test]
fn access_point_events_of_one_transaction_each_move_the_time() {
    // Two events with one tag (Clause 12.31.27.1), stamped by the point,
    // never share a time: the second would send no COV report.
    let read_then_grant = |point: &mut AccessPointObject| {
        let mut times = Vec::new();
        for event in [
            AccessEvent::AUTHENTICATION_FACTOR_READ,
            AccessEvent::GRANTED,
        ] {
            point
                .set_access_event(AccessEventReport::new(event, 20))
                .unwrap();
            times.push(point.read_property(P::ACCESS_EVENT_TIME, None).unwrap());
        }
        times
    };
    let mut clockless = point();
    clockless.bind_clock_internal(None);
    assert_eq!(read_then_grant(&mut clockless), [sequence(1), sequence(2)]);

    // With a clock, the second event in the same hundredth is stamped one
    // hundredth later.
    let mut clocked = point();
    let later = |hundredths: u8| {
        PropertyValue::ApplicationData(vec![
            0x2E, 0xA4, 126, 10, 2, 5, 0xB4, 11, 30, 0, hundredths, 0x2F,
        ])
    };
    assert_eq!(read_then_grant(&mut clocked), [stamped(11), later(1)]);
    // A clock set back is no reason to go back either.
    clocked.bind_clock_internal(Some(Arc::new(FixedClock(10))));
    clocked
        .set_access_event(AccessEventReport::new(AccessEvent::GRANTED, 21))
        .unwrap();
    assert_eq!(
        clocked.read_property(P::ACCESS_EVENT_TIME, None).unwrap(),
        later(2)
    );
}

#[test]
fn access_point_stamp_after_the_last_hundredth_of_a_day_carries_into_the_next() {
    use bacnet_types::primitives::{Date, Time};
    // The clock reads 11:30 on 2026-10-02, before each time served below.
    let mut point = point();
    let at = |year: u8, month: u8, day: u8, day_of_week: u8, hour: u8| BACnetTimeStamp::DateTime {
        date: Date {
            year,
            month,
            day,
            day_of_week,
        },
        time: Time {
            hour,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
    };
    let midnight = |year: u8, month: u8, day: u8, day_of_week: u8| {
        PropertyValue::ApplicationData(vec![
            0x2E,
            0xA4,
            year,
            month,
            day,
            day_of_week,
            0xB4,
            0,
            0,
            0,
            0,
            0x2F,
        ])
    };
    for (served, expected) in [
        // Within the day, one hundredth carries through every field.
        (at(126, 10, 2, 5, 22), {
            PropertyValue::ApplicationData(vec![0x2E, 0xA4, 126, 10, 2, 5, 0xB4, 23, 0, 0, 0, 0x2F])
        }),
        // The end of a day, a month and a year (2026-12-31 is a Thursday).
        (at(126, 10, 2, 5, 23), midnight(126, 10, 3, 6)),
        (at(126, 10, 31, 6, 23), midnight(126, 11, 1, 7)),
        (at(126, 12, 31, 4, 23), midnight(127, 1, 1, 5)),
    ] {
        point
            .set_access_event(AccessEventReport {
                time: Some(served),
                ..AccessEventReport::new(AccessEvent::GRANTED, 1)
            })
            .unwrap();
        point
            .set_access_event(AccessEventReport::new(AccessEvent::GRANTED, 1))
            .unwrap();
        assert_eq!(
            point.read_property(P::ACCESS_EVENT_TIME, None).unwrap(),
            expected
        );
    }
}

#[test]
fn access_point_out_of_service_event_tag_wraps_at_the_top_of_its_range() {
    let mut point = point();
    granted_at(&mut point, u64::MAX);
    write_out_of_service(&mut point, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(
        served(&point),
        event(AccessEvent::OUT_OF_SERVICE, 0, stamped(11), true)
    );
}

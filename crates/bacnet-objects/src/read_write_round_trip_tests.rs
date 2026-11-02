//! Catalogue-wide check that every built-in object takes back, on the
//! object API, the value a read of a writable row just returned (#2272).
//!
//! A read and a write of the same property must agree on the value's shape:
//! a list read as one encoded element per item is taken back as that list,
//! and a constructed value read as its octets is taken back as them. The
//! check writes each row the value it reads, through `write_property_from`
//! (a commandable value takes writes only there), and expects success and
//! the same value read afterwards. A registered Network Port's mode rows are
//! checked in `network_port::round_trip_tests`, since a catalogue port is
//! NORMAL and unregistered.
//!
//! Each object first takes the writes in `read_write_round_trip_seeds::prelude`, the state a
//! row needs before it takes writes at all, and then non-empty values for
//! the lists it starts with empty (`seeds`). The rows in [`REFUSED`]
//! still refuse their value, for a reason that isn't its shape, so each is
//! pinned to the error it gives.

use crate::property_metadata::is_writable_property;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

use crate::catalogue::representatives;
use crate::command_source::test_origin;
use crate::read_write_round_trip_seeds::{prelude, seeds};

/// Writable rows that refuse their own value at a catalogue object's
/// initial state, and why. None of these is a shape mismatch.
const REFUSED: &[(ObjectType, P, ErrorClass, ErrorCode)] = &[
    // Clearing a log stamps a log-status record, which needs the device's
    // clock; a catalogue object has none. The same missing clock keeps
    // Trend Log Multiple's references unseeded below.
    (
        ObjectType::TREND_LOG,
        P::RECORD_COUNT,
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    ),
    (
        ObjectType::EVENT_LOG,
        P::RECORD_COUNT,
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    ),
    (
        ObjectType::TREND_LOG_MULTIPLE,
        P::RECORD_COUNT,
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM,
    ),
    // Buffer_Size changes only while logging is off, and turning it off
    // stamps a log-status record, which needs the clock too.
    (
        ObjectType::AUDIT_LOG,
        P::BUFFER_SIZE,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    ),
    // A new object's command reads as NONE, which only reports that nothing
    // was written: no client can write it.
    (
        ObjectType::LIGHTING_OUTPUT,
        P::LIGHTING_COMMAND,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    ),
    (
        ObjectType::COLOR,
        P::COLOR_COMMAND,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    ),
    (
        ObjectType::COLOR_TEMPERATURE,
        P::COLOR_COMMAND,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    ),
];

fn refusal(object_type: ObjectType, property: P) -> Option<(ErrorClass, ErrorCode)> {
    REFUSED
        .iter()
        .find(|(t, p, ..)| *t == object_type && *p == property)
        .map(|(_, _, class, code)| (*class, *code))
}

#[test]
fn every_writable_row_takes_back_the_value_it_reads() {
    let mut failures = Vec::new();
    let mut lists = 0;
    let mut refused = 0;
    let mut checked = 0;
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        // Out of service, an input's Present_Value and Reliability are
        // writable too.
        if is_writable_property(&*object, P::OUT_OF_SERVICE) {
            object
                .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
                .unwrap();
        }
        for (property, value) in prelude(object_type) {
            object
                .write_property(property, None, value, None)
                .unwrap_or_else(|error| panic!("{object_type} {property} prelude: {error:?}"));
        }
        // Lists a new object holds empty get entries, so the round trip
        // sees their elements.
        for (property, value) in seeds(object_type) {
            object
                .write_property_from(property, None, value, None, &test_origin())
                .unwrap_or_else(|error| panic!("{object_type} {property} seed: {error:?}"));
        }
        let rows = object.property_metadata().into_owned();
        for row in rows {
            let property = row.property_identifier;
            if !is_writable_property(&*object, property) {
                continue;
            }
            let value = object.read_property(property, None).unwrap();
            if matches!(&value, PropertyValue::List(items) if !items.is_empty()) {
                lists += 1;
            }
            let result =
                object.write_property_from(property, None, value.clone(), None, &test_origin());
            match (result, refusal(object_type, property)) {
                (Ok(()), None) => {
                    let again = object.read_property(property, None).unwrap();
                    if again != value {
                        failures.push(format!(
                            "{object_type} {property}: read {value:?}, then {again:?}"
                        ));
                    }
                }
                (Err(Error::Remote(RemoteError::Error { class, code })), Some(expected))
                    if (class, code) == expected =>
                {
                    refused += 1;
                }
                (result, expected) => failures.push(format!(
                    "{object_type} {property} [{value:?}]: {result:?}, expected {expected:?}"
                )),
            }
            checked += 1;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(refused, REFUSED.len(), "a pinned refusal no longer happens");
    assert!(checked > 400, "only {checked} rows checked");
    assert!(lists > 40, "only {lists} non-empty lists checked");
}

/// Split `octets` into one element per encoded entry, as a caller holding
/// the entries one by one would pass them.
fn elements(octets: &[u8], end_of: fn(&[u8], usize) -> usize) -> PropertyValue {
    let mut items = Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        let end = end_of(octets, offset);
        items.push(PropertyValue::ApplicationData(octets[offset..end].to_vec()));
        offset = end;
    }
    assert!(items.len() > 1, "a list of several entries");
    PropertyValue::List(items)
}

/// Recipient_List and Subscribed_Recipients read as their framed octets, and
/// take the same entries as a list of encoded elements too, the shape the
/// other constructed lists read as (#2272).
#[test]
fn recipient_lists_take_their_entries_one_element_at_a_time() {
    use bacnet_encoding::constructed::{
        decode_destination, decode_event_notification_subscription,
    };
    let destination: fn(&[u8], usize) -> usize = |data, at| decode_destination(data, at).unwrap().1;
    let subscription: fn(&[u8], usize) -> usize =
        |data, at| decode_event_notification_subscription(data, at).unwrap().1;
    let mut checked = 0;
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        for (property, seed) in seeds(object_type) {
            let end_of = match property {
                P::RECIPIENT_LIST => destination,
                P::SUBSCRIBED_RECIPIENTS => subscription,
                _ => continue,
            };
            let PropertyValue::ApplicationData(octets) = &seed else {
                panic!("{object_type} {property}: framed seed");
            };
            let list = elements(octets, end_of);
            object
                .write_property(property, None, list, None)
                .unwrap_or_else(|error| panic!("{object_type} {property}: {error:?}"));
            assert_eq!(object.read_property(property, None).unwrap(), seed);
            checked += 1;
        }
    }
    assert_eq!(checked, 3, "Notification Class and Forwarder rows");
}

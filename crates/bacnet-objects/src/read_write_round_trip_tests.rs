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
//! The rows in [`REFUSED`] refuse the value for a reason that isn't its
//! shape, so each is pinned to the error it gives.

use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

use crate::catalogue::representatives;
use crate::command_source::test_origin;

/// Writable rows that refuse their own value on a catalogue object, and why.
const REFUSED: &[(ObjectType, P, ErrorClass, ErrorCode)] = &[
    // Clearing a log stamps a log-status record, which needs the device's
    // clock; a catalogue object has none.
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
    // Event_Algorithm_Inhibit is written only while event detection runs.
    (
        ObjectType::BINARY_OUTPUT,
        P::EVENT_ALGORITHM_INHIBIT,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    ),
    (
        ObjectType::MULTI_STATE_OUTPUT,
        P::EVENT_ALGORITHM_INHIBIT,
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    ),
    // Buffer_Size changes only while logging is off.
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
        if object.is_writable_property(P::OUT_OF_SERVICE) {
            object
                .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
                .unwrap();
        }
        let rows = object.property_metadata().into_owned();
        for row in rows {
            let property = row.property_identifier;
            if !object.is_writable_property(property) {
                continue;
            }
            let value = object.read_property(property, None).unwrap();
            if matches!(value, PropertyValue::List(_)) {
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
    assert!(lists > 50, "only {lists} list values checked");
}

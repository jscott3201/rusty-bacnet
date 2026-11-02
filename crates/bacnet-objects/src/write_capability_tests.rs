//! Catalogue-wide check that each row's write capability matches what the
//! object's write arm does (#2175).
//!
//! Every row of one object of each catalogue type is written the value it
//! reads (NULL where it doesn't read whole), through `write_property_from`
//! and, for a row that refuses, `write_property` too:
//! - a row with no write route of its own (`ReadOnly`, `Through`) is refused
//!   with PROPERTY / WRITE_ACCESS_DENIED on both routes;
//! - an `Always` row isn't refused for access: the object may still refuse
//!   the value, which `read_write_round_trip_tests` pins;
//! - a `WhenOutOfService` row is refused for access while the object is in
//!   service, and isn't once Out_Of_Service is TRUE;
//! - a `WhenCommandOwner` row isn't refused for its command's writer, and
//!   is refused for anyone else.
//!
//! Rows writable by capability whose object refuses access in the
//! catalogue's starting state are listed in [`STATE_REFUSED`], each with
//! its reason. A row that leaves the list, or one that joins it unlisted,
//! fails the test.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::catalogue::representatives;
use crate::command_source::{CommandOrigin, test_origin};
use crate::property_metadata::PropertyWriteCapability as W;
use crate::traits::BACnetObject;

/// Rows writable by capability that the object refuses for access in the
/// catalogue's starting state, and why.
const STATE_REFUSED: &[(ObjectType, P, &str)] = &[
    // The inhibit takes writes only while event detection is on, and an
    // output starts with Event_Detection_Enable FALSE.
    (
        ObjectType::BINARY_OUTPUT,
        P::EVENT_ALGORITHM_INHIBIT,
        "Event_Detection_Enable starts FALSE on an output",
    ),
    (
        ObjectType::MULTI_STATE_OUTPUT,
        P::EVENT_ALGORITHM_INHIBIT,
        "Event_Detection_Enable starts FALSE on an output",
    ),
    (
        ObjectType::AUDIT_LOG,
        P::BUFFER_SIZE,
        "Buffer_Size changes only while Log_Enable is FALSE, and a new Audit Log is enabled",
    ),
];

fn state_refused(object_type: ObjectType, property: P) -> bool {
    STATE_REFUSED
        .iter()
        .any(|(t, p, _)| *t == object_type && *p == property)
}

fn access_denied(result: &Result<(), Error>) -> bool {
    matches!(
        result,
        Err(Error::Remote(RemoteError::Error { class, code }))
            if *class == ErrorClass::PROPERTY && *code == ErrorCode::WRITE_ACCESS_DENIED
    )
}

/// A local writer other than [`test_origin`]'s, owned by another device.
fn other_origin() -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, 2).unwrap(),
        initiating_object: None,
    }
}

/// What the row reads whole, or NULL for a row that doesn't read (a log's
/// ReadRange-only Log_Buffer, the Staging object's unset Present_Stage).
fn current(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object
        .read_property(property, None)
        .unwrap_or(PropertyValue::Null)
}

/// Write `property` its current value from `origin`.
fn write_back(
    object: &mut dyn BACnetObject,
    property: P,
    origin: &CommandOrigin,
) -> Result<(), Error> {
    let value = current(object, property);
    object.write_property_from(property, None, value, None, origin)
}

#[derive(Default)]
struct Tally {
    failures: Vec<String>,
    refused: usize,
    out_of_service: usize,
    owned: usize,
    state_refused: Vec<(ObjectType, P)>,
}

impl Tally {
    fn fail(
        &mut self,
        object: &dyn BACnetObject,
        property: P,
        what: &str,
        result: &Result<(), Error>,
    ) {
        let object_type = object.object_identifier().object_type();
        self.failures
            .push(format!("{object_type} {property}: {what}, got {result:?}"));
    }
}

/// The in-service pass: every row, in row order.
fn in_service(object: &mut dyn BACnetObject, tally: &mut Tally) {
    let object_type = object.object_identifier().object_type();
    let rows = object.property_metadata().into_owned();
    for row in rows {
        let property = row.property_identifier;
        match row.write_capability {
            W::ReadOnly | W::Through(_) | W::WhenOutOfService => {
                let from = write_back(object, property, &test_origin());
                if !access_denied(&from) {
                    tally.fail(
                        object,
                        property,
                        "write_property_from must be denied",
                        &from,
                    );
                }
                let value = current(object, property);
                let direct = object.write_property(property, None, value, None);
                if !access_denied(&direct) {
                    tally.fail(object, property, "write_property must be denied", &direct);
                }
                tally.refused += 1;
            }
            W::Always => {
                let result = write_back(object, property, &test_origin());
                match (access_denied(&result), state_refused(object_type, property)) {
                    (false, false) => {}
                    (true, true) => tally.state_refused.push((object_type, property)),
                    (true, false) => tally.fail(object, property, "must not be denied", &result),
                    (false, true) => {
                        tally.fail(object, property, "STATE_REFUSED expects a denial", &result)
                    }
                }
            }
            W::WhenCommandOwner => {
                // Present_Value's row comes first, so its write above made
                // test_origin the writer this row follows.
                let owner = write_back(object, property, &test_origin());
                if access_denied(&owner) {
                    tally.fail(object, property, "the writer must not be denied", &owner);
                }
                let other = write_back(object, property, &other_origin());
                if !access_denied(&other) {
                    tally.fail(object, property, "another writer must be denied", &other);
                }
                tally.owned += 1;
            }
        }
    }
}

/// The out-of-service pass: the rows only Out_Of_Service opens.
fn out_of_service(object: &mut dyn BACnetObject, tally: &mut Tally) {
    let rows = object.property_metadata().into_owned();
    if !rows
        .iter()
        .any(|row| row.write_capability == W::WhenOutOfService)
    {
        return;
    }
    let result = object.write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None);
    if result.is_err() {
        tally.fail(object, P::OUT_OF_SERVICE, "must take TRUE", &result);
        return;
    }
    for row in rows
        .iter()
        .filter(|row| row.write_capability == W::WhenOutOfService)
    {
        let property = row.property_identifier;
        let result = write_back(object, property, &test_origin());
        if access_denied(&result) {
            tally.fail(
                object,
                property,
                "must not be denied out of service",
                &result,
            );
        }
        tally.out_of_service += 1;
    }
}

#[test]
fn every_row_write_capability_matches_the_write_arm() {
    let mut tally = Tally::default();
    for mut object in representatives() {
        if let Ok(PropertyValue::Boolean(out)) = object.read_property(P::OUT_OF_SERVICE, None) {
            assert!(!out, "{:?} starts in service", object.object_identifier());
        }
        in_service(object.as_mut(), &mut tally);
        out_of_service(object.as_mut(), &mut tally);
    }
    assert!(tally.failures.is_empty(), "{}", tally.failures.join("\n"));
    let listed: Vec<(ObjectType, P)> = STATE_REFUSED.iter().map(|(t, p, _)| (*t, *p)).collect();
    assert_eq!(tally.state_refused, listed, "STATE_REFUSED is exact");
    assert!(
        tally.refused > 600,
        "only {} refusals checked",
        tally.refused
    );
    assert!(
        tally.out_of_service > 20,
        "only {} out-of-service rows",
        tally.out_of_service
    );
    assert!(tally.owned >= 8, "only {} owner rows", tally.owned);
}

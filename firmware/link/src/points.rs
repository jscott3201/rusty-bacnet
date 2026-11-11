//! The points image's scenario (#2445).
//!
//! [`run`] builds its database with `bacnet-objects`' point-list builder,
//! `points::build_database`, from the [`PointList`] it is handed ([`list`]):
//! the objects image's four points (AI-1 and AI-2, BV-1 and BV-2) and one
//! each of the other seven analog, binary and multi-state types, so all nine
//! types the builder covers are in it, each with settings of its family. It
//! then sets AI-1's and AI-2's Present_Value as the application's inputs, and
//! hands `bacnet-device-core`'s `Responder` the objects image's requests
//! ([`APDUS`](crate::objects::APDUS)), which it must answer as the objects
//! image does, then [`READ_APDUS`]: a ReadProperty of the Device's
//! Object_List, which shows the list's order, and a ReadPropertyMultiple of
//! one setting the list gave each of the seven other points.
//!
//! So the image links what a device whose database comes from a point list
//! links, and its size less the objects image's is the builder's price for
//! all nine types: its checks and the setters it calls, and the seven object
//! types the objects image doesn't have, whose constructors the builder
//! reaches whatever the list holds. `firmware/link/size-budget.toml` splits
//! the two. The image passes the list through `black_box`, as it does the
//! requests, so the builder can't be specialized to a list known at build
//! time (without it the image measured the same).
//!
//! The work lives on the heap, in an arena of [`ARENA`] octets.
//! `tests/points.rs` runs the scenario natively and compares the whole
//! [`Summary`] with `expected()`, then runs it again in the image's allocator
//! over that arena and prints what the list, the build and the requests
//! take. The `points` image (`src/bin/points.rs`) links it.

use alloc::vec::Vec;

use bacnet_device_core::responder::{Inbound, Limits, Outcome, Responder, SERVICES};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::points::{
    Access, Analog, Binary, DeviceInfo, MultiState, Point, PointList, build_database,
};
use bacnet_objects::traits::{LocalInputValue, not_supported};
use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};

use crate::objects::{self, ANALOG_VALUES, DEVICE, MAX_REPLY, VENDOR};

/// The heap the scenario runs in, in octets: the objects image's
/// ([`crate::objects::ARENA`]) and 12 KiB, for the seven more objects and the
/// list they are built from.
///
/// On a 64-bit host, whose blocks bound a 32-bit target's,
/// `tests/points.rs` measures, with `bacnet-objects`' capability features
/// off as the image builds it and the identity's 32 octets counted in each
/// figure but the list's: the list at 1,624 octets, the build's peak at
/// 22,144, the database it leaves at 21,672, and the scenario's peak at
/// 25,320. A host that keeps its list and builds from a clone, so that both
/// are live, peaks at 23,768. Most of the database is the outputs and
/// values, about 2 KB each on the host, where an input takes about 0.5 KB.
pub const ARENA: usize = objects::ARENA + 12 * 1024;

/// The requests [`run`] hands the responder after the objects image's.
pub const READS: usize = 2;

/// Each read's APDU, written by hand as the objects image's are
/// ([`APDUS`](crate::objects::APDUS)), invoke IDs 12 and 13.
pub const READ_APDUS: [&[u8]; READS] = [
    // ReadProperty of the Device's (0x020004D2) Object_List (76).
    &[
        0x00, 0x05, 0x0C, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4C,
    ],
    // ReadPropertyMultiple: per object, the identifier in [0] and inside
    // [1] one reference: AO-1's Relinquish_Default (104), AV-1's
    // COV_Increment (22), BI-1's Active_Text (4), BO-1's Inactive_Text
    // (46), MSI-1's State_Text (110), MSO-1's Relinquish_Default and
    // MSV-1's Description (28).
    &[
        0x00, 0x05, 0x0D, 0x0E, //
        0x0C, 0x00, 0x40, 0x00, 0x01, 0x1E, 0x09, 0x68, 0x1F, //
        0x0C, 0x00, 0x80, 0x00, 0x01, 0x1E, 0x09, 0x16, 0x1F, //
        0x0C, 0x00, 0xC0, 0x00, 0x01, 0x1E, 0x09, 0x04, 0x1F, //
        0x0C, 0x01, 0x00, 0x00, 0x01, 0x1E, 0x09, 0x2E, 0x1F, //
        0x0C, 0x03, 0x40, 0x00, 0x01, 0x1E, 0x09, 0x6E, 0x1F, //
        0x0C, 0x03, 0x80, 0x00, 0x01, 0x1E, 0x09, 0x68, 0x1F, //
        0x0C, 0x04, 0xC0, 0x00, 0x01, 0x1E, 0x09, 0x1C, 0x1F,
    ],
];

/// What [`run`] saw, compared whole by the host test.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// What the objects image's requests got: in a good run, what they get
    /// there (`objects::expected()`). Its own error count stays zero.
    pub objects: objects::Summary,
    /// The reply to each of [`READ_APDUS`].
    pub reads: [Vec<u8>; READS],
    /// Calls that failed where the scenario expects success: building the
    /// identity, the database or the responder, or setting an input. Zero
    /// in a good run.
    pub errors: u8,
}

/// The scenario's point list, in the order its Object_List gives them after
/// the Device: AI-1 and AI-2 (no units, read-only), AO-1 (percent, 0 to 100,
/// relinquishing to 50), AV-1 (degrees Celsius, commandable, a COV_Increment
/// of 0.5), BI-1 ("On" and "Off"), BO-1 (relinquishing to INACTIVE, "Run"
/// and "Stop"), BV-1 and BV-2 (commandable, BV-2 relinquishing to ACTIVE),
/// MSI-1 (three states, "Off", "Low" and "High"), MSO-1 (four states,
/// relinquishing to 2) and MSV-1 (two states, writable, described as
/// "Mode"). The Device keeps the identity's name and is described as
/// "Points image". The objects image's four points are as it builds them.
///
/// Every instance is 1 or 2, so making an identifier never fails; the
/// `Result` is `ObjectIdentifier::new`'s.
pub fn list() -> Result<PointList, Error> {
    let oid = ObjectIdentifier::new;
    let points = alloc::vec![
        Point::new(
            oid(ObjectType::ANALOG_INPUT, 1)?,
            "AI-1",
            Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
        ),
        Point::new(
            oid(ObjectType::ANALOG_INPUT, 2)?,
            "AI-2",
            Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
        ),
        Point::new(
            oid(ObjectType::ANALOG_OUTPUT, 1)?,
            "AO-1",
            Analog::new(
                EngineeringUnits::PERCENT,
                Access::Commandable {
                    relinquish_default: Some(50.0),
                },
            )
            .with_min_pres_value(0.0)
            .with_max_pres_value(100.0),
        ),
        Point::new(
            oid(ObjectType::ANALOG_VALUE, 1)?,
            "AV-1",
            Analog::new(
                EngineeringUnits::DEGREES_CELSIUS,
                Access::Commandable {
                    relinquish_default: None,
                },
            )
            .with_cov_increment(0.5),
        ),
        Point::new(
            oid(ObjectType::BINARY_INPUT, 1)?,
            "BI-1",
            Binary::new(Access::ReadOnly)
                .with_active_text("On")
                .with_inactive_text("Off"),
        ),
        Point::new(
            oid(ObjectType::BINARY_OUTPUT, 1)?,
            "BO-1",
            Binary::new(Access::Commandable {
                relinquish_default: Some(BinaryPV::INACTIVE),
            })
            .with_active_text("Run")
            .with_inactive_text("Stop"),
        ),
        Point::new(
            oid(ObjectType::BINARY_VALUE, 1)?,
            "BV-1",
            Binary::new(Access::Commandable {
                relinquish_default: None,
            }),
        ),
        // With no command at any priority, Present_Value is
        // Relinquish_Default.
        Point::new(
            oid(ObjectType::BINARY_VALUE, 2)?,
            "BV-2",
            Binary::new(Access::Commandable {
                relinquish_default: Some(BinaryPV::ACTIVE),
            }),
        ),
        Point::new(
            oid(ObjectType::MULTI_STATE_INPUT, 1)?,
            "MSI-1",
            MultiState::new(3, Access::ReadOnly).with_state_texts(["Off", "Low", "High"]),
        ),
        Point::new(
            oid(ObjectType::MULTI_STATE_OUTPUT, 1)?,
            "MSO-1",
            MultiState::new(
                4,
                Access::Commandable {
                    relinquish_default: Some(2),
                },
            ),
        ),
        Point::new(
            oid(ObjectType::MULTI_STATE_VALUE, 1)?,
            "MSV-1",
            MultiState::new(2, Access::Writable),
        )
        .with_description("Mode"),
    ];
    Ok(PointList::new(
        DeviceInfo::new(DEVICE).with_description("Points image"),
        points,
    ))
}

/// The device's identity, declaring the services the responder runs: the
/// objects image's.
pub fn identity() -> Result<DeviceIdentity, Error> {
    Ok(DeviceIdentity::new(DEVICE, VENDOR)?.with_services(SERVICES))
}

/// Run the whole scenario on `list` ([`list`]), handing the responder
/// `apdus` ([`APDUS`](crate::objects::APDUS)) and then `reads`
/// ([`READ_APDUS`]), all passed in so an image can hide them from the
/// optimizer.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and stops.
pub fn run(list: PointList, apdus: &[&[u8]; objects::REQUESTS], reads: &[&[u8]; READS]) -> Summary {
    let mut summary = Summary {
        objects: objects::Summary {
            outcomes: [Outcome::Unhandled; objects::REQUESTS],
            answers: core::array::from_fn(|_| Vec::new()),
            written: Vec::new(),
            errors: 0,
        },
        reads: core::array::from_fn(|_| Vec::new()),
        errors: 0,
    };
    let Ok(identity) = identity() else {
        summary.errors += 1;
        return summary;
    };
    let Ok(mut db) = build_database(list, &identity) else {
        summary.errors += 1;
        return summary;
    };
    if set_inputs(&mut db).is_err() {
        summary.errors += 1;
        return summary;
    }
    let Ok(mut responder) = Responder::new(identity, Limits::default()) else {
        summary.errors += 1;
        return summary;
    };
    let mut handle = |request: &[u8], answer: &mut Vec<u8>| {
        let mut out = BytesMut::new();
        let inbound = Inbound::new(Bytes::copy_from_slice(request), MAX_REPLY)
            .with_source(objects::requester());
        let handled = responder.handle(&mut db, inbound, &mut out);
        *answer = out.to_vec();
        handled
    };
    let served = &mut summary.objects;
    for ((request, outcome), answer) in apdus
        .iter()
        .zip(&mut served.outcomes)
        .zip(&mut served.answers)
    {
        let handled = handle(request, answer);
        *outcome = handled.outcome;
        served.written.extend(handled.objects);
    }
    for (request, answer) in reads.iter().zip(&mut summary.reads) {
        handle(request, answer);
    }
    summary
}

/// Give AI-1 and AI-2 their Present_Value, [`ANALOG_VALUES`], as the
/// application does for an input once the builder has made it: through the
/// object's local input, before anything serves the database.
fn set_inputs(db: &mut ObjectDatabase) -> Result<(), Error> {
    for (instance, value) in (1..).zip(ANALOG_VALUES) {
        let input = db
            .get_mut(&ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance)?)
            .and_then(|object| object.local_input())
            .ok_or_else(not_supported)?;
        input.apply_local_input(LocalInputValue::present_value(PropertyValue::Real(value)))?;
    }
    Ok(())
}

/// The summary of a good run, which the host test compares with [`run`]'s.
/// Only host builds have it: the images compare nothing.
#[cfg(not(target_os = "none"))]
pub fn expected() -> Summary {
    let reads: [&[u8]; READS] = [
        // ComplexACK (0x30), invoke ID 12, ReadProperty: the request's [0]
        // and [1], then inside [3] the Device and the eleven points as
        // application-tagged identifiers (0xC4), in list order.
        &[
            0x30, 0x0C, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4C, 0x3E, //
            0xC4, 0x02, 0x00, 0x04, 0xD2, // the Device
            0xC4, 0x00, 0x00, 0x00, 0x01, // AI-1
            0xC4, 0x00, 0x00, 0x00, 0x02, // AI-2
            0xC4, 0x00, 0x40, 0x00, 0x01, // AO-1
            0xC4, 0x00, 0x80, 0x00, 0x01, // AV-1
            0xC4, 0x00, 0xC0, 0x00, 0x01, // BI-1
            0xC4, 0x01, 0x00, 0x00, 0x01, // BO-1
            0xC4, 0x01, 0x40, 0x00, 0x01, // BV-1
            0xC4, 0x01, 0x40, 0x00, 0x02, // BV-2
            0xC4, 0x03, 0x40, 0x00, 0x01, // MSI-1
            0xC4, 0x03, 0x80, 0x00, 0x01, // MSO-1
            0xC4, 0x04, 0xC0, 0x00, 0x01, // MSV-1
            0x3F,
        ],
        // ComplexACK, invoke ID 13, ReadPropertyMultiple: per object its
        // identifier in [0] and, inside [1], the property in [2] and the
        // value inside [4].
        &[
            0x30, 0x0D, 0x0E, //
            // AO-1: REAL 50.0.
            0x0C, 0x00, 0x40, 0x00, 0x01, 0x1E, 0x29, 0x68, 0x4E, 0x44, 0x42, 0x48, 0x00, 0x00,
            0x4F, 0x1F, //
            // AV-1: REAL 0.5.
            0x0C, 0x00, 0x80, 0x00, 0x01, 0x1E, 0x29, 0x16, 0x4E, 0x44, 0x3F, 0x00, 0x00, 0x00,
            0x4F, 0x1F, //
            // BI-1: CharacterString (0x73: three octets, the character set
            // 0 then the text) "On".
            0x0C, 0x00, 0xC0, 0x00, 0x01, 0x1E, 0x29, 0x04, 0x4E, 0x73, 0x00, 0x4F, 0x6E, 0x4F,
            0x1F, //
            // BO-1: "Stop" (0x75 0x05: five octets).
            0x0C, 0x01, 0x00, 0x00, 0x01, 0x1E, 0x29, 0x2E, 0x4E, 0x75, 0x05, 0x00, 0x53, 0x74,
            0x6F, 0x70, 0x4F, 0x1F, //
            // MSI-1: the whole array, "Off", "Low", "High".
            0x0C, 0x03, 0x40, 0x00, 0x01, 0x1E, 0x29, 0x6E, 0x4E, 0x74, 0x00, 0x4F, 0x66, 0x66,
            0x74, 0x00, 0x4C, 0x6F, 0x77, 0x75, 0x05, 0x00, 0x48, 0x69, 0x67, 0x68, 0x4F, 0x1F,
            // MSO-1: Unsigned 2.
            0x0C, 0x03, 0x80, 0x00, 0x01, 0x1E, 0x29, 0x68, 0x4E, 0x21, 0x02, 0x4F, 0x1F, //
            // MSV-1: "Mode".
            0x0C, 0x04, 0xC0, 0x00, 0x01, 0x1E, 0x29, 0x1C, 0x4E, 0x75, 0x05, 0x00, 0x4D, 0x6F,
            0x64, 0x65, 0x4F, 0x1F,
        ],
    ];
    Summary {
        objects: objects::expected(),
        reads: reads.map(<[u8]>::to_vec),
        errors: 0,
    }
}

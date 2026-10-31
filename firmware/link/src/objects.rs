//! The objects image's scenario (#2178).
//!
//! [`run`] builds a `bacnet-objects` database, a Device from its
//! `DeviceIdentity` with two Analog Inputs and two Binary Values, and hands
//! each request APDU given to it (the image and the host test give
//! [`REQUESTS`]) to `bacnet-device-core`'s `Responder::handle`, as firmware
//! that owns its database would: Who-Is in range and out of it, a Who-Has,
//! a ReadProperty of each Present_Value, one of an object the device lacks,
//! one whose parameters stop short, and a segmented request. Each request
//! is copied into a `Bytes` of its own, as a receive path would. It returns
//! a [`Summary`]: what `handle` reported for each request, and the APDU it
//! wrote.
//!
//! The work lives on the heap: the database, each request's `Bytes` and
//! each answer. The image gives its allocator an arena of [`ARENA`] octets.
//!
//! `tests/objects.rs` runs it natively and compares the whole summary with
//! `expected()`, then runs it again in the image's allocator over an arena
//! of that size. The `objects` image (`src/bin/objects.rs`) links it.

use alloc::boxed::Box;
use alloc::vec::Vec;

use bacnet_device_core::responder::{Inbound, Limits, Outcome, Responder, SERVICES};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};

/// The heap the objects image gives its allocator, in octets.
///
/// The database is most of the scenario's use: `tests/objects.rs` runs the
/// scenario in the same first-fit allocator over an arena of this size, on
/// a 64-bit host, whose blocks are no smaller than a 32-bit target's, and
/// prints the peak. It fails when the scenario doesn't fit.
pub const ARENA: usize = 16 * 1024;

/// The device's instance.
pub const DEVICE: u32 = 1234;
/// The device's vendor identifier.
pub const VENDOR: u16 = 0x0105;
/// The longest reply the requests' client takes, as its requests say.
pub const MAX_REPLY: u16 = 1476;

/// The requests the scenario hands the responder, in this order.
pub const REQUESTS: usize = 10;

/// Each request's APDU, written by hand from the layouts in Clause 20.1 and
/// the service productions. A confirmed request's header is the type and
/// flags, 0x05 (segments unspecified, APDUs up to 1476), the invoke ID and
/// the service; a ReadProperty's parameters are context `[0]`, the object
/// identifier (type in the top ten bits), and `[1]`, the property.
pub const APDUS: [&[u8]; REQUESTS] = [
    // Who-Is 1000 to 2000 (Unconfirmed-Request 0x10, Who-Is 0x08).
    &[0x10, 0x08, 0x0A, 0x03, 0xE8, 0x1A, 0x07, 0xD0],
    // Who-Is 1 to 100, which leaves the device out.
    &[0x10, 0x08, 0x09, 0x01, 0x19, 0x64],
    // Who-Has by name, "BV-2" (Who-Has 0x07; context [3], five octets: the
    // character set, 0, then the name).
    &[0x10, 0x07, 0x3D, 0x05, 0x00, 0x42, 0x56, 0x2D, 0x32],
    // ReadProperty (0x0C) of Present_Value (85) on AI-1, AI-2 (type 0), BV-1
    // and BV-2 (type 5), invoke IDs 1 to 4.
    &[
        0x00, 0x05, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55,
    ],
    &[
        0x00, 0x05, 0x02, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x02, 0x19, 0x55,
    ],
    &[
        0x00, 0x05, 0x03, 0x0C, 0x0C, 0x01, 0x40, 0x00, 0x01, 0x19, 0x55,
    ],
    &[
        0x00, 0x05, 0x04, 0x0C, 0x0C, 0x01, 0x40, 0x00, 0x02, 0x19, 0x55,
    ],
    // ReadProperty of AI-9, which the device lacks.
    &[
        0x00, 0x05, 0x05, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x09, 0x19, 0x55,
    ],
    // ReadProperty of AI-1 with no property identifier.
    &[0x00, 0x05, 0x06, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01],
    // The first segment of a ReadProperty (segmented and more follow:
    // 0x0C), invoke ID 7, sequence number 0 and window 1.
    &[
        0x0C, 0x05, 0x07, 0x00, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55,
    ],
];

/// AI-1's and AI-2's Present_Value.
pub const ANALOG_VALUES: [f32; 2] = [21.5, 72.5];

/// What [`run`] saw, compared whole by the host test.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// What `Responder::handle` reported for each request.
    pub outcomes: [Outcome; REQUESTS],
    /// The APDU it wrote for each, empty when it wrote none.
    pub answers: [Vec<u8>; REQUESTS],
    /// Calls that failed where the scenario expects success: building the
    /// identity or the database. Zero in a good run.
    pub errors: u8,
}

/// Run the whole scenario, handing the responder `requests`: [`APDUS`],
/// passed in so an image can hide them from the optimizer.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and stops.
pub fn run(requests: &[&[u8]; REQUESTS]) -> Summary {
    let mut summary = Summary {
        outcomes: [Outcome::Unhandled; REQUESTS],
        answers: core::array::from_fn(|_| Vec::new()),
        errors: 0,
    };
    let Ok((identity, mut db)) = device() else {
        summary.errors += 1;
        return summary;
    };
    let responder = Responder::new(identity, Limits::default());
    for ((request, outcome), answer) in requests
        .iter()
        .zip(&mut summary.outcomes)
        .zip(&mut summary.answers)
    {
        let mut out = BytesMut::new();
        let inbound = Inbound {
            apdu: Bytes::copy_from_slice(request),
            max_reply: MAX_REPLY,
        };
        *outcome = responder.handle(&mut db, inbound, &mut out);
        *answer = out.to_vec();
    }
    summary
}

/// The device's identity, declaring the services the responder runs, and
/// its database: the Device, AI-1 and AI-2 with [`ANALOG_VALUES`], BV-1
/// inactive and BV-2 active.
fn device() -> Result<(DeviceIdentity, ObjectDatabase), Error> {
    let identity = DeviceIdentity::new(DEVICE, VENDOR)?.with_services(SERVICES);
    let mut extra: Vec<Box<dyn BACnetObject>> = Vec::new();
    for (instance, value) in (1..).zip(ANALOG_VALUES) {
        let name = if instance == 1 { "AI-1" } else { "AI-2" };
        let mut input = AnalogInputObject::new(instance, name, EngineeringUnits::NO_UNITS)?;
        input.set_present_value(value);
        extra.push(Box::new(input));
    }
    extra.push(Box::new(BinaryValueObject::new(1, "BV-1")?));
    // With no command at any priority, Present_Value is Relinquish_Default.
    let mut active = BinaryValueObject::new(2, "BV-2")?;
    active.set_relinquish_default(1)?;
    extra.push(Box::new(active));
    let db = build_database_with_extra(&identity, extra)?;
    Ok((identity, db))
}

/// The summary of a good run, which the host test compares with
/// [`run`]'s. Only host builds have it: the images compare nothing.
#[cfg(not(target_os = "none"))]
pub fn expected() -> Summary {
    use Outcome::{Announce, Reply, Silent};
    let answers: [&[u8]; REQUESTS] = [
        // I-Am (0x10 0x00): device 1234 (0x020004D2), Unsigned 1476
        // (0x05C4), Enumerated 3 (no segmentation) and Unsigned 261.
        &[
            0x10, 0x00, 0xC4, 0x02, 0x00, 0x04, 0xD2, 0x22, 0x05, 0xC4, 0x91, 0x03, 0x22, 0x01,
            0x05,
        ],
        &[],
        // I-Have (0x10 0x01): the device, BV-2 (0x01400002) and its name,
        // an application CharacterString of five octets.
        &[
            0x10, 0x01, 0xC4, 0x02, 0x00, 0x04, 0xD2, 0xC4, 0x01, 0x40, 0x00, 0x02, 0x75, 0x05,
            0x00, 0x42, 0x56, 0x2D, 0x32,
        ],
        // ComplexACKs (0x30, invoke ID, ReadProperty), the request's [0] and
        // [1], then the value inside [3]: REAL 21.5 and 72.5, Enumerated 0
        // and 1.
        &[
            0x30, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x41, 0xAC,
            0x00, 0x00, 0x3F,
        ],
        &[
            0x30, 0x02, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x02, 0x19, 0x55, 0x3E, 0x44, 0x42, 0x91,
            0x00, 0x00, 0x3F,
        ],
        &[
            0x30, 0x03, 0x0C, 0x0C, 0x01, 0x40, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x91, 0x00, 0x3F,
        ],
        &[
            0x30, 0x04, 0x0C, 0x0C, 0x01, 0x40, 0x00, 0x02, 0x19, 0x55, 0x3E, 0x91, 0x01, 0x3F,
        ],
        // Error (0x50), invoke ID 5, ReadProperty: OBJECT (1), UNKNOWN_OBJECT
        // (31).
        &[0x50, 0x05, 0x0C, 0x91, 0x01, 0x91, 0x1F],
        // Reject (0x60), invoke ID 6: MISSING_REQUIRED_PARAMETER (5).
        &[0x60, 0x06, 0x05],
        // Abort from the server (0x71), invoke ID 7:
        // SEGMENTATION_NOT_SUPPORTED (4).
        &[0x71, 0x07, 0x04],
    ];
    Summary {
        outcomes: [
            Announce, Silent, Announce, Reply, Reply, Reply, Reply, Reply, Reply, Reply,
        ],
        answers: answers.map(<[u8]>::to_vec),
        errors: 0,
    }
}

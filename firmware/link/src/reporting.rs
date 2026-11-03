//! The reporting image's scenario (#2303).
//!
//! [`run`] first runs the objects image's scenario (`crate::objects::run`:
//! the `Responder` answering reads, writes and discovery), so the image
//! links what a device that reports also serves, and its size less the
//! objects image's is what COV costs. Then it builds the objects image's
//! database again (the Device, two Analog Inputs and two Binary Values) and
//! a `bacnet-device-core` `CovTable` capped at [`CAP`] subscriptions, and
//! hands the table each request in
//! [`REQUESTS`] through the core's staged functions,
//! `CovTable::subscribe_cov` and `subscribe_cov_property`, as firmware that
//! owns its table would: a SubscribeCOV of AI-1, a SubscribeCOVProperty of
//! BV-1's Present_Value, a renewal of the first, six more subscriptions up
//! to the cap, one past it, which is refused, and the cancellation of the
//! first two. After each request it records the reply it would send and how
//! many subscriptions the table holds. Last, it runs the expiry pass once
//! every lifetime left has run out. The core sends no notifications yet.
//!
//! The work lives on the heap, in an arena of the objects image's size
//! ([`ARENA`](crate::objects::ARENA)). `tests/reporting.rs` runs the
//! scenario natively and compares the whole [`Summary`] with `expected()`,
//! then runs it again in the image's allocator over that arena. The
//! `reporting` image (`src/bin/reporting.rs`) links it.

use alloc::vec::Vec;

use bacnet_device_core::cov::{CovCounters, CovPolicy, CovTable, SubscriberEndpoint};
use bacnet_device_core::reply::error_apdu;
use bacnet_encoding::apdu::{Apdu, SimpleAck, encode_apdu};
use bacnet_time::{Duration, Instant};
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::BytesMut;

/// The most subscriptions the table holds, all subscribers together.
pub const CAP: usize = 8;

/// The requests [`run`] hands the table, in this order.
pub const STEPS: usize = 12;

/// One request: the service, the MS/TP station it comes from, the second
/// it arrives at, and its parameters.
#[derive(Debug, Clone, Copy)]
pub struct Request {
    /// SubscribeCOVProperty when set, SubscribeCOV otherwise.
    pub property: bool,
    /// The subscriber's station.
    pub peer: u8,
    /// When the request arrives, in seconds from the start.
    pub at: u64,
    /// The service parameters.
    pub body: &'static [u8],
}

const fn cov(peer: u8, at: u64, body: &'static [u8]) -> Request {
    Request {
        property: false,
        peer,
        at,
        body,
    }
}

const fn property(peer: u8, at: u64, body: &'static [u8]) -> Request {
    Request {
        property: true,
        peer,
        at,
        body,
    }
}

/// Each request's parameters, written by hand from the SubscribeCOV and
/// SubscribeCOVProperty productions: `[0]` the subscriber's process, `[1]`
/// the object (type in the top ten bits), `[2]` whether notifications are
/// confirmed and `[3]` the lifetime in seconds; a property subscription
/// adds `[4]`, the property reference, Present_Value (85) in its `[0]`.
pub const REQUESTS: [Request; STEPS] = [
    // AI-1, process 1, unconfirmed, 60 s.
    cov(
        7,
        0,
        &[
            0x09, 0x01, 0x1C, 0x00, 0x00, 0x00, 0x01, 0x29, 0x00, 0x39, 0x3C,
        ],
    ),
    // BV-1's Present_Value, process 2, confirmed, 120 s.
    property(
        7,
        0,
        &[
            0x09, 0x02, 0x1C, 0x01, 0x40, 0x00, 0x01, 0x29, 0x01, 0x39, 0x78, 0x4E, 0x09, 0x55,
            0x4F,
        ],
    ),
    // AI-1 again, process 1: a renewal, now confirmed, for 300 s.
    cov(
        7,
        10,
        &[
            0x09, 0x01, 0x1C, 0x00, 0x00, 0x00, 0x01, 0x29, 0x01, 0x3A, 0x01, 0x2C,
        ],
    ),
    // AI-2 from station 8, processes 10 to 15, unconfirmed, 30 s: the
    // table reaches its cap.
    cov(
        8,
        10,
        &[
            0x09, 0x0A, 0x1C, 0x00, 0x00, 0x00, 0x02, 0x29, 0x00, 0x39, 0x1E,
        ],
    ),
    cov(
        8,
        10,
        &[
            0x09, 0x0B, 0x1C, 0x00, 0x00, 0x00, 0x02, 0x29, 0x00, 0x39, 0x1E,
        ],
    ),
    cov(
        8,
        10,
        &[
            0x09, 0x0C, 0x1C, 0x00, 0x00, 0x00, 0x02, 0x29, 0x00, 0x39, 0x1E,
        ],
    ),
    cov(
        8,
        10,
        &[
            0x09, 0x0D, 0x1C, 0x00, 0x00, 0x00, 0x02, 0x29, 0x00, 0x39, 0x1E,
        ],
    ),
    cov(
        8,
        10,
        &[
            0x09, 0x0E, 0x1C, 0x00, 0x00, 0x00, 0x02, 0x29, 0x00, 0x39, 0x1E,
        ],
    ),
    cov(
        8,
        10,
        &[
            0x09, 0x0F, 0x1C, 0x00, 0x00, 0x00, 0x02, 0x29, 0x00, 0x39, 0x1E,
        ],
    ),
    // Process 16: one past the cap, refused.
    cov(
        8,
        10,
        &[
            0x09, 0x10, 0x1C, 0x00, 0x00, 0x00, 0x02, 0x29, 0x00, 0x39, 0x1E,
        ],
    ),
    // Cancellations: no confirmed flag and no lifetime.
    cov(7, 20, &[0x09, 0x01, 0x1C, 0x00, 0x00, 0x00, 0x01]),
    property(
        7,
        20,
        &[
            0x09, 0x02, 0x1C, 0x01, 0x40, 0x00, 0x01, 0x4E, 0x09, 0x55, 0x4F,
        ],
    ),
];

/// When [`run`] runs the expiry pass, in seconds: after every lifetime the
/// requests left.
pub const EXPIRY_AT: u64 = 100;

/// What [`run`] saw, compared whole by the host test.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The objects image's scenario, run first.
    pub objects: crate::objects::Summary,
    /// The reply to each request: a SimpleACK, or the Error or Reject.
    pub answers: [Vec<u8>; STEPS],
    /// How many subscriptions the table held after each request.
    pub held: [usize; STEPS],
    /// The subscriptions the expiry pass removed, and how many it left.
    pub expired: (usize, usize),
    /// The table's subscription counts over the whole run.
    pub counts: CovCounters,
    /// Calls that failed where the scenario expects success: building the
    /// database or encoding a reply. Zero in a good run.
    pub errors: u8,
}

/// Run the whole scenario: the objects image's on `apdus`
/// ([`APDUS`](crate::objects::APDUS)), then the table's on `requests`
/// ([`REQUESTS`]), both passed in so an image can hide them from the
/// optimizer.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and stops.
pub fn run(apdus: &[&[u8]; crate::objects::REQUESTS], requests: &[Request; STEPS]) -> Summary {
    let mut summary = Summary {
        objects: crate::objects::run(apdus),
        answers: core::array::from_fn(|_| Vec::new()),
        held: [0; STEPS],
        expired: (0, 0),
        counts: CovCounters::default(),
        errors: 0,
    };
    let Ok((_, db)) = crate::objects::device() else {
        summary.errors += 1;
        return summary;
    };
    let mut table = CovTable::new(CovPolicy {
        max_subscriptions_global: CAP,
        ..CovPolicy::default()
    });
    let start = Instant::ZERO;
    for (((invoke_id, request), answer), held) in (1u8..)
        .zip(requests)
        .zip(&mut summary.answers)
        .zip(&mut summary.held)
    {
        let peer = SubscriberEndpoint::new(&[request.peer], None);
        let now = start + Duration::from_secs(request.at);
        let (service, result) = if request.property {
            let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY;
            (
                service,
                table.subscribe_cov_property(&db, &peer, request.body, now),
            )
        } else {
            let service = ConfirmedServiceChoice::SUBSCRIBE_COV;
            (service, table.subscribe_cov(&db, &peer, request.body, now))
        };
        let reply = match result {
            Ok(_) => Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: service,
            }),
            Err(error) => error_apdu(invoke_id, service, &error),
        };
        let mut out = BytesMut::new();
        if encode_apdu(&mut out, &reply).is_err() {
            summary.errors += 1;
            return summary;
        }
        *answer = out.to_vec();
        *held = table.len();
        add(&mut summary.counts, table.take_counts());
    }
    let expired = table.expire(start + Duration::from_secs(EXPIRY_AT));
    summary.expired = (expired, table.len());
    add(&mut summary.counts, table.take_counts());
    summary
}

/// Add `counts` to `total`, taking its live count.
fn add(total: &mut CovCounters, counts: CovCounters) {
    total.subscriptions_created += counts.subscriptions_created;
    total.subscriptions_cancelled += counts.subscriptions_cancelled;
    total.subscriptions_purged += counts.subscriptions_purged;
    total.subscriptions_rejected_quota += counts.subscriptions_rejected_quota;
    total.subscriptions_rejected_capacity += counts.subscriptions_rejected_capacity;
    total.subscriptions_rejected_indefinite += counts.subscriptions_rejected_indefinite;
    total.subscriptions_active = counts.subscriptions_active;
}

/// The summary of a good run, which the host test compares with
/// [`run`]'s. Only host builds have it: the images compare nothing.
#[cfg(not(target_os = "none"))]
pub fn expected() -> Summary {
    // SimpleACK (0x20), the invoke ID, then SubscribeCOV (5) or
    // SubscribeCOVProperty (28).
    let ack = |invoke_id: u8, service: u8| alloc::vec![0x20, invoke_id, service];
    Summary {
        objects: crate::objects::expected(),
        answers: [
            ack(1, 5),
            ack(2, 28),
            ack(3, 5),
            ack(4, 5),
            ack(5, 5),
            ack(6, 5),
            ack(7, 5),
            ack(8, 5),
            ack(9, 5),
            // Error (0x50), invoke ID 10, SubscribeCOV: RESOURCES (3),
            // NO_SPACE_TO_ADD_LIST_ELEMENT (19).
            alloc::vec![0x50, 0x0A, 0x05, 0x91, 0x03, 0x91, 0x13],
            ack(11, 5),
            ack(12, 28),
        ],
        held: [1, 2, 2, 3, 4, 5, 6, 7, 8, 8, 7, 6],
        // The six subscriptions of AI-2 lapsed at 40 s.
        expired: (6, 0),
        counts: CovCounters {
            subscriptions_active: 0,
            subscriptions_created: 8,
            subscriptions_rejected_capacity: 1,
            subscriptions_cancelled: 2,
            subscriptions_purged: 6,
            ..CovCounters::default()
        },
        errors: 0,
    }
}

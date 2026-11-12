//! The reporting image's scenario (#2303).
//!
//! [`run`] builds the objects image's database (the Device, two Analog
//! Inputs and two Binary Values), its Device declaring the `Reporter`'s
//! services, and a `bacnet-device-core` `Reporter` capped at [`CAP`]
//! subscriptions with an outbox of [`OUTBOX`]. It first hands the reporter
//! the objects image's requests ([`APDUS`](crate::objects::APDUS)), which
//! it must answer as the objects image's `Responder` does, so the image
//! links what a device that reports also serves, through one request path,
//! and its size less the objects image's is what COV costs (#2363). Then
//! it runs the reporter as firmware's main loop would: each request in [`REQUESTS`]
//! goes through `Reporter::handle` at its second, the deadlines on the way
//! through `on_deadline`, and after each step every notification `pop`
//! hands out is sent (an unconfirmed one reported sent, a confirmed one
//! from 10 s on acknowledged). The requests are a SubscribeCOV of AI-1, a
//! confirmed SubscribeCOVProperty of BV-1's Present_Value, which nobody
//! answers, so it is sent again three times and then fails, a renewal of
//! the first, now confirmed, six more subscriptions up to the cap, one past
//! it, which is refused, and the cancellation of the first two.
//!
//! Then the application's own control loop runs (#2446), [`CYCLES`] cycles
//! a second apart from [`LOOP_AT`], through the `Reporter`'s local writes.
//! Each cycle gives AI-2 its reading ([`READINGS`]) with `Reporter::update`,
//! which AI-2's six subscribers hear of, and runs BV-2 through
//! [`COMMANDS`] as the local Device with `command` and `relinquish`: a
//! command at 8, one at 12 below it, which leaves Present_Value as it was,
//! relinquishing 8, which lets 12's value take over, and relinquishing 12,
//! which leaves Relinquish_Default. Nobody subscribes to BV-2, so its
//! steps report nothing. The last cycle also finds AI-2's sensor at fault
//! ([`FAULT`]) and sets its Reliability with `set_reliability`, which the
//! six hear of as Status_Flags with FAULT.
//!
//! Then a WriteProperty takes AI-2 out of service ([`CHANGE`]), which its
//! six subscribers hear of, a ReadProperty of the Device's
//! Active_COV_Subscriptions ([`READ_LIST`]) lists them, the application's
//! next reading of AI-2 is refused, since a client's out-of-service value
//! stands, and at [`EXPIRY_AT`] every lifetime left has run out.
//!
//! With the crate's `reporting-multiple` feature, which only a measurement
//! build and the host test turn on (#2320), the scenario goes on once those
//! six have lapsed: a SubscribeCOVPropertyMultiple (`MULTIPLE`) subscribes
//! AI-1's Present_Value with timestamps, which reports it at once with the
//! Device's time, and a WriteProperty takes AI-1 out of service
//! (`MULTIPLE_CHANGE`), which that context hears of with the time of the
//! change.
//!
//! The reporter's `Responder` has the core's Device clock (#2321), on a
//! timer the scenario moves with each request, in UTC_Offset [`ZONE`]. A
//! UTCTimeSynchronization ([`SYNC`]) at [`SYNC_AT`] sets it, and two
//! ReadProperty requests then read the Device's Local_Date and Local_Time
//! ([`READ_CLOCK`]), so the image links the clock, and the size check's
//! `core::fmt` and 64-bit division counts cover it. The host then moves the
//! zone into daylight-saving time (#2395), which the clock's recipient
//! window shows an hour on.
//!
//! The work lives on the heap, in an arena of [`ARENA`] octets.
//! `tests/reporting.rs` runs the
//! scenario natively and compares the whole [`Summary`] with `expected()`,
//! then runs it again in the image's allocator over that arena. The
//! `reporting` image (`src/bin/reporting.rs`) links it.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use bacnet_device_core::clock::{self, DeviceClock, Zone};
use bacnet_device_core::cov::{CovPolicy, Delivery, SubscriberEndpoint};
use bacnet_device_core::report::{
    self, Bindings, Destination, InvokeIds, LocalValue, OutboxLimits, ReportConfig, Reporter,
    peer_address,
};
use bacnet_device_core::responder::{Inbound, Limits, Outcome, Responder};
use bacnet_device_core::write::Applied;
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, SimpleAck, encode_apdu};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::traits::BACnetObject;
use bacnet_time::{Duration, Instant};
use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{
    BinaryPV, ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};
use bytes::{Bytes, BytesMut};

/// The most subscriptions the table holds, all subscribers together.
pub const CAP: usize = 8;

/// The heap the scenario runs in, in octets: the objects image's
/// ([`crate::objects::ARENA`]) and 8 KiB, which the subscriptions' baselines,
/// the outbox and the confirmed table take (`tests/reporting.rs` prints the
/// peak).
#[cfg(not(feature = "reporting-multiple"))]
pub const ARENA: usize = crate::objects::ARENA + 8 * 1024;

/// The heap the scenario runs in with a COV-multiple context (#2320), in
/// octets: 12 KiB more, for that report's histories, reads, claim and
/// notification while it is built (`tests/reporting.rs` prints a peak of
/// 32,560 octets on a 64-bit host, where the scenario without it peaks at
/// 19,768).
#[cfg(feature = "reporting-multiple")]
pub const ARENA: usize = crate::objects::ARENA + 20 * 1024;

/// The requests [`run`] hands the reporter, in this order.
pub const STEPS: usize = 12;

/// The outbox: room for the six notifications AI-2's subscribers are sent
/// at once, for each of the control loop's readings, for its fault and for
/// [`CHANGE`], 34 octets of parameters at most each.
pub const OUTBOX: OutboxLimits = OutboxLimits {
    entries: 6,
    octets: 256,
};

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

/// WriteProperty of AI-2's Out_Of_Service, TRUE, from station 9 at
/// [`CHANGE_AT`]: `[0]` the object, `[1]` Out_Of_Service (81), `[3]` the
/// value. It changes AI-2's Status_Flags.
pub const CHANGE: &[u8] = &[0x0C, 0x00, 0x00, 0x00, 0x02, 0x19, 0x51, 0x3E, 0x11, 0x3F];

/// When [`CHANGE`] arrives, in seconds.
pub const CHANGE_AT: u64 = 25;

/// ReadProperty of the Device's (1234, 0x020004D2) Active_COV_Subscriptions
/// (152), a second after [`CHANGE`].
pub const READ_LIST: &[u8] = &[0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x98];

/// How many cycles the application's control loop runs, one a second.
pub const CYCLES: usize = 4;

/// When the control loop's first cycle runs, in seconds: after the
/// cancellations, so AI-2's six subscriptions are all the table holds, and
/// before [`CHANGE`].
pub const LOOP_AT: u64 = 21;

/// AI-2's reading in each cycle, which the application gives it with
/// `Reporter::update`. Each differs from the last, and AI-2's
/// COV_Increment is 0.0, so each is reported.
pub const READINGS: [f32; CYCLES] = [73.0, 73.5, 74.0, 74.5];

/// The fault the last cycle finds in AI-2's sensor, which the application
/// sets as its Reliability with `Reporter::set_reliability`.
pub const FAULT: Reliability = Reliability::OVER_RANGE;

/// BV-2's steps in each cycle, as the local Device: a priority, and the
/// value commanded there, or `None` to relinquish it. ACTIVE at 8 leaves
/// Present_Value ACTIVE, as Relinquish_Default has it; INACTIVE at 12 is
/// below 8, so Present_Value stays ACTIVE; relinquishing 8 lets 12's
/// INACTIVE take over; and relinquishing 12 leaves Relinquish_Default,
/// ACTIVE, again.
pub const COMMANDS: [(u8, Option<BinaryPV>); 4] = [
    (8, Some(BinaryPV::ACTIVE)),
    (12, Some(BinaryPV::INACTIVE)),
    (8, None),
    (12, None),
];

/// A local write's result as the summary keeps it: what `Reporter` did
/// ([`Applied`]), or the refusal as a peer would hear it
/// (`Error::remote`), `None` for one no peer could.
pub type Local = Result<Applied, Option<RemoteError>>;

/// What one cycle of the control loop did.
#[derive(Debug, Clone, PartialEq)]
pub struct Cycle {
    /// What `Reporter::update` did with AI-2's reading, and the octets of
    /// the first notification it sent.
    pub update: (Local, Vec<u8>),
    /// What each of [`COMMANDS`] did to BV-2, and BV-2's Present_Value
    /// after it.
    pub commands: [(Local, Option<BinaryPV>); 4],
}

impl Cycle {
    /// A cycle that hasn't run: nothing written and no Present_Value.
    const NOT_RUN: Self = Self {
        update: (Err(None), Vec::new()),
        commands: [(Err(None), None); 4],
    };
}

#[cfg(feature = "reporting-multiple")]
/// SubscribeCOVPropertyMultiple from station 7 at [`MULTIPLE_AT`]: `[0]`
/// process 3, `[1]` unconfirmed, `[2]` a lifetime of 30 s, `[3]` a
/// Max_Notification_Delay of 5 s, and in `[4]` one specification: `[0]`
/// AI-1 and in `[1]` one reference, `[0]` Present_Value (85) with `[2]`
/// timestamps.
pub const MULTIPLE: &[u8] = &[
    0x09, 0x03, 0x19, 0x00, 0x29, 0x1E, 0x39, 0x05, 0x4E, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x0E,
    0x09, 0x55, 0x0F, 0x29, 0x01, 0x1F, 0x4F,
];

#[cfg(feature = "reporting-multiple")]
/// When [`MULTIPLE`] arrives, in seconds: after the subscriptions of AI-2
/// have lapsed, so the table has room under its cap.
pub const MULTIPLE_AT: u64 = 45;

#[cfg(feature = "reporting-multiple")]
/// WriteProperty of AI-1's Out_Of_Service, TRUE, from station 9, five
/// seconds after [`MULTIPLE`]: as [`CHANGE`], for the object the
/// COV-multiple context monitors.
pub const MULTIPLE_CHANGE: &[u8] = &[0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x51, 0x3E, 0x11, 0x3F];

/// When [`run`] runs the last deadlines, in seconds: after every lifetime
/// the requests left.
pub const EXPIRY_AT: u64 = 100;

/// The Device's UTC_Offset, in minutes west of UTC: five hours, with no
/// daylight-saving time.
pub const ZONE: i16 = 300;

/// A UTCTimeSynchronization (Unconfirmed-Request 0x10, service 9): an
/// application-tagged Date, 2026-11-04 (year 126), a Wednesday (3), then a
/// Time, 15:30:00.00, from station 9 at [`SYNC_AT`].
pub const SYNC: &[u8] = &[
    0x10, 0x09, 0xA4, 0x7E, 0x0B, 0x04, 0x03, 0xB4, 0x0F, 0x1E, 0x00, 0x00,
];

/// When [`SYNC`] arrives, in seconds.
pub const SYNC_AT: u64 = 27;

/// ReadProperty of the Device's Local_Date (56) and Local_Time (57), a
/// second and a half after [`SYNC`].
pub const READ_CLOCK: [&[u8]; 2] = [
    &[0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x38],
    &[0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x39],
];

/// What [`run`] saw, compared whole by the host test.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The objects image's scenario, run first.
    pub objects: crate::objects::Summary,
    /// The reply to each request: a SimpleACK, or the Error or Reject.
    pub answers: [Vec<u8>; STEPS],
    /// How many subscriptions the table held after each request.
    pub held: [usize; STEPS],
    /// Every notification sent, in order: its subscriber's station, its
    /// invoke ID when confirmed, and its APDU's length in octets.
    pub sent: Vec<(u8, Option<u8>, usize)>,
    /// The control loop's cycles, in order.
    pub control: [Cycle; CYCLES],
    /// What `Reporter::set_reliability` did with [`FAULT`] in the last
    /// cycle, and the octets of the first notification it sent.
    pub fault: (Local, Vec<u8>),
    /// The replies to [`CHANGE`] and to [`READ_LIST`].
    pub change: Vec<u8>,
    /// See [`change`](Self::change).
    pub list: Vec<u8>,
    /// What `Reporter::update` did with AI-2's next reading once
    /// [`CHANGE`] had taken it out of service.
    pub refused: Local,
    /// What `handle` did with [`SYNC`], and the replies to
    /// [`READ_CLOCK`].
    pub clock: (Outcome, [Vec<u8>; 2]),
    /// The clock's recipient window at the instant the reads were answered,
    /// once the host has moved the zone into daylight-saving time: the
    /// local day and time, an hour on from the reads'. The time before the
    /// change is the Local_Time the second read returned; a second window
    /// here would cost the RISC-V image flash.
    pub window: Option<(DaysOfWeek, Time)>,
    /// The replies to [`MULTIPLE`] and to [`MULTIPLE_CHANGE`].
    #[cfg(feature = "reporting-multiple")]
    pub multiple: [Vec<u8>; 2],
    /// Subscriptions left at [`EXPIRY_AT`].
    pub left: usize,
    /// The reporter's counts: subscriptions created, cancelled, purged and
    /// refused for capacity; notifications sent, confirmed and
    /// unconfirmed; resends, failures and reports owed.
    pub counts: [u64; 10],
    /// Calls that failed where the scenario expects success: building the
    /// database or the reporter, or encoding a request. Zero in a good run.
    pub errors: u8,
}

/// Invoke IDs in turn; nothing else shares them.
struct Ids(u8);

impl InvokeIds for Ids {
    fn take(&mut self, _: &BACnetAddress, _: ConfirmedServiceChoice) -> Option<u8> {
        let id = self.0;
        self.0 = self.0.wrapping_add(1);
        Some(id)
    }
    fn release(&mut self, _: &BACnetAddress, _: u8) {}
}

/// No Device recipients: COV notifications go to their subscribers.
struct NoBindings;

impl Bindings for NoBindings {
    fn address_of(&self, _: ObjectIdentifier) -> Option<BACnetAddress> {
        None
    }
}

/// The device's main loop around its reporter.
struct Device {
    reporter: Reporter,
    db: ObjectDatabase,
    ids: Ids,
    invoke_id: u8,
    /// The Device clock's timer, in milliseconds: the instant of the
    /// request being handled.
    timer: Arc<AtomicU32>,
}

impl Device {
    /// Hand the unconfirmed `apdu` from station 9 to the reporter at `now`;
    /// what it did.
    fn unconfirmed(&mut self, apdu: &'static [u8], now: Instant) -> Outcome {
        self.tick(now);
        let peer = SubscriberEndpoint::new(&[9], None);
        let inbound = Inbound::new(Bytes::from_static(apdu), crate::objects::MAX_REPLY);
        let mut out = BytesMut::new();
        self.reporter
            .handle(&mut self.db, &peer, inbound, now, &mut out)
            .outcome
    }

    /// Move the clock's timer to `now`.
    fn tick(&self, now: Instant) {
        let millis = u32::try_from(now.as_millis()).unwrap_or(u32::MAX);
        self.timer.store(millis, Ordering::Relaxed);
    }

    /// Hand the confirmed request `service` with `body` from `station` to
    /// the reporter at `now`, after the deadlines before it; its reply.
    fn request(
        &mut self,
        station: u8,
        service: ConfirmedServiceChoice,
        body: &'static [u8],
        now: Instant,
        summary: &mut Summary,
    ) -> Option<Vec<u8>> {
        self.until(now, summary);
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let mut apdu = BytesMut::new();
        let request = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: crate::objects::MAX_REPLY,
            invoke_id: self.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: Bytes::from_static(body),
        });
        encode_apdu(&mut apdu, &request).ok()?;
        self.tick(now);
        let peer = SubscriberEndpoint::new(&[station], None);
        let inbound = Inbound::new(apdu.freeze(), crate::objects::MAX_REPLY);
        let mut out = BytesMut::new();
        self.reporter
            .handle(&mut self.db, &peer, inbound, now, &mut out);
        self.send(now, summary, false);
        Some(out.to_vec())
    }

    /// Run every deadline up to `now`, sending what each calls for.
    fn until(&mut self, now: Instant, summary: &mut Summary) {
        while let Some(due) = self.reporter.next_deadline().filter(|due| *due <= now) {
            self.reporter.on_deadline(&self.db, &mut self.ids, due);
            self.send(due, summary, false);
        }
    }

    /// Run the application's control loop (#2446) on AI-2 (`ai2`) and BV-2
    /// (`bv2`): [`CYCLES`] cycles from [`LOOP_AT`], each after the deadlines
    /// before it, sending what each change reports before the next.
    fn control(&mut self, ai2: ObjectIdentifier, bv2: ObjectIdentifier, summary: &mut Summary) {
        for (n, reading) in READINGS.into_iter().enumerate() {
            let now = Instant::from_secs(LOOP_AT + n as u64);
            self.until(now, summary);
            self.tick(now);
            let mut cycle = Cycle::NOT_RUN;
            let value = LocalValue::Real(reading);
            let updated = self.reporter.update(&mut self.db, ai2, value, now);
            cycle.update = (local(updated), self.send(now, summary, true));
            if n + 1 == CYCLES {
                let faulted = self.reporter.set_reliability(&mut self.db, ai2, FAULT, now);
                summary.fault = (local(faulted), self.send(now, summary, true));
            }
            for (step, (priority, value)) in cycle.commands.iter_mut().zip(COMMANDS) {
                let applied = match value {
                    Some(value) => {
                        let value = LocalValue::Binary(value);
                        self.reporter
                            .command(&mut self.db, bv2, priority, value, now)
                    }
                    None => self.reporter.relinquish(&mut self.db, bv2, priority, now),
                };
                self.send(now, summary, false);
                *step = (local(applied), self.binary(bv2));
            }
            summary.control[n] = cycle;
        }
    }

    /// `oid`'s Present_Value, when it is a binary state.
    fn binary(&self, oid: ObjectIdentifier) -> Option<BinaryPV> {
        let object = self.db.get(&oid)?;
        match object.read_property(PropertyIdentifier::PRESENT_VALUE, None) {
            Ok(PropertyValue::Enumerated(state)) => Some(BinaryPV::from_raw(state)),
            _ => None,
        }
    }

    /// Send every notification the outbox hands out at `now`: report an
    /// unconfirmed one sent, and acknowledge a confirmed one from 10 s on.
    /// With `keep`, the octets of the first one sent; nothing otherwise.
    fn send(&mut self, now: Instant, summary: &mut Summary, keep: bool) -> Vec<u8> {
        let mut kept = Vec::new();
        let mut out = BytesMut::new();
        while let Some(sent) = self.reporter.pop(&NoBindings, &mut self.ids, now, &mut out) {
            let Destination::Subscriber(to) = sent.to else {
                continue;
            };
            summary.sent.push((to.mac[0], sent.invoke_id, out.len()));
            if keep && kept.is_empty() {
                kept.extend_from_slice(&out);
            }
            out.clear();
            match sent.invoke_id {
                None => self
                    .reporter
                    .delivered(sent.token, Delivery::Sent, &mut self.ids, now),
                Some(invoke_id) if now >= Instant::from_secs(10) => {
                    let ack = Apdu::SimpleAck(SimpleAck {
                        invoke_id,
                        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                    });
                    if encode_apdu(&mut out, &ack).is_ok() {
                        self.reporter
                            .reply(&peer_address(&to), &out, &mut self.ids, now);
                    }
                    out.clear();
                }
                Some(_) => {}
            }
        }
        kept
    }
}

/// `result` as the summary keeps it ([`Local`]).
fn local(result: Result<Applied, Error>) -> Local {
    result.map_err(|error| error.remote())
}

/// The objects image's database (the Device, AI-1 and AI-2 with
/// [`ANALOG_VALUES`](crate::objects::ANALOG_VALUES), BV-1 inactive and BV-2
/// active), its Device declaring the `Reporter`'s services, so it lists
/// Active_COV_Subscriptions, and the clock's two synchronizations. Built
/// here, so the objects image's own builder stays as it is.
fn device() -> Result<(DeviceIdentity, ObjectDatabase), Error> {
    use crate::objects::{ANALOG_VALUES, DEVICE, VENDOR};
    let services: Vec<_> = report::SERVICES
        .iter()
        .chain(clock::SERVICES)
        .copied()
        .collect();
    let identity = DeviceIdentity::new(DEVICE, VENDOR)?.with_services(&services);
    let mut extra: Vec<Box<dyn BACnetObject>> = Vec::new();
    for (instance, value) in (1..).zip(ANALOG_VALUES) {
        let name = if instance == 1 { "AI-1" } else { "AI-2" };
        let mut input = AnalogInputObject::new(instance, name, EngineeringUnits::NO_UNITS)?;
        input.set_present_value(value);
        extra.push(Box::new(input));
    }
    extra.push(Box::new(BinaryValueObject::new(1, "BV-1")?));
    let mut active = BinaryValueObject::new(2, "BV-2")?;
    active.set_relinquish_default(1)?;
    extra.push(Box::new(active));
    let db = build_database_with_extra(&identity, extra)?;
    Ok((identity, db))
}

/// Run the whole scenario: the objects image's on `apdus`
/// ([`APDUS`](crate::objects::APDUS)), then the reporter's on `requests`
/// ([`REQUESTS`]), both passed in so an image can hide them from the
/// optimizer.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and stops.
pub fn run(apdus: &[&[u8]; crate::objects::REQUESTS], requests: &[Request; STEPS]) -> Summary {
    let mut summary = Summary {
        objects: crate::objects::Summary {
            outcomes: [Outcome::Unhandled; crate::objects::REQUESTS],
            answers: core::array::from_fn(|_| Vec::new()),
            written: Vec::new(),
            errors: 0,
        },
        answers: core::array::from_fn(|_| Vec::new()),
        held: [0; STEPS],
        sent: Vec::new(),
        control: [Cycle::NOT_RUN; CYCLES],
        fault: (Err(None), Vec::new()),
        change: Vec::new(),
        list: Vec::new(),
        refused: Err(None),
        clock: (Outcome::Unhandled, [Vec::new(), Vec::new()]),
        window: None,
        #[cfg(feature = "reporting-multiple")]
        multiple: [Vec::new(), Vec::new()],
        left: 0,
        counts: [0; 10],
        errors: 0,
    };
    let timer = Arc::new(AtomicU32::new(0));
    let built = device().and_then(|(identity, mut db)| {
        let millis = Arc::clone(&timer);
        let now = move || Instant::from_millis(u64::from(millis.load(Ordering::Relaxed)));
        let clock = DeviceClock::new(Zone::new(ZONE, false)?, Arc::new(now));
        let responder = Responder::new(identity, Limits::default())?.with_clock(clock, &mut db);
        let config = ReportConfig::new(CovPolicy {
            max_subscriptions_global: CAP,
            ..CovPolicy::default()
        })
        .with_outbox(OUTBOX);
        Ok((Reporter::new(responder, config)?, db))
    });
    let Ok((reporter, db)) = built else {
        summary.errors += 1;
        return summary;
    };
    let mut device = Device {
        reporter,
        db,
        ids: Ids(0),
        invoke_id: 0,
        timer,
    };
    // The objects image's requests first, through the same reporter: one
    // device answering them as a device that reports does, which must
    // answer as the objects image's `Responder` alone does.
    let requester = SubscriberEndpoint::new(&[crate::objects::REQUESTER], None);
    let objects = &mut summary.objects;
    for ((request, outcome), answer) in apdus
        .iter()
        .zip(&mut objects.outcomes)
        .zip(&mut objects.answers)
    {
        let mut out = BytesMut::new();
        let inbound = Inbound::new(Bytes::copy_from_slice(request), crate::objects::MAX_REPLY)
            .with_source(crate::objects::requester());
        let handled =
            device
                .reporter
                .handle(&mut device.db, &requester, inbound, Instant::ZERO, &mut out);
        *outcome = handled.outcome;
        objects.written.extend(handled.objects);
        *answer = out.to_vec();
    }
    // A write the requests make reports nothing yet: nobody subscribes.
    device.send(Instant::ZERO, &mut summary, false);
    let at = |seconds| Instant::ZERO + Duration::from_secs(seconds);
    for (step, request) in requests.iter().enumerate() {
        let service = if request.property {
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY
        } else {
            ConfirmedServiceChoice::SUBSCRIBE_COV
        };
        let now = at(request.at);
        let Some(answer) = device.request(request.peer, service, request.body, now, &mut summary)
        else {
            summary.errors += 1;
            return summary;
        };
        summary.answers[step] = answer;
        summary.held[step] = device.reporter.table().len();
    }
    let points = (
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2),
        ObjectIdentifier::new(ObjectType::BINARY_VALUE, 2),
    );
    let (Ok(ai2), Ok(bv2)) = points else {
        summary.errors += 1;
        return summary;
    };
    device.control(ai2, bv2, &mut summary);
    let write = ConfirmedServiceChoice::WRITE_PROPERTY;
    let read = ConfirmedServiceChoice::READ_PROPERTY;
    let replies = (
        device.request(9, write, CHANGE, at(CHANGE_AT), &mut summary),
        device.request(9, read, READ_LIST, at(CHANGE_AT + 1), &mut summary),
    );
    let (Some(change), Some(list)) = replies else {
        summary.errors += 1;
        return summary;
    };
    (summary.change, summary.list) = (change, list);
    // Out of service, AI-2 keeps the value a client gives it: the
    // application's next reading is refused, and reports nothing.
    let reading = LocalValue::Real(READINGS[0]);
    let refused = device
        .reporter
        .update(&mut device.db, ai2, reading, at(CHANGE_AT + 1));
    summary.refused = local(refused);
    device.send(at(CHANGE_AT + 1), &mut summary, false);
    let synced = device.unconfirmed(SYNC, at(SYNC_AT));
    let read_at = at(SYNC_AT) + Duration::from_millis(1_500);
    let reads = READ_CLOCK.map(|body| device.request(9, read, body, read_at, &mut summary));
    let [Some(date), Some(time)] = reads else {
        summary.errors += 1;
        return summary;
    };
    summary.clock = (synced, [date, time]);
    // The host's own change of zone: daylight saving begins.
    let summer = Zone::new(ZONE, true);
    if summer
        .and_then(|zone| device.reporter.set_zone(&mut device.db, zone))
        .is_err()
    {
        summary.errors += 1;
    }
    summary.window = device
        .reporter
        .clock()
        .and_then(|clock| clock.clock().window(read_at));
    // A COV-multiple context with a timestamped reference, its initial
    // report, and a change it reports with the time it happened.
    #[cfg(feature = "reporting-multiple")]
    {
        let subscribe = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
        let replies = [
            device.request(7, subscribe, MULTIPLE, at(MULTIPLE_AT), &mut summary),
            device.request(9, write, MULTIPLE_CHANGE, at(MULTIPLE_AT + 5), &mut summary),
        ];
        let [Some(subscribed), Some(changed)] = replies else {
            summary.errors += 1;
            return summary;
        };
        summary.multiple = [subscribed, changed];
    }
    device.until(at(EXPIRY_AT), &mut summary);
    summary.left = device.reporter.table().len();
    let counters = device.reporter.counters();
    let cov = &counters.cov;
    summary.counts = [
        cov.subscriptions_created,
        cov.subscriptions_cancelled,
        cov.subscriptions_purged,
        cov.subscriptions_rejected_capacity,
        cov.notifications_sent,
        cov.notifications_confirmed,
        cov.notifications_unconfirmed,
        counters.resent,
        counters.failed,
        counters.owed,
    ];
    summary
}

/// The summary of a good run, which the host test compares with
/// [`run`]'s. Only host builds have it: the images compare nothing.
#[cfg(not(target_os = "none"))]
pub fn expected() -> Summary {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    // SimpleACK (0x20), the invoke ID, then SubscribeCOV (5) or
    // SubscribeCOVProperty (28).
    let ack = |invoke_id: u8, service: u8| alloc::vec![0x20, invoke_id, service];
    // An UnconfirmedCOVNotification (0x10, service 2) of AI-2 to process
    // 10, written by hand from the production: `[0]` the process, `[1]`
    // the Device, `[2]` AI-2, `[3]` the seconds `left` of its lifetime, and
    // in `[4]` Present_Value (85), the REAL `value`, then Status_Flags
    // (111), whose bit string's one octet is `flags`.
    let notified = |left: u8, value: f32, flags: u8| {
        let mut apdu = alloc::vec![
            0x10, 0x02, 0x09, 0x0A, 0x1C, 0x02, 0x00, 0x04, 0xD2, 0x2C, 0x00, 0x00, 0x00, 0x02,
            0x39, left, 0x4E, 0x09, 0x55, 0x2E, 0x44,
        ];
        apdu.extend_from_slice(&value.to_be_bytes());
        apdu.extend_from_slice(&[0x2F, 0x09, 0x6F, 0x2E, 0x82, 0x04, flags, 0x2F, 0x4F]);
        apdu
    };
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
        sent: [
            // AI-1's first notification at 0 s; BV-1's, confirmed, under
            // invoke ID 0, then again at 3, 6 and 9 s, unanswered.
            alloc::vec![(7, None, 34)],
            alloc::vec![(7, Some(0), 33); 4],
            // The renewal's, confirmed, at 10 s, acknowledged: two octets
            // more of header and one of Time_Remaining (300 s).
            alloc::vec![(7, Some(1), 37)],
            // The six subscriptions of AI-2 at 10 s; then, from 21 s, each
            // hearing of the four readings and of the fault, which change
            // none of the notification's lengths; then each hearing of its
            // Status_Flags at 25 s.
            alloc::vec![(8, None, 34); 6],
            alloc::vec![(8, None, 34); 6 * (CYCLES + 1)],
            alloc::vec![(8, None, 34); 6],
            // The COV-multiple context's initial report at 45 s, and
            // AI-1's Status_Flags at 50 s: 58 octets each, with the
            // envelope's timestamp, Present_Value and Status_Flags, each
            // with its Time_Of_Change.
            #[cfg(feature = "reporting-multiple")]
            alloc::vec![(7, None, 58); 2],
        ]
        .concat(),
        // Each reading written and reported, the first notification to
        // process 10, at 19 s to 16 s left. BV-2's four steps each write,
        // and Present_Value goes as COMMANDS has it: ACTIVE under 8 and
        // over 12, INACTIVE once 12 holds alone, ACTIVE again by default.
        control: core::array::from_fn(|n| Cycle {
            update: (Ok(Applied::Written), notified(19 - n as u8, READINGS[n], 0)),
            commands: [
                (Ok(Applied::Written), Some(BinaryPV::ACTIVE)),
                (Ok(Applied::Written), Some(BinaryPV::ACTIVE)),
                (Ok(Applied::Written), Some(BinaryPV::INACTIVE)),
                (Ok(Applied::Written), Some(BinaryPV::ACTIVE)),
            ],
        }),
        // The fault written, and reported at 24 s with the last reading:
        // Status_Flags with FAULT, the second bit (0x40).
        fault: (Ok(Applied::Written), notified(16, READINGS[3], 0x40)),
        // SimpleACK, invoke ID 13, WriteProperty (15).
        change: ack(13, 15),
        list: {
            // ComplexACK (0x30), invoke ID 14, ReadProperty (12): the
            // Device, Active_COV_Subscriptions, then the list in [3].
            let mut list = alloc::vec![
                0x30, 0x0E, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x98, 0x3E,
            ];
            for process in 10..=15 {
                // Station 8 on the local network (network 0), the process,
                // AI-2's Present_Value, unconfirmed, 14 s left, and
                // AI-2's COV_Increment, 0.0.
                list.extend_from_slice(&[
                    0x0E, 0x0E, 0x1E, 0x21, 0x00, 0x61, 0x08, 0x1F, 0x0F, 0x19, process, 0x0F,
                    0x1E, 0x0C, 0x00, 0x00, 0x00, 0x02, 0x19, 0x55, 0x1F, 0x29, 0x00, 0x39, 0x0E,
                    0x4C, 0x00, 0x00, 0x00, 0x00,
                ]);
            }
            list.push(0x3F);
            list
        },
        // Out of service, AI-2 takes no reading from the application:
        // PROPERTY, WRITE_ACCESS_DENIED.
        refused: Err(Some(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::WRITE_ACCESS_DENIED,
        })),
        // The synchronization answers nothing. ComplexACKs, invoke IDs 15
        // and 16, ReadProperty: the Device, the property, then in [3] the
        // Date, 2026-11-04 (a Wednesday), and the Time 1.5 s after 15:30
        // UTC, less UTC_Offset's five hours: 10:30:01.50.
        clock: (
            Outcome::Silent,
            [
                alloc::vec![
                    0x30, 0x0F, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x38, 0x3E, 0xA4, 0x7E,
                    0x0B, 0x04, 0x03, 0x3F,
                ],
                alloc::vec![
                    0x30, 0x10, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x39, 0x3E, 0xB4, 0x0A,
                    0x1E, 0x01, 0x32, 0x3F,
                ],
            ],
        ),
        // In daylight-saving time the same instant is 11:30:01.50, and the
        // clock is still synchronized.
        window: Some((
            DaysOfWeek::WEDNESDAY,
            Time {
                hour: 11,
                minute: 30,
                second: 1,
                hundredths: 50,
            },
        )),
        // SimpleACKs: invoke ID 17, SubscribeCOVPropertyMultiple (30), and
        // invoke ID 18, WriteProperty.
        #[cfg(feature = "reporting-multiple")]
        multiple: [ack(17, 30), ack(18, 15)],
        // The six subscriptions of AI-2 lapsed at 40 s, and with
        // `reporting-multiple` the COV-multiple reference at 75 s.
        left: 0,
        // Created 8, cancelled 2, purged 6, refused 1; sent 45 (2
        // confirmed, 43 unconfirmed, 30 of them the control loop's),
        // resent 3, failed 1, owed none.
        #[cfg(not(feature = "reporting-multiple"))]
        counts: [8, 2, 6, 1, 45, 2, 43, 3, 1, 0],
        // With the COV-multiple context: one more created and purged, and
        // two more unconfirmed notifications.
        #[cfg(feature = "reporting-multiple")]
        counts: [9, 2, 7, 1, 47, 2, 45, 3, 1, 0],
        errors: 0,
    }
}

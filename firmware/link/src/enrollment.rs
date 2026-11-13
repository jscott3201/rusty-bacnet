//! The enrollment image's scenario (#2319).
//!
//! [`run`] runs the reporting image's scenario first
//! ([`reporting::run`](crate::reporting::run)), so the image links what a
//! device that reports COV and intrinsic events serves, and its size less
//! the reporting image's is what algorithmic reporting costs. Then it runs a
//! second device as firmware's main loop would, its `Reporter` sweeping and
//! running the Event Enrollment pass once a second: EE-1 watches AI-7's
//! Present_Value with OUT_OF_RANGE over 80 (low limit 0, deadband 1) after
//! a Time_Delay of two seconds, and with FAULT_OUT_OF_RANGE outside -40 to
//! 120, through Notification Class 1, whose one recipient, station 9, takes
//! every transition unconfirmed and whose TO_OFFNORMAL needs
//! acknowledging. The device has no clock, so each transition is stamped
//! with the next event sequence number.
//!
//! The application's readings of AI-7 ([`READINGS`], with
//! `Reporter::update`) take it over the limit at 3 s, which the pass at 4 s
//! sees and the one at 6 s, two passes on, reports; the application
//! acknowledges it locally at 8 s (`Reporter::acknowledge_local`); back in
//! range at 10 s, it returns to NORMAL at 13 s, after its delay; at 15 s a
//! reading past 120 faults it at the pass at 16 s; and at 18 s a normal
//! reading clears the fault at 19 s. Every notification goes out through the
//! reporter's outbox, and the scenario keeps each one sent.
//!
//! The work lives on the heap, in an arena of [`ARENA`] octets.
//! `tests/enrollment.rs` runs the scenario natively and compares the whole
//! [`Summary`] with `expected()`, then runs it again in the image's
//! allocator over that arena. The `enrollment` image
//! (`src/bin/enrollment.rs`) links it.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use bacnet_device_core::cov::{CovPolicy, Delivery};
use bacnet_device_core::report::{
    self, Bindings, Destination, InvokeIds, LocalValue, OutboxLimits, ReportConfig, Reporter,
};
use bacnet_device_core::responder::{Limits, Responder};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_time::{Duration, Instant};
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAddress, BACnetDestination, BACnetDeviceObjectPropertyReference, BACnetEventParameter,
    BACnetRecipient, FaultParameters,
};
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, EventState, EventType, ObjectType,
    PropertyIdentifier as P,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

use crate::reporting::{self, STEPS};

/// The heap the scenario runs in, in octets: the reporting image's
/// ([`reporting::ARENA`]), which its scenario needs first, the second
/// device and its reporter being smaller (`tests/enrollment.rs` prints the
/// peak).
pub const ARENA: usize = reporting::ARENA;

/// The application's readings of AI-7: at each second, the value.
pub const READINGS: [(u64, f32); 4] = [(3, 90.0), (10, 50.0), (15, 130.0), (18, 50.0)];

/// When the application acknowledges EE-1's transition to HIGH_LIMIT, in
/// seconds.
pub const ACKNOWLEDGE_AT: u64 = 8;

/// When the scenario ends, in seconds.
pub const END_AT: u64 = 20;

/// One event notification sent: `(second, MS/TP station, APDU)`.
pub type Sent = (u64, u8, Vec<u8>);

/// What the scenario saw.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The reporting image's scenario, run first.
    pub reporting: reporting::Summary,
    /// Every event notification the second device sent, in order.
    pub sent: Vec<Sent>,
    /// EE-1's Event_State, raw, as each of [`READINGS`] arrives and at
    /// [`END_AT`]: NORMAL, HIGH_LIMIT, NORMAL, FAULT and NORMAL.
    pub states: Vec<u32>,
    /// EE-1's Reliability at the end.
    pub reliability: u32,
    /// Whether `acknowledge_local` took the acknowledgement.
    pub acknowledged: bool,
    /// The reporter's event notifications handed out, dropped and
    /// suppressed.
    pub counts: [u64; 3],
    /// Calls that failed where the scenario expects success. Zero in a good
    /// run.
    pub errors: u8,
}

/// Invoke IDs: none, as every notification here is unconfirmed.
struct NoIds;

impl InvokeIds for NoIds {
    fn take(&mut self, _: &BACnetAddress, _: ConfirmedServiceChoice) -> Option<u8> {
        None
    }
    fn release(&mut self, _: &BACnetAddress, _: u8) {}
}

/// No Device recipients: the class names its recipient's address.
struct NoBindings;

impl Bindings for NoBindings {
    fn address_of(&self, _: ObjectIdentifier) -> Option<BACnetAddress> {
        None
    }
}

fn ai7() -> Result<ObjectIdentifier, Error> {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7)
}

fn ee1() -> Result<ObjectIdentifier, Error> {
    ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1)
}

/// The second device's database: the Device, AI-7 at 20, EE-1 and
/// Notification Class 1.
fn database() -> Result<ObjectDatabase, Error> {
    use crate::objects::{DEVICE, VENDOR};
    let services: Vec<_> = report::SERVICES
        .iter()
        .chain(report::EVENT_SERVICES)
        .copied()
        .collect();
    let identity = DeviceIdentity::new(DEVICE, VENDOR)?.with_services(&services);
    let mut input = AnalogInputObject::new(7, "AI-7", EngineeringUnits::DEGREES_CELSIUS)?;
    input.set_present_value(20.0);
    let mut enrollment = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE)?;
    let reference =
        BACnetDeviceObjectPropertyReference::new_local(ai7()?, P::PRESENT_VALUE.to_raw());
    enrollment.set_object_property_reference(Some(reference))?;
    enrollment.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 2,
        low_limit: 0.0,
        high_limit: 80.0,
        deadband: 1.0,
    });
    enrollment.set_fault_parameters(Some(FaultParameters::FaultOutOfRange {
        min_normal: -40.0,
        max_normal: 120.0,
    }));
    enrollment.set_event_enable(EventTransitionBits::all());
    enrollment.set_notification_class(1);
    let mut class = NotificationClass::new(1, "NC-1")?;
    class.ack_required = EventTransitionBits::TO_OFFNORMAL;
    let time = |hour, minute, second, hundredths| Time {
        hour,
        minute,
        second,
        hundredths,
    };
    class.add_destination(BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: time(0, 0, 0, 0),
        to_time: time(23, 59, 59, 99),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[9]),
        }),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    })?;
    let extra: Vec<Box<dyn BACnetObject>> =
        vec![Box::new(input), Box::new(enrollment), Box::new(class)];
    build_database_with_extra(&identity, extra)
}

/// The second device's main loop around its reporter.
struct Device {
    reporter: Reporter,
    db: ObjectDatabase,
}

impl Device {
    /// Run every deadline up to `now`, sending what each calls for.
    fn until(&mut self, now: Instant, summary: &mut Summary) {
        while let Some(due) = self.reporter.next_deadline().filter(|due| *due <= now) {
            self.reporter.on_deadline(&mut self.db, &mut NoIds, due);
            self.send(due, summary);
        }
    }

    /// Send every notification the outbox hands out at `now`, each
    /// reported sent.
    fn send(&mut self, now: Instant, summary: &mut Summary) {
        let mut out = BytesMut::new();
        while let Some(sent) = self.reporter.pop(&NoBindings, &mut NoIds, now, &mut out) {
            if let Destination::Recipient(to) = &sent.to {
                let station = to.mac_address.first().copied().unwrap_or_default();
                summary.sent.push((now.as_secs(), station, out.to_vec()));
            } else {
                summary.errors += 1;
            }
            out.clear();
            self.reporter
                .delivered(sent.token, Delivery::Sent, &mut NoIds, now);
        }
    }

    /// `oid`'s enumerated `property`.
    fn enumerated(&self, oid: ObjectIdentifier, property: P) -> Option<u32> {
        match self.db.get(&oid)?.read_property(property, None) {
            Ok(PropertyValue::Enumerated(value)) => Some(value),
            _ => None,
        }
    }
}

/// Run the whole scenario: the reporting image's on `apdus` and
/// `requests`, then the Event Enrollment device's, all passed in so an
/// image can hide them from the optimizer.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and stops.
pub fn run(
    apdus: &[&[u8]; crate::objects::REQUESTS],
    requests: &[reporting::Request; STEPS],
) -> Summary {
    let mut summary = Summary {
        reporting: reporting::run(apdus, requests),
        sent: Vec::new(),
        states: Vec::new(),
        reliability: 0,
        acknowledged: false,
        counts: [0; 3],
        errors: 0,
    };
    if scenario(&mut summary).is_err() {
        summary.errors += 1;
    }
    summary
}

/// The Event Enrollment device's part of [`run`].
fn scenario(summary: &mut Summary) -> Result<(), Error> {
    let (ai7, ee1) = (ai7()?, ee1()?);
    let identity = DeviceIdentity::new(crate::objects::DEVICE, crate::objects::VENDOR)?;
    let responder = Responder::new(identity, Limits::default())?;
    let second = Duration::from_secs(1);
    let config = ReportConfig::new(CovPolicy::default())
        .with_outbox(OutboxLimits {
            entries: 2,
            octets: 256,
        })
        .with_sweep(second)
        .with_event_enrollment(second);
    let mut device = Device {
        reporter: Reporter::new(responder, config)?,
        db: database()?,
    };
    let at = Instant::from_secs;
    for (second, reading) in READINGS {
        if second > ACKNOWLEDGE_AT && !summary.acknowledged {
            device.until(at(ACKNOWLEDGE_AT), summary);
            let state = EventState::HIGH_LIMIT;
            let local =
                device
                    .reporter
                    .acknowledge_local(&mut device.db, ee1, state, at(ACKNOWLEDGE_AT));
            summary.acknowledged = local.is_ok();
            device.send(at(ACKNOWLEDGE_AT), summary);
        }
        device.until(at(second), summary);
        summary
            .states
            .extend(device.enumerated(ee1, P::EVENT_STATE));
        let value = LocalValue::Real(reading);
        if device
            .reporter
            .update(&mut device.db, ai7, value, at(second))
            .is_err()
        {
            summary.errors += 1;
        }
        device.send(at(second), summary);
    }
    device.until(at(END_AT), summary);
    summary
        .states
        .extend(device.enumerated(ee1, P::EVENT_STATE));
    summary.reliability = device.enumerated(ee1, P::RELIABILITY).unwrap_or(u32::MAX);
    let counters = device.reporter.counters();
    summary.counts = [
        counters.events,
        counters.events_dropped,
        counters.enrollments_suppressed,
    ];
    Ok(())
}

/// What a good run sees, octets from the production checked field by field
/// when they were taken.
#[cfg(not(target_os = "none"))]
pub fn expected() -> Summary {
    // UnconfirmedEventNotification (0x10, service 3): `[0]` process 1, `[1]`
    // the Device (1234), `[2]` EE-1, `[3]` the event sequence number
    // `sequence` (no clock), `[4]` class 1, `[5]` priority 255, `[6]` the
    // event type: the head every notification shares. An enrollment
    // commits an empty message text, so `[7]` is left out.
    let head = |sequence: u8, event_type: u8| {
        vec![
            0x10, 0x03, 0x09, 0x01, 0x1C, 0x02, 0x00, 0x04, 0xD2, 0x2C, 0x02, 0x40, 0x00, 0x01,
            0x3E, 0x19, sequence, 0x3F, 0x49, 0x01, 0x59, 0xFF, 0x69, event_type,
        ]
    };
    // OUT_OF_RANGE (5)'s Event Values: AI-7's `value`, its Status_Flags
    // (none set), deadband 1 and the limit 80.
    let out_of_range = |value: [u8; 4]| {
        let mut values = vec![0xCE, 0x5E, 0x0C];
        values.extend_from_slice(&value);
        values.extend_from_slice(&[0x1A, 0x04, 0x00, 0x2C, 0x3F, 0x80, 0x00, 0x00]);
        values.extend_from_slice(&[0x3C, 0x42, 0xA0, 0x00, 0x00, 0x5F, 0xCF]);
        values
    };
    // At 6 s: `[8]` ALARM (0), `[9]` Ack_Required TRUE, `[10]` from NORMAL,
    // `[11]` to HIGH_LIMIT (3), and the Event Values at 90.
    let mut high = head(0, 0x05);
    high.extend_from_slice(&[0x89, 0x00, 0x99, 0x01, 0xA9, 0x00, 0xB9, 0x03]);
    high.extend(out_of_range([0x42, 0xB4, 0x00, 0x00]));
    // At 8 s, the local acknowledgement's: `[8]` ACK_NOTIFICATION (2),
    // `[11]` HIGH_LIMIT, stamped with the next sequence number.
    let mut acknowledged = head(1, 0x05);
    acknowledged.extend_from_slice(&[0x89, 0x02, 0xB9, 0x03]);
    // At 13 s: from HIGH_LIMIT to NORMAL, no acknowledgement needed, at 50.
    let mut normal = head(2, 0x05);
    normal.extend_from_slice(&[0x89, 0x00, 0x99, 0x00, 0xA9, 0x03, 0xB9, 0x00]);
    normal.extend(out_of_range([0x42, 0x48, 0x00, 0x00]));
    // CHANGE_OF_RELIABILITY (19)'s Event Values, `[19]` in an extended
    // tag: `[0]` the Reliability, `[1]` EE-1's Status_Flags, `[2]` the
    // monitored values: Object_Property_Reference (AI-7's Present_Value),
    // AI-7's Present_Value `value`, its Reliability (NO_FAULT_DETECTED)
    // and its Status_Flags (none set).
    let reliability = |reliability: u8, flags: u8, value: [u8; 4]| {
        let mut values = vec![0xCE, 0xFE, 0x13, 0x09, reliability, 0x1A, 0x04, flags];
        values.extend_from_slice(&[0x2E, 0x09, 0x4E, 0x2E, 0x0C, 0x00, 0x00, 0x00, 0x07]);
        values.extend_from_slice(&[0x19, 0x55, 0x2F, 0x09, 0x55, 0x2E, 0x44]);
        values.extend_from_slice(&value);
        values.extend_from_slice(&[0x2F, 0x09, 0x67, 0x2E, 0x91, 0x00, 0x2F]);
        values.extend_from_slice(&[0x09, 0x6F, 0x2E, 0x82, 0x04, 0x00, 0x2F, 0x2F]);
        values.extend_from_slice(&[0xFF, 0x13, 0xCF]);
        values
    };
    // At 16 s, the fault: from NORMAL to FAULT (1), OVER_RANGE (2), EE-1
    // IN_ALARM and FAULT, at 130.
    let mut fault = head(3, 0x13);
    fault.extend_from_slice(&[0x89, 0x00, 0x99, 0x00, 0xA9, 0x00, 0xB9, 0x01]);
    fault.extend(reliability(0x02, 0xC0, [0x43, 0x02, 0x00, 0x00]));
    // At 19 s, the fault clears: from FAULT to NORMAL, NO_FAULT_DETECTED,
    // no flags, at 50.
    let mut cleared = head(4, 0x13);
    cleared.extend_from_slice(&[0x89, 0x00, 0x99, 0x00, 0xA9, 0x01, 0xB9, 0x00]);
    cleared.extend(reliability(0x00, 0x00, [0x42, 0x48, 0x00, 0x00]));
    let (normal_state, high_limit, fault_state) = (0, 3, 1);
    Summary {
        reporting: reporting::expected(),
        sent: vec![
            (6, 9, high),
            (8, 9, acknowledged),
            (13, 9, normal),
            (16, 9, fault),
            (19, 9, cleared),
        ],
        states: vec![
            normal_state,
            high_limit,
            normal_state,
            fault_state,
            normal_state,
        ],
        reliability: 0,
        acknowledged: true,
        counts: [5, 0, 0],
        errors: 0,
    }
}

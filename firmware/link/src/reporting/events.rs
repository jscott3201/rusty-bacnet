//! The scenario's intrinsic events (#2303): once the Device clock is set,
//! the application's reading takes AI-3 over its high limit at
//! [`EVENT_AT`], so after its one-second Time_Delay the sweep
//! sends the TO_OFFNORMAL notification to Notification Class 1's one
//! recipient, station 9. A GetEventInformation from station 9 lists it, and
//! the application acknowledges it locally (`Reporter::acknowledge_local`),
//! which sends the acknowledgement notification. Each notification's APDU
//! is kept ([`Events::sent`]), with the GetEventInformation reply.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_time::{Duration, Instant};
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::enums::{ConfirmedServiceChoice, EngineeringUnits, EventState, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};

use super::{Device, LocalValue, Summary};

/// When AI-3 goes over its limit, in seconds: after the clock's reads
/// ([`SYNC_AT`](super::SYNC_AT)), before the COV-multiple context.
pub const EVENT_AT: u64 = 30;

/// What the event scenario sent and was answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Events {
    /// Each event notification's APDU, in order sent.
    pub sent: Vec<Vec<u8>>,
    /// The reply to the GetEventInformation.
    pub information: Vec<u8>,
    /// Whether `acknowledge_local` took the acknowledgement.
    pub acknowledged: bool,
}

/// AI-3, reporting OUT_OF_RANGE over 80 after a second through class 1;
/// class 1, whose TO_OFFNORMAL needs acknowledging, with station 9 as its
/// one recipient, unconfirmed, every day, all day; and class 0.
pub(super) fn objects(extra: &mut Vec<Box<dyn BACnetObject>>) -> Result<(), Error> {
    let mut input = AnalogInputObject::new(3, "AI-3", EngineeringUnits::NO_UNITS)?;
    for (property, real) in [
        (P::HIGH_LIMIT, 80.0f32),
        (P::LOW_LIMIT, 0.0),
        (P::DEADBAND, 1.0),
    ] {
        input.write_property(property, None, PropertyValue::Real(real), None)?;
    }
    for (property, data) in [(P::LIMIT_ENABLE, (6, 0xC0)), (P::EVENT_ENABLE, (5, 0xE0))] {
        let (unused_bits, bits) = data;
        let value = PropertyValue::BitString {
            unused_bits,
            data: vec![bits],
        };
        input.write_property(property, None, value, None)?;
    }
    for (property, unsigned) in [(P::TIME_DELAY, 1), (P::NOTIFICATION_CLASS, 1)] {
        input.write_property(property, None, PropertyValue::Unsigned(unsigned), None)?;
    }
    input.set_present_value(10.0);
    let mut class = NotificationClass::new(1, "NC-1")?;
    class.ack_required = EventTransitionBits::TO_OFFNORMAL;
    let midnight = |hour, minute, second, hundredths| Time {
        hour,
        minute,
        second,
        hundredths,
    };
    class.add_destination(BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: midnight(0, 0, 0, 0),
        to_time: midnight(23, 59, 59, 99),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[9]),
        }),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    })?;
    extra.push(Box::new(input));
    extra.push(Box::new(class));
    // Class 0, with no recipient, for AI-1 and AI-2, which name it: AI-2's
    // fault is a FAULT transition that goes nowhere, and GetEventInformation
    // can project its summary.
    extra.push(Box::new(NotificationClass::new(0, "NC-0")?));
    Ok(())
}

/// Run the event scenario on `device`, keeping what it sent in `summary`.
pub(super) fn run(device: &mut Device, summary: &mut Summary) {
    let Ok(ai3) = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3) else {
        summary.errors += 1;
        return;
    };
    let at = |millis| Instant::from_secs(EVENT_AT) + Duration::from_millis(millis);
    device.until(at(0), summary);
    device.tick(at(0));
    let reading = LocalValue::Real(90.0);
    if device
        .reporter
        .update(&mut device.db, ai3, reading, at(0))
        .is_err()
    {
        summary.errors += 1;
    }
    device.send(at(0), summary, false);
    // The Time_Delay ends: the sweep commits and sends the transition.
    device.tick(at(1_000));
    device.until(at(1_000), summary);
    let service = ConfirmedServiceChoice::GET_EVENT_INFORMATION;
    match device.request(9, service, &[], at(2_000), summary) {
        Some(reply) => summary.events.information = reply,
        None => summary.errors += 1,
    }
    device.tick(at(3_000));
    let state = EventState::HIGH_LIMIT;
    let acknowledged = device
        .reporter
        .acknowledge_local(&mut device.db, ai3, state, at(3_000));
    summary.events.acknowledged = acknowledged.is_ok();
    device.send(at(3_000), summary, false);
}

/// What a good run's event scenario sends and is answered, octets from the
/// production checked field by field when they were taken.
#[cfg(not(target_os = "none"))]
pub(super) fn expected() -> Events {
    // UnconfirmedEventNotification (0x10, service 3): `[0]` process 1,
    // `[1]` the Device (1234), `[2]` AI-3, `[3]` the Device clock's date
    // and time (2026-11-04, a Wednesday, 11:31 and the second), `[4]` class
    // 1, `[5]` priority 255, `[6]` OUT_OF_RANGE (5): the head both
    // notifications share but for the stamp's `second`.
    let head = |second: u8| {
        let mut head = vec![
            0x10, 0x03, 0x09, 0x01, 0x1C, 0x02, 0x00, 0x04, 0xD2, 0x2C, 0x00, 0x00, 0x00, 0x03,
            0x3E, 0x2E, 0xA4, 0x7E, 0x0B, 0x04, 0x03, 0xB4, 0x0B, 0x1E,
        ];
        head.extend_from_slice(&[second, 0x00, 0x2F, 0x3F, 0x49, 0x01, 0x59, 0xFF]);
        head.extend_from_slice(&[0x69, 0x05]);
        head
    };
    // The transition, stamped at 11:30:04, 31 s: `[7]` the message text, `[8]`
    // ALARM (0), `[9]` Ack_Required TRUE, `[10]` from NORMAL, `[11]` to
    // HIGH_LIMIT (3), `[12]` its Event Values: 90 over the limit 80,
    // Status_Flags IN_ALARM, deadband 1.
    let mut transition = head(0x04);
    transition.extend_from_slice(&[0x7D, 0x25, 0x00]);
    transition.extend_from_slice(b"ANALOG_INPUT,3: NORMAL -> HIGH_LIMIT");
    transition.extend_from_slice(&[
        0x89, 0x00, 0x99, 0x01, 0xA9, 0x00, 0xB9, 0x03, 0xCE, 0x5E, 0x0C, 0x42, 0xB4, 0x00, 0x00,
        0x1A, 0x04, 0x80, 0x2C, 0x3F, 0x80, 0x00, 0x00, 0x3C, 0x42, 0xA0, 0x00, 0x00, 0x5F, 0xCF,
    ]);
    // The acknowledgement, stamped when it ran, 11:30:06, 33 s: `[8]`
    // ACK_NOTIFICATION (2), `[11]` to HIGH_LIMIT.
    let mut acknowledgement = head(0x06);
    acknowledgement.extend_from_slice(&[0x89, 0x02, 0xB9, 0x03]);
    Events {
        sent: vec![transition, acknowledgement],
        // The ComplexACK (0x30, invoke ID 17, service 29): AI-2 in FAULT,
        // AI-3 at HIGH_LIMIT with TO_OFFNORMAL unacknowledged when it was
        // asked, BV-1 and BV-2 OFFNORMAL from their ACTIVE values; no more
        // events.
        information: vec![
            48, 17, 29, 14, 12, 0, 0, 0, 2, 25, 1, 42, 5, 224, 62, 25, 0, 46, 164, 90, 1, 1, 1,
            180, 0, 0, 24, 0, 47, 25, 0, 63, 73, 0, 90, 5, 0, 110, 33, 255, 33, 255, 33, 255, 111,
            12, 0, 0, 0, 3, 25, 3, 42, 5, 96, 62, 46, 164, 126, 11, 4, 3, 180, 11, 30, 4, 0, 47,
            25, 0, 25, 0, 63, 73, 0, 90, 5, 224, 110, 33, 255, 33, 255, 33, 255, 111, 12, 1, 64, 0,
            1, 25, 2, 42, 5, 224, 62, 46, 164, 90, 1, 1, 1, 180, 0, 0, 0, 0, 47, 25, 0, 25, 0, 63,
            73, 0, 90, 5, 0, 110, 33, 255, 33, 255, 33, 255, 111, 12, 1, 64, 0, 2, 25, 2, 42, 5,
            224, 62, 46, 164, 90, 1, 1, 1, 180, 0, 0, 24, 0, 47, 25, 0, 46, 164, 90, 1, 1, 1, 180,
            0, 0, 24, 0, 47, 63, 73, 0, 90, 5, 0, 110, 33, 255, 33, 255, 33, 255, 111, 15, 25, 0,
        ],
        acknowledged: true,
    }
}

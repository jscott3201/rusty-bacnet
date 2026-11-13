//! The `Reporter`'s event path (#2303): when it sweeps, how a confirmed
//! event notification waits for an invoke ID, how an event evicts COV, and
//! where an event notification goes.

use super::*;
use crate::report::LocalValue;
use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::enums::UnconfirmedServiceChoice;
use bacnet_types::primitives::Time;

#[cfg(feature = "event-enrollment")]
#[path = "reporter_enrollment_tests.rs"]
mod enrollment;
#[path = "reporter_event_order_tests.rs"]
mod order;
#[path = "reporter_tail_tests.rs"]
mod tail;

/// The intrinsic Analog Input: high limit 80, low limit 0, deadband 1,
/// reading 10.
fn alarm() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap()
}

/// The second intrinsic Analog Input, as [`alarm`].
fn alarm2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 4).unwrap()
}

fn recipient_address() -> BACnetAddress {
    BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&[0x0A, 0, 0, 7, 0xBA, 0xC0]),
    }
}

/// A destination open every day, all day, for every transition.
fn destination(recipient: BACnetRecipient, confirmed: bool) -> BACnetDestination {
    BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient,
        process_identifier: 7,
        issue_confirmed_notifications: confirmed,
        transitions: EventTransitionBits::all(),
    }
}

fn intrinsic(instance: u32, time_delay: u64) -> Box<dyn bacnet_objects::traits::BACnetObject> {
    let name = if instance == 3 { "AI-3" } else { "AI-4" };
    let mut object = AnalogInputObject::new(instance, name, EngineeringUnits::NO_UNITS).unwrap();
    for (property, value) in [
        (P::HIGH_LIMIT, 80.0f32),
        (P::LOW_LIMIT, 0.0),
        (P::DEADBAND, 1.0),
    ] {
        object
            .write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    for (property, unused_bits, bits) in [(P::LIMIT_ENABLE, 6, 0xC0), (P::EVENT_ENABLE, 5, 0xE0)] {
        let value = PropertyValue::BitString {
            unused_bits,
            data: vec![bits],
        };
        object.write_property(property, None, value, None).unwrap();
    }
    let delay = PropertyValue::Unsigned(time_delay);
    object
        .write_property(P::TIME_DELAY, None, delay, None)
        .unwrap();
    object.set_present_value(10.0);
    Box::new(object)
}

/// [`Device`] with the event path on, sweeping every `sweep` seconds: two
/// intrinsic inputs with Time_Delay `delay`, reporting through a
/// Notification Class with `destinations`; swept once at `T0`.
fn device(
    outbox: OutboxLimits,
    delay: u64,
    destinations: Vec<BACnetDestination>,
    sweep: u64,
) -> Device {
    device_at(outbox, delay, (destinations, [255; 3]), sweep)
}

/// [`device`], the Notification Class's Priority `priority`.
fn device_at(
    outbox: OutboxLimits,
    delay: u64,
    (destinations, priority): (Vec<BACnetDestination>, [u8; 3]),
    sweep: u64,
) -> Device {
    let identity = DeviceIdentity::new(9, 0x105)
        .unwrap()
        .with_services(SERVICES);
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    class.priority = priority;
    for destination in destinations {
        class.add_destination(destination).unwrap();
    }
    let mut sensor = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    sensor.set_present_value(10.0);
    sensor.set_cov_increment(1.0).unwrap();
    let second = AnalogInputObject::new(2, "AI-2", EngineeringUnits::NO_UNITS).unwrap();
    let objects: Vec<Box<dyn bacnet_objects::traits::BACnetObject>> = vec![
        Box::new(sensor),
        Box::new(second),
        intrinsic(3, delay),
        intrinsic(4, delay),
        Box::new(class),
    ];
    let db = build_database_with_extra(&identity, objects).unwrap();
    let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
    let config = ReportConfig::new(crate::cov::CovPolicy::default())
        .with_outbox(outbox)
        .with_confirmed(FlightTiming::new(Duration::from_secs(3), 1))
        .with_sweep(Duration::from_secs(sweep));
    let mut device = Device {
        reporter: Reporter::new(responder, config).unwrap(),
        db,
        ids: Ids {
            limit: 4,
            ..Ids::default()
        },
        invoke_id: 0,
    };
    // The first sweep runs at the first deadline.
    assert_eq!(device.reporter.next_deadline(), Some(Instant::ZERO));
    device
        .reporter
        .on_deadline(&mut device.db, &mut device.ids, T0);
    device
}

fn address_destination(confirmed: bool) -> BACnetDestination {
    destination(BACnetRecipient::Address(recipient_address()), confirmed)
}

fn ms(millis: u64) -> Instant {
    T0.saturating_add(Duration::from_millis(millis))
}

impl Device {
    /// Raise `object` to 90, over its high limit.
    fn raise(&mut self, object: ObjectIdentifier, now: Instant) {
        let applied = self
            .reporter
            .update(&mut self.db, object, LocalValue::Real(90.0), now);
        assert!(applied.is_ok(), "{applied:?}");
    }

    /// Run every deadline up to `until`; how many runs.
    fn run_until(&mut self, until: Instant) -> Vec<Instant> {
        let mut runs = Vec::new();
        while let Some(at) = self.reporter.next_deadline().filter(|at| *at <= until) {
            self.reporter.on_deadline(&mut self.db, &mut self.ids, at);
            runs.push(at);
        }
        runs
    }

    fn pop_with(&mut self, bindings: &dyn Bindings, now: Instant) -> Option<(Notification, Apdu)> {
        let mut out = BytesMut::new();
        let notification = self.reporter.pop(bindings, &mut self.ids, now, &mut out)?;
        Some((notification, decode_apdu(out.freeze()).unwrap()))
    }
}

/// Whether `apdu` is an event notification, confirmed or not.
fn is_event(apdu: &Apdu) -> bool {
    match apdu {
        Apdu::UnconfirmedRequest(request) => {
            request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
        }
        Apdu::ConfirmedRequest(request) => {
            request.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
        }
        _ => false,
    }
}

#[test]
fn a_write_seeds_a_due_instant_a_long_period_does_not_hide() {
    // A ten-second period, longer than the two-second delay: the reporter
    // takes the due instant from the change, so it sweeps when the delay
    // ends rather than at the next period (decision 20's invariant).
    let mut device = device(
        OutboxLimits::default(),
        2,
        vec![address_destination(false)],
        10,
    );
    assert_eq!(device.reporter.next_deadline(), Some(ms(10_000)));
    device.raise(alarm(), ms(900));
    assert_eq!(device.reporter.next_deadline(), Some(ms(2_900)));
    assert_eq!(device.run_until(ms(2_899)), Vec::<Instant>::new());
    assert!(device.pop(ms(2_899)).is_none(), "not before its delay ends");
    assert_eq!(device.run_until(ms(2_900)), vec![ms(2_900)]);
    let (sent, apdu) = device.pop(ms(2_900)).expect("the transition");
    assert!(is_event(&apdu), "{apdu:?}");
    assert_eq!(sent.service, NotifyService::Event);
    assert_eq!(sent.to, Destination::Recipient(recipient_address()));
    assert_eq!(sent.priority, NetworkPriority::Normal, "Table 13-6 for 255");
    assert_eq!(device.reporter.counters().events, 1);
    // The cadence is the period's again.
    assert_eq!(device.reporter.next_deadline(), Some(ms(10_000)));
}

#[test]
fn due_sweeps_keep_the_floor_after_the_last_sweep() {
    let mut device = device(
        OutboxLimits::default(),
        1,
        vec![address_destination(false)],
        1,
    );
    device.raise(alarm(), T0);
    device.raise(alarm2(), ms(50));
    // Due at 1.0 s and 1.05 s: the second waits for the floor, 100 ms
    // after the sweep the first woke.
    assert_eq!(device.run_until(ms(1_200)), vec![ms(1_000), ms(1_100)]);
    let events = core::iter::from_fn(|| device.pop(ms(1_100))).count();
    assert_eq!(events, 2);
    assert_eq!(device.reporter.next_deadline(), Some(ms(2_000)));
}

#[test]
fn a_confirmed_event_refused_an_id_goes_when_one_comes_back() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    assert!(device.pop(ms(100)).is_none(), "no invoke ID");
    assert!(device.reporter.owes_invoke_id());
    assert_eq!(device.reporter.counters().owed, 1);
    // Passed by while it waits; not looked at again without an ID.
    assert!(device.pop(ms(200)).is_none());
    assert_eq!(
        device.reporter.next_deadline(),
        Some(ms(1_000)),
        "the sweep"
    );
    device.ids.limit = 1;
    device.reporter.invoke_ids_released(1);
    assert_eq!(device.reporter.next_deadline(), Some(ms(200)));
    device.run_until(ms(200));
    let (sent, apdu) = device.pop(ms(200)).expect("the held notification");
    assert!(is_event(&apdu), "{apdu:?}");
    assert_eq!(sent.invoke_id, Some(0));
    assert!(!device.reporter.owes_invoke_id());
    assert_eq!(device.reporter.counters().owed, 1, "counted once");
    assert_eq!(
        device.ids.services.last(),
        Some(&ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION)
    );
}

#[test]
fn an_event_and_a_cov_report_owed_one_id_back_the_event_goes_first() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    device.subscribe_to(alarm(), 1, true, ms(10));
    let (first, _) = device.pop(ms(10)).expect("the initial notification");
    assert!(device.ack(first.invoke_id.unwrap(), ms(20)));
    device.run_until(ms(20));
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    // The event first, then the COV report of its Status_Flags and value.
    assert!(device.pop(ms(100)).is_none());
    assert!(device.reporter.owes_invoke_id());
    assert_eq!(device.reporter.counters().owed, 2);
    device.ids.limit = 1;
    device.reporter.invoke_ids_released(1);
    device.run_until(ms(200));
    let (event, apdu) = device.pop(ms(200)).expect("the event takes the ID");
    assert!(is_event(&apdu), "{apdu:?}");
    assert!(device.pop(ms(200)).is_none(), "one ID, one report");
    assert!(
        device.reporter.owes_invoke_id(),
        "the COV report still waits"
    );
    // The event's answer gives its ID back: the COV report gets its look.
    let mut answer = BytesMut::new();
    let ack = SimpleAck {
        invoke_id: event.invoke_id.unwrap(),
        service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
    };
    encode_apdu(&mut answer, &Apdu::SimpleAck(ack)).unwrap();
    let from = recipient_address();
    assert!(
        device
            .reporter
            .reply(&from, &answer, &mut device.ids, ms(300))
    );
    device.run_until(ms(300));
    let (cov, apdu) = device.pop(ms(300)).expect("the COV report");
    assert_eq!(cov.service, NotifyService::Cov);
    assert_eq!(Device::reported_pv(&apdu), PropertyValue::Real(90.0));
    assert!(!device.reporter.owes_invoke_id());
}

#[test]
fn an_event_evicts_a_report_a_later_change_relied_on() {
    let outbox = OutboxLimits {
        entries: 2,
        octets: 2048,
    };
    let mut device = device(outbox, 0, vec![address_destination(false)], 1);
    // AI-2's initial notification, then AI-1's, fill the outbox.
    device.subscribe_to(ai2(), 2, false, ms(10));
    device.subscribe_to(ai(), 1, false, ms(10));
    // Under COV_Increment from AI-1's queued 10: no report, relying on it.
    let applied = device
        .reporter
        .update(&mut device.db, ai(), LocalValue::Real(10.5), ms(20));
    assert!(applied.is_ok());
    device.raise(alarm(), ms(30));
    assert_eq!(device.reporter.counters().evicted, 1);
    let (second, _) = device.pop(ms(30)).expect("AI-2's, the oldest");
    assert_eq!(second.service, NotifyService::Cov);
    device
        .reporter
        .delivered(second.token, Delivery::Sent, &mut device.ids, ms(30));
    let (event, apdu) = device.pop(ms(30)).expect("the event, in AI-1's place");
    assert!(is_event(&apdu), "{apdu:?}");
    device
        .reporter
        .delivered(event.token, Delivery::Sent, &mut device.ids, ms(30));
    // AI-3's own COV report found no room: it has no subscriber, so none.
    device.run_until(ms(40));
    let (fresh, apdu) = device.pop(ms(40)).expect("AI-1's fresh look");
    assert_eq!(Device::reported_pv(&apdu), PropertyValue::Real(10.5));
    device
        .reporter
        .delivered(fresh.token, Delivery::Sent, &mut device.ids, ms(40));
    device.run_until(ms(50));
    assert!(device.pop(ms(50)).is_none(), "current");
}

#[test]
fn an_event_with_no_cov_left_to_evict_is_dropped_and_counted() {
    let outbox = OutboxLimits {
        entries: 1,
        octets: 2048,
    };
    let two = vec![address_destination(false), address_destination(false)];
    let mut device = device(outbox, 0, two, 1);
    device.raise(alarm(), ms(10));
    let counters = device.reporter.counters();
    assert_eq!((counters.events_dropped, counters.evicted), (1, 0));
    assert!(device.pop(ms(10)).is_some());
    assert!(device.pop(ms(10)).is_none());
}

struct OneBinding;

impl Bindings for OneBinding {
    fn address_of(&self, device: ObjectIdentifier) -> Option<BACnetAddress> {
        (device.instance_number() == 77).then(recipient_address)
    }
}

#[test]
fn a_device_recipient_resolves_through_bindings_or_is_counted() {
    let bound = ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap();
    let unbound = ObjectIdentifier::new(ObjectType::DEVICE, 78).unwrap();
    let destinations = vec![
        destination(BACnetRecipient::Device(unbound), false),
        destination(BACnetRecipient::Device(bound), false),
    ];
    let mut device = device(OutboxLimits::default(), 0, destinations, 1);
    device.raise(alarm(), ms(10));
    let (sent, _) = device.pop_with(&OneBinding, ms(10)).expect("the bound one");
    assert_eq!(sent.to, Destination::Recipient(recipient_address()));
    assert!(device.pop_with(&OneBinding, ms(10)).is_none());
    assert_eq!(device.reporter.counters().unresolved, 1);
}

#[test]
fn without_a_sweep_the_event_path_is_off() {
    // As the endpoint hosts it: no sweep, no AcknowledgeAlarm, no events.
    let mut device = Device::new(OutboxLimits::default());
    assert_eq!(device.reporter.next_deadline(), None);
    let reply = device.request(ConfirmedServiceChoice::GET_EVENT_INFORMATION, &[], T0);
    assert!(matches!(reply, Apdu::Reject(_)), "{reply:?}");
}

#[test]
fn with_the_event_path_on_get_event_information_answers() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(false)],
        1,
    );
    device.raise(alarm(), ms(10));
    let reply = device.request(ConfirmedServiceChoice::GET_EVENT_INFORMATION, &[], ms(20));
    let Apdu::ComplexAck(ack) = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(
        ack.service_choice,
        ConfirmedServiceChoice::GET_EVENT_INFORMATION
    );
    // The object in alarm is listed: its identifier's encoding is there.
    let encoded = alarm().encode();
    assert!(ack.service_ack.windows(4).any(|window| window == encoded));
}

#[test]
fn a_local_acknowledgement_of_the_wrong_state_is_refused() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(false)],
        1,
    );
    device.raise(alarm(), ms(10));
    assert!(device.pop(ms(10)).is_some(), "the transition");
    // The transition it made was to HIGH_LIMIT.
    let low = bacnet_types::enums::EventState::LOW_LIMIT;
    let refused = device
        .reporter
        .acknowledge_local(&mut device.db, alarm(), low, ms(20));
    let error = refused.expect_err("not the state it reached");
    let code = crate::reply::error_fields(&error).1;
    assert_eq!(code, bacnet_types::enums::ErrorCode::INVALID_EVENT_STATE);
    assert!(device.pop(ms(20)).is_none());
}

#[test]
fn a_fault_pass_without_a_sweep_is_refused() {
    let identity = DeviceIdentity::new(9, 0x105).unwrap();
    let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
    let config = ReportConfig::default().with_fault_detection(Duration::from_secs(10));
    assert!(Reporter::new(responder, config).is_err());
}

#[test]
fn a_notification_class_priority_sets_the_network_priority() {
    // TO_OFFNORMAL at event priority 10: life safety, by Table 13-6.
    let destinations = (vec![address_destination(false)], [10, 255, 255]);
    let mut device = device_at(OutboxLimits::default(), 0, destinations, 1);
    device.raise(alarm(), ms(10));
    let (sent, _) = device.pop(ms(10)).expect("the transition");
    assert_eq!(sent.priority, NetworkPriority::LifeSafety);
}

/// An Analog Value with no limits enabled, whose fault limits (-100 to
/// 100) only the fault pass evaluates, reporting through class 0.
fn faulting_value() -> Box<dyn BACnetObject> {
    use bacnet_objects::analog::AnalogValueObject;
    let mut value = AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap();
    value.configure_fault_out_of_range(-100.0, 100.0).unwrap();
    let enable = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![0xE0],
    };
    value
        .write_property(P::EVENT_ENABLE, None, enable, None)
        .unwrap();
    Box::new(value)
}

#[test]
fn the_fault_pass_reports_to_cov_and_the_sweep_sends_the_fault() {
    let identity = DeviceIdentity::new(9, 0x105)
        .unwrap()
        .with_services(SERVICES);
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    class.add_destination(address_destination(false)).unwrap();
    let objects = vec![faulting_value(), Box::new(class) as Box<dyn BACnetObject>];
    let db = build_database_with_extra(&identity, objects).unwrap();
    let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
    let config = ReportConfig::new(crate::cov::CovPolicy::default())
        .with_sweep(Duration::from_secs(1))
        .with_fault_detection(Duration::from_millis(2_500));
    let mut device = Device {
        reporter: Reporter::new(responder, config).unwrap(),
        db,
        ids: Ids::default(),
        invoke_id: 0,
    };
    device
        .reporter
        .on_deadline(&mut device.db, &mut device.ids, T0);
    let av = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    device.subscribe_to(av, 1, false, T0);
    let (initial, _) = device.pop(T0).expect("the initial report");
    device
        .reporter
        .delivered(initial.token, Delivery::Sent, &mut device.ids, T0);
    // The application's sample, out of the fault range: COV now; only the
    // fault pass evaluates the range.
    let sample = LocalValue::Real(150.0);
    let applied = device
        .reporter
        .command(&mut device.db, av, 16, sample, ms(100));
    assert!(applied.is_ok(), "{applied:?}");
    let (change, _) = device.pop(ms(100)).expect("the value's report");
    device
        .reporter
        .delivered(change.token, Delivery::Sent, &mut device.ids, ms(100));
    device.run_until(ms(2_499));
    assert!(device.pop(ms(2_499)).is_none(), "nothing has looked yet");
    // The pass finds the fault: COV at once.
    assert!(device.run_until(ms(2_500)).contains(&ms(2_500)));
    let (cov, _) = device.pop(ms(2_500)).expect("the fault's COV report");
    assert_eq!(cov.service, NotifyService::Cov);
    assert!(device.pop(ms(2_500)).is_none(), "the transition waits");
    // The next sweep turns the Reliability into its FAULT transition.
    device.run_until(ms(3_000));
    let (fault, apdu) = device.pop(ms(3_000)).expect("the FAULT transition");
    assert_eq!(fault.service, NotifyService::Event);
    assert!(is_event(&apdu), "{apdu:?}");
}

#[test]
fn the_shared_cov_deadline_refuses_a_reporter_with_the_event_path() {
    let mut device = device(
        OutboxLimits::default(),
        2,
        vec![address_destination(false)],
        1,
    );
    device.raise(alarm(), ms(100));
    let due = device.reporter.next_deadline();
    let refused = device
        .reporter
        .on_cov_deadline(&device.db, &mut device.ids, ms(1_000));
    assert!(refused.is_err());
    assert_eq!(device.reporter.next_deadline(), due, "nothing ran");
    let mut cov_only = Device::new(OutboxLimits::default());
    let ran = cov_only
        .reporter
        .on_cov_deadline(&cov_only.db, &mut cov_only.ids, T0);
    assert!(ran.is_ok());
}

/// A clock reader at 10:00 on a Monday, which the host lends the database
/// without giving the `Responder` a clock.
struct LentClock;

impl bacnet_objects::clock::ClockReader for LentClock {
    fn read_clock(&self) -> Option<bacnet_objects::clock::ClockFrame> {
        Some(bacnet_objects::clock::ClockFrame {
            local_date: bacnet_types::primitives::Date {
                year: 126,
                month: 11,
                day: 2,
                day_of_week: 1,
            },
            local_time: Time {
                hour: 10,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

/// Without a clock of its own the reporter selects only destinations open
/// every day, all day, though the database's reader gives a valid frame
/// whose window a restricted entry from the build would open on.
#[test]
fn without_a_clock_a_lent_reader_opens_no_window() {
    let mut mornings = address_destination(false);
    mornings.to_time = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    mornings.recipient = BACnetRecipient::Address(BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&[0x0A, 0, 0, 8, 0xBA, 0xC0]),
    });
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(false), mornings],
        1,
    );
    device
        .db
        .set_clock_reader(Some(alloc::sync::Arc::new(LentClock)));
    device.raise(alarm(), ms(10));
    let (sent, _) = device.pop(ms(10)).expect("the destination open all day");
    assert_eq!(sent.to, Destination::Recipient(recipient_address()));
    assert!(device.pop(ms(10)).is_none(), "not the morning one");
}

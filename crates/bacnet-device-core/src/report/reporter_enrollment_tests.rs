//! The `Reporter`'s Event Enrollment pass (#2319): when it runs, what it
//! sends and through which path, and what it leaves alone.

use super::*;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::{EventState, EventType, Reliability};
use bacnet_types::primitives::BACnetTimeStamp;

/// Event Enrollment `instance` watching `property` of AI-1 (10 at first)
/// with OUT_OF_RANGE over 80 after `time_delay` seconds, through
/// Notification Class 0.
fn enrollment(instance: u32, property: P, time_delay: u32) -> EventEnrollmentObject {
    let name = alloc::format!("EE-{instance}");
    let mut ee = EventEnrollmentObject::new(instance, &name, EventType::OUT_OF_RANGE).unwrap();
    let reference = BACnetDeviceObjectPropertyReference::new_local(ai1(), property.to_raw());
    ee.set_object_property_reference(Some(reference)).unwrap();
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay,
        low_limit: 0.0,
        high_limit: 80.0,
        deadband: 1.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    ee
}

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn ee(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, instance).unwrap()
}

/// The event path's [`device`] with `enrollments` too and a pass every
/// `pass`, sweeping every `sweep` seconds; its first deadline run at `T0`.
fn enrolled(
    enrollments: Vec<EventEnrollmentObject>,
    destinations: Vec<BACnetDestination>,
    timing: (u64, Duration),
) -> Device {
    enrolled_with(Vec::new(), enrollments, destinations, timing)
}

/// [`enrolled`], with `more` objects ahead of the enrollments. Notification
/// Class 0's TO_OFFNORMAL needs acknowledging.
fn enrolled_with(
    more: Vec<Box<dyn bacnet_objects::traits::BACnetObject>>,
    enrollments: Vec<EventEnrollmentObject>,
    destinations: Vec<BACnetDestination>,
    (sweep, pass): (u64, Duration),
) -> Device {
    let identity = DeviceIdentity::new(9, 0x105)
        .unwrap()
        .with_services(SERVICES);
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    class.ack_required = EventTransitionBits::TO_OFFNORMAL;
    for destination in destinations {
        class.add_destination(destination).unwrap();
    }
    let mut sensor = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    sensor.set_present_value(10.0);
    let mut objects: Vec<Box<dyn bacnet_objects::traits::BACnetObject>> =
        vec![Box::new(sensor), Box::new(class)];
    objects.extend(more);
    objects.extend(enrollments.into_iter().map(|ee| Box::new(ee) as Box<_>));
    let db = build_database_with_extra(&identity, objects).unwrap();
    let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
    let config = ReportConfig::new(crate::cov::CovPolicy::default())
        .with_confirmed(FlightTiming::new(Duration::from_secs(3), 1))
        .with_sweep(Duration::from_secs(sweep))
        .with_event_enrollment(pass);
    let mut device = Device {
        reporter: Reporter::new(responder, config).unwrap(),
        db,
        ids: Ids {
            limit: 4,
            ..Ids::default()
        },
        invoke_id: 0,
    };
    device
        .reporter
        .on_deadline(&mut device.db, &mut device.ids, T0);
    device
}

impl Device {
    /// Take AI-1 to `value`, as the application's reading.
    fn read(&mut self, value: f32, now: Instant) {
        let reading = LocalValue::Real(value);
        let applied = self.reporter.update(&mut self.db, ai1(), reading, now);
        assert!(applied.is_ok(), "{applied:?}");
    }

    fn event_state(&self, oid: ObjectIdentifier) -> PropertyValue {
        let object = self.db.get(&oid).unwrap();
        object.read_property(P::EVENT_STATE, None).unwrap()
    }

    fn events(&mut self, now: Instant) -> usize {
        core::iter::from_fn(|| self.pop(now))
            .inspect(|(_, apdu)| assert!(is_event(apdu), "{apdu:?}"))
            .count()
    }
}

fn state(state: EventState) -> PropertyValue {
    PropertyValue::Enumerated(state.to_raw())
}

const TEN: Duration = Duration::from_secs(10);

#[test]
fn an_enrollment_pass_needs_the_event_path() {
    let identity = DeviceIdentity::new(9, 0x105).unwrap();
    let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
    let config = ReportConfig::default().with_event_enrollment(TEN);
    assert!(Reporter::new(responder, config).is_err());
}

/// The first pass runs at the first deadline, then one a period later:
/// a value the host changes in between is seen at the next pass, and the
/// transition goes out from there, as an intrinsic one does.
#[test]
fn a_pass_sees_the_value_at_its_period() {
    let recipient = address_destination(false);
    let mut device = enrolled(
        vec![enrollment(1, P::PRESENT_VALUE, 0)],
        vec![recipient],
        (60, TEN),
    );
    assert_eq!(device.reporter.next_deadline(), Some(ms(10_000)));
    device.read(90.0, ms(500));
    assert_eq!(device.events(ms(500)), 0, "no pass yet");
    assert_eq!(device.event_state(ee(1)), state(EventState::NORMAL));
    assert_eq!(device.run_until(ms(10_000)), vec![ms(10_000)]);
    let (sent, apdu) = device.pop(ms(10_000)).expect("the transition");
    assert!(is_event(&apdu), "{apdu:?}");
    assert_eq!(sent.service, NotifyService::Event);
    assert_eq!(sent.to, Destination::Recipient(recipient_address()));
    assert_eq!(device.event_state(ee(1)), state(EventState::HIGH_LIMIT));
    assert_eq!(device.reporter.counters().events, 1);
    assert_eq!(device.reporter.next_deadline(), Some(ms(20_000)));
}

/// A host that calls late gets one pass, not one per period missed: a
/// Time_Delay of 15 s at 10 s a pass counts two passes, so a burst at the
/// late call would fire it there.
#[test]
fn a_late_call_runs_one_pass() {
    let mut device = enrolled(
        vec![enrollment(1, P::PRESENT_VALUE, 15)],
        vec![address_destination(false)],
        (60, TEN),
    );
    device.read(90.0, ms(500));
    assert_eq!(device.run_until(ms(10_000)), vec![ms(10_000)], "seeded");
    device
        .reporter
        .on_deadline(&mut device.db, &mut device.ids, ms(35_000));
    assert_eq!(device.events(ms(35_000)), 0, "one pass at 35 s");
    assert_eq!(device.event_state(ee(1)), state(EventState::NORMAL));
    assert_eq!(device.reporter.next_deadline(), Some(ms(45_000)));
    assert_eq!(device.run_until(ms(45_000)), vec![ms(45_000)]);
    assert_eq!(device.events(ms(45_000)), 1, "the second pass fires it");
}

/// The period counts in whole seconds, rounded down, at least one.
#[test]
fn the_period_rounds_down_to_whole_seconds() {
    for (period, next) in [(1_900, 1_000), (500, 1_000), (2_000, 2_000)] {
        let device = enrolled(
            vec![enrollment(1, P::PRESENT_VALUE, 0)],
            Vec::new(),
            (60, Duration::from_millis(period)),
        );
        assert_eq!(device.reporter.next_deadline(), Some(ms(next)), "{period}");
    }
}

/// A transition Event_Enable leaves out commits and sends nothing; one
/// whose Event Values don't project commits and is counted.
#[test]
fn what_a_pass_commits_but_does_not_send() {
    let mut cleared = enrollment(1, P::PRESENT_VALUE, 0);
    cleared.set_event_enable(EventTransitionBits::empty());
    // AI-1's Notification_Class, an Unsigned 0, under a low limit of 5:
    // LOW_LIMIT commits with no REAL to project.
    let mut unsigned = enrollment(2, P::NOTIFICATION_CLASS, 0);
    unsigned.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 5.0,
        high_limit: 80.0,
        deadband: 1.0,
    });
    let mut device = enrolled(
        vec![cleared, unsigned],
        vec![address_destination(false)],
        (60, TEN),
    );
    device.read(90.0, ms(500));
    device.run_until(ms(10_000));
    assert_eq!(device.events(ms(10_000)), 0);
    assert_eq!(device.event_state(ee(1)), state(EventState::HIGH_LIMIT));
    assert_eq!(device.event_state(ee(2)), state(EventState::LOW_LIMIT));
    assert_eq!(device.reporter.counters().enrollments_suppressed, 1);
}

/// A confirmed transition with no invoke ID free is held as an intrinsic
/// one is, and goes when an ID comes back.
#[test]
fn a_confirmed_transition_waits_for_an_invoke_id() {
    let mut device = enrolled(
        vec![enrollment(1, P::PRESENT_VALUE, 0)],
        vec![address_destination(true)],
        (60, TEN),
    );
    device.ids.limit = 0;
    device.read(90.0, ms(500));
    device.run_until(ms(10_000));
    assert!(device.pop(ms(10_000)).is_none(), "no invoke ID");
    assert!(device.reporter.owes_invoke_id());
    device.ids.limit = 1;
    device.reporter.invoke_ids_released(1);
    let (sent, apdu) = device.pop(ms(10_100)).expect("now it goes");
    assert!(is_event(&apdu) && sent.invoke_id.is_some(), "{apdu:?}");
}

/// A monitored reference naming another device isn't evaluated: the core
/// reads local references only, so each pass is an observation gap, with
/// no transition and no Reliability change, as on the server.
#[test]
fn a_reference_to_another_device_is_not_evaluated() {
    let mut remote = enrollment(1, P::PRESENT_VALUE, 0);
    let elsewhere = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
    let reference = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(elsewhere),
        ..BACnetDeviceObjectPropertyReference::new_local(ai1(), P::PRESENT_VALUE.to_raw())
    };
    remote
        .set_object_property_reference(Some(reference))
        .unwrap();
    let mut device = enrolled(vec![remote], vec![address_destination(false)], (60, TEN));
    device.read(90.0, ms(500));
    device.run_until(ms(30_000));
    assert_eq!(device.events(ms(30_000)), 0);
    assert_eq!(device.event_state(ee(1)), state(EventState::NORMAL));
    let reliability = device
        .db
        .get(&ee(1))
        .unwrap()
        .read_property(P::RELIABILITY, None);
    let none = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    assert_eq!(reliability.unwrap(), none);
}

/// With no period set the reporter runs no pass.
#[test]
fn without_a_period_no_pass_runs() {
    let identity = DeviceIdentity::new(9, 0x105).unwrap();
    let objects: Vec<Box<dyn bacnet_objects::traits::BACnetObject>> = vec![
        Box::new(AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap()),
        Box::new(enrollment(1, P::PRESENT_VALUE, 0)),
    ];
    let mut db = build_database_with_extra(&identity, objects).unwrap();
    let responder = Responder::new(identity, crate::responder::Limits::default()).unwrap();
    let config = ReportConfig::default().with_sweep(Duration::from_secs(1));
    let mut reporter = Reporter::new(responder, config).unwrap();
    let applied = reporter.update(&mut db, ai1(), LocalValue::Real(90.0), T0);
    assert!(applied.is_ok());
    for second in 0..30 {
        let now = T0.saturating_add(Duration::from_secs(second));
        reporter.on_deadline(&mut db, &mut Ids::default(), now);
    }
    let object = db.get(&ee(1)).unwrap();
    let event_state = object.read_property(P::EVENT_STATE, None).unwrap();
    assert_eq!(event_state, state(EventState::NORMAL));
}

/// The Event Enrollment an event notification reports.
fn reported(apdu: &Apdu) -> ObjectIdentifier {
    let params = match apdu {
        Apdu::UnconfirmedRequest(request) => &request.service_request,
        Apdu::ConfirmedRequest(request) => &request.service_request,
        other => panic!("not a notification: {other:?}"),
    };
    decode_event_notification(params)
        .unwrap()
        .event_object_identifier
}

/// Enrollments that trip at one pass go out in the database's order, the
/// order they were added in, not by identifier.
#[test]
fn one_pass_sends_in_database_order() {
    let enrollments = [3, 1, 2].map(|instance| enrollment(instance, P::PRESENT_VALUE, 0));
    let mut device = enrolled(
        enrollments.into(),
        vec![address_destination(false)],
        (60, TEN),
    );
    device.read(90.0, ms(500));
    device.run_until(ms(10_000));
    let popped: Vec<_> = core::iter::from_fn(|| device.pop(ms(10_000)))
        .map(|(_, apdu)| reported(&apdu))
        .collect();
    assert_eq!(popped, [ee(3), ee(1), ee(2)]);
}

/// In one `on_deadline` the sweep runs before the pass: an intrinsic
/// transition the sweep commits there sets the input's IN_ALARM, which an
/// enrollment watching its Status_Flags sees in the same call.
#[test]
fn the_pass_sees_what_the_sweep_committed_in_the_same_call() {
    let mut watching =
        EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_BITSTRING).unwrap();
    let alarm3 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let reference =
        BACnetDeviceObjectPropertyReference::new_local(alarm3, P::STATUS_FLAGS.to_raw());
    watching
        .set_object_property_reference(Some(reference))
        .unwrap();
    watching.set_event_parameters(BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (4, vec![0x80]),
        list_of_values: vec![(4, vec![0x80])],
    });
    watching.set_event_enable(EventTransitionBits::all());
    let second = Duration::from_secs(1);
    let mut device = enrolled_with(
        vec![intrinsic(3, 1)],
        vec![watching],
        vec![address_destination(false)],
        (1, second),
    );
    // Due a second on, when the sweep and the pass both run.
    device.raise(alarm3, T0);
    assert_eq!(device.run_until(ms(1_000)), vec![ms(1_000)]);
    let popped: Vec<_> = core::iter::from_fn(|| device.pop(ms(1_000)))
        .map(|(_, apdu)| reported(&apdu))
        .collect();
    assert_eq!(
        popped,
        [alarm3, ee(1)],
        "the intrinsic one, then the pass's"
    );
    assert_eq!(device.event_state(ee(1)), state(EventState::OFFNORMAL));
}

/// AcknowledgeAlarm through `handle` acknowledges an enrollment's
/// transition: the SimpleACK, then its acknowledgement notification.
#[test]
fn acknowledge_alarm_takes_an_enrollments_transition() {
    let mut device = enrolled(
        vec![enrollment(1, P::PRESENT_VALUE, 0)],
        vec![address_destination(false)],
        (60, TEN),
    );
    device.read(90.0, ms(500));
    device.run_until(ms(10_000));
    assert_eq!(device.events(ms(10_000)), 1, "the transition");
    let object = device.db.get(&ee(1)).unwrap();
    let coordinate = bacnet_objects::event::EventTransition::ToOffnormal;
    let optional = crate::event::MessageTexts::Optional;
    let committed = crate::event::read_committed_history_snapshot(object, coordinate, optional);
    let mut body = BytesMut::new();
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 7,
        event_object_identifier: ee(1),
        event_state_acknowledged: EventState::HIGH_LIMIT,
        timestamp: committed.unwrap().timestamp,
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(9),
    }
    .encode(&mut body)
    .unwrap();
    let reply = device.request(ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, &body, ms(11_000));
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    let (_, apdu) = device.pop(ms(11_000)).expect("its acknowledgement");
    assert_eq!(reported(&apdu), ee(1));
    let params = match &apdu {
        Apdu::UnconfirmedRequest(request) => request.service_request.clone(),
        other => panic!("{other:?}"),
    };
    let notification = decode_event_notification(&params).unwrap();
    let ack = bacnet_types::enums::NotifyType::ACK_NOTIFICATION;
    assert_eq!(notification.notify_type, ack);
}

/// GetEventInformation through `handle` lists an enrollment in alarm.
#[test]
fn get_event_information_lists_an_enrollment() {
    let mut device = enrolled(
        vec![enrollment(1, P::PRESENT_VALUE, 0)],
        vec![address_destination(false)],
        (60, TEN),
    );
    device.read(90.0, ms(500));
    device.run_until(ms(10_000));
    let reply = device.request(
        ConfirmedServiceChoice::GET_EVENT_INFORMATION,
        &[],
        ms(11_000),
    );
    let Apdu::ComplexAck(ack) = reply else {
        panic!("{reply:?}")
    };
    let encoded = ee(1).encode();
    assert!(ack.service_ack.windows(4).any(|window| window == encoded));
}

//! Intrinsic event reporting (#2303): the server and bacnet-device-core's
//! `Reporter`, given the same writes and requests over the same fixture at
//! the same paused instants, send the same event notifications, octet for
//! octet, to the same recipients at the same network priority, and answer
//! AcknowledgeAlarm and GetEventInformation alike.
//!
//! Both start at one instant: the server's intrinsic task sweeps when it
//! starts, and the `Reporter` sweeps at its first deadline, given that
//! instant, so their periodic sweeps fall together. Each test then steps
//! through the `Reporter`'s deadlines, letting the server's timers due at
//! the same instant run, and compares what each sent there. Every
//! destination is unconfirmed, so the APDUs compare whole.
use super::*;
use crate::runtime_clock;
use bacnet_device_core::cov::{Delivery, Token};
use bacnet_device_core::event::{MessageTexts, read_committed_history_snapshot};
use bacnet_device_core::report::{Bindings, Destination, InvokeIds};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_npdu::NetworkPriority;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::event::EventTransition;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::EventState;
use bacnet_types::primitives::BACnetTimeStamp;

#[path = "events_confirmed.rs"]
mod confirmed_events;
#[path = "events_device.rs"]
mod device;
#[path = "events_enrollment.rs"]
mod enrollment;
#[path = "events_more.rs"]
mod more;

/// What differs by design: without a Device clock the server judges a
/// destination's window by the system clock's UTC day and time, and the
/// core, which has no wall clock, sends only to a destination open every
/// day, all day (§6.3, decision 14).
const DIFFER: &[&str] = &["a restricted window without a clock"];

/// Recipients, by the MAC their notifications go to.
const ALWAYS: [u8; 6] = [10, 0, 9, 1, 0xBA, 0xC0];
const MONDAYS: [u8; 6] = [10, 0, 9, 2, 0xBA, 0xC0];
const AFTERNOON: [u8; 6] = [10, 0, 9, 3, 0xBA, 0xC0];
const MIDDAY: [u8; 6] = [10, 0, 9, 4, 0xBA, 0xC0];
const TO_NORMAL: [u8; 6] = [10, 0, 9, 5, 0xBA, 0xC0];
const NO_REAL_TIME: [u8; 6] = [10, 0, 9, 6, 0xBA, 0xC0];

fn av() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 1)
}

fn at(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn destination(
    mac: [u8; 6],
    valid_days: DaysOfWeek,
    (from_time, to_time): (Time, Time),
    transitions: EventTransitionBits,
) -> BACnetDestination {
    BACnetDestination {
        valid_days,
        from_time,
        to_time,
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&mac),
        }),
        process_identifier: u32::from(mac[3]),
        issue_confirmed_notifications: false,
        transitions,
    }
}

/// The fixture's destinations: by day, by window and by transition, around
/// the fixed clock's Friday noon.
fn recipients() -> Vec<BACnetDestination> {
    let all_day = (
        at(0, 0),
        Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
    );
    let every = EventTransitionBits::all();
    vec![
        destination(ALWAYS, DaysOfWeek::all(), all_day, every),
        destination(MONDAYS, DaysOfWeek::MONDAY, all_day, every),
        destination(AFTERNOON, DaysOfWeek::all(), (at(13, 0), at(14, 0)), every),
        destination(MIDDAY, DaysOfWeek::all(), (at(11, 0), at(12, 30)), every),
        destination(
            TO_NORMAL,
            DaysOfWeek::all(),
            all_day,
            EventTransitionBits::TO_NORMAL,
        ),
    ]
}

/// Analog Value `instance` alarming over 80 (deadband 1) after
/// `time_delay` seconds, back after one, through Notification Class 1.
fn alarm_object(instance: u32, time_delay: u64) -> AnalogValueObject {
    let name = format!("AV-{instance}");
    let mut value =
        AnalogValueObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (property, real) in [
        (P::HIGH_LIMIT, 80.0f32),
        (P::LOW_LIMIT, -10.0),
        (P::DEADBAND, 1.0),
    ] {
        value
            .write_property(property, None, PropertyValue::Real(real), None)
            .unwrap();
    }
    for (property, unused_bits, bits) in [(P::LIMIT_ENABLE, 6, 0xC0), (P::EVENT_ENABLE, 5, 0xE0)] {
        let bits = PropertyValue::BitString {
            unused_bits,
            data: vec![bits],
        };
        value.write_property(property, None, bits, None).unwrap();
    }
    for (property, unsigned) in [
        (P::TIME_DELAY, time_delay),
        (P::TIME_DELAY_NORMAL, 1),
        (P::NOTIFICATION_CLASS, 1),
    ] {
        let unsigned = PropertyValue::Unsigned(unsigned);
        value
            .write_property(property, None, unsigned, None)
            .unwrap();
    }
    value
}

/// Notification Class 1 with `destinations`, whose TO_OFFNORMAL needs
/// acknowledging, and event priorities other than 255.
fn alarm_class(destinations: Vec<BACnetDestination>) -> NotificationClass {
    let mut class = NotificationClass::new(1, "NC-1").unwrap();
    class.ack_required = EventTransitionBits::TO_OFFNORMAL;
    // Event priorities other than 255, so the network priority is compared
    // (Table 13-6: critical equipment, then normal).
    class.priority = [100, 200, 255];
    for destination in destinations {
        class.add_destination(destination).unwrap();
    }
    class
}

/// An Analog Value alarming over 80 (deadband 1) after `time_delay`
/// seconds, back after one, through Notification Class 1 with
/// `destinations`, whose TO_OFFNORMAL needs acknowledging.
fn alarm_value(
    time_delay: u64,
    destinations: Vec<BACnetDestination>,
) -> Vec<Box<dyn BACnetObject>> {
    let value = alarm_object(1, time_delay);
    let class = alarm_class(destinations);
    // The fixture's Analog Input reports through class 0.
    let class0 = NotificationClass::new(0, "NC-0").unwrap();
    vec![Box::new(value), Box::new(class), Box::new(class0)]
}

fn delayed(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    database_with(identity, alarm_value(2, recipients()))
}

/// Time_Delay 0, with the destination open all day and one whose window
/// starts at an hour no real time has: open all day by the window test the
/// server applies to the system clock's time, closed by the clockless
/// test. No other window, which the server would judge by the wall clock.
fn undelayed(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let always = recipients().swap_remove(0);
    let bogus = (at(25, 0), at(24, 0));
    let every = EventTransitionBits::all();
    let unreal = destination(NO_REAL_TIME, DaysOfWeek::all(), bogus, every);
    database_with(identity, alarm_value(0, vec![always, unreal]))
}

/// The core side's invoke IDs: the next free one in turn, so a late answer
/// to an ended notification doesn't match the one after it.
#[derive(Default)]
struct Ids {
    taken: Vec<(BACnetAddress, u8)>,
    next: u8,
}

impl InvokeIds for Ids {
    fn take(&mut self, to: &BACnetAddress, _: ConfirmedServiceChoice) -> Option<u8> {
        let id = (0..=u8::MAX)
            .map(|step| self.next.wrapping_add(step))
            .find(|id| !self.taken.contains(&(to.clone(), *id)))?;
        self.next = id.wrapping_add(1);
        self.taken.push((to.clone(), id));
        Some(id)
    }
    fn release(&mut self, to: &BACnetAddress, id: u8) {
        self.taken.retain(|taken| *taken != (to.clone(), id));
    }
}

/// The core side's Device bindings: what its host's Who-Is/I-Am cache holds.
/// Empty, it resolves nothing.
#[derive(Default)]
struct Known(Vec<(ObjectIdentifier, BACnetAddress)>);

impl Bindings for Known {
    fn address_of(&self, device: ObjectIdentifier) -> Option<BACnetAddress> {
        let found = self.0.iter().find(|known| known.0 == device);
        found.map(|known| known.1.clone())
    }
}

/// `apdu`'s invoke ID, zeroed in it when it is a confirmed request: each
/// host lends its own, so the rest is what compares.
fn strip(apdu: &mut [u8]) -> Option<u8> {
    let confirmed = apdu[0] >> 4 == 0;
    confirmed.then(|| core::mem::take(&mut apdu[2]))
}

/// The unconfirmed notifications of `sent`, in order.
fn unconfirmed_ones<T>(sent: &[(Wire, Option<T>)]) -> Vec<Wire> {
    let unconfirmed = sent.iter().filter(|sent| sent.1.is_none());
    unconfirmed.map(|sent| sent.0.clone()).collect()
}

/// All of `sent`, in order.
fn wires<T>(sent: &[(Wire, Option<T>)]) -> Vec<Wire> {
    sent.iter().map(|sent| sent.0.clone()).collect()
}

/// A confirmed notification both sides sent, awaiting its answer: the
/// recipient's link MAC and the invoke ID each side lent it.
struct Outstanding {
    mac: Vec<u8>,
    server: u8,
    core: u8,
    token: Token,
}

/// One notification as it went out: link MAC, NPDU priority and APDU, a
/// confirmed one's invoke ID zeroed.
type Wire = (Vec<u8>, NetworkPriority, Vec<u8>);

/// Both sides of one fixture, the `Reporter` with its event path on.
struct EventParity {
    parity: Parity,
    /// The instant both started.
    begun: bacnet_time::Instant,
    invoke_id: u8,
    /// The core side's invoke IDs and Device bindings.
    ids: Ids,
    bindings: Known,
    /// The confirmed notifications neither side has been answered for.
    outstanding: Vec<Outstanding>,
    /// What the server sent at each comparison, which matched, until taken.
    compared: Vec<Wire>,
}

impl EventParity {
    /// Both sides of `fixture`, the core's `Responder` with a Device clock
    /// set to the fixed clock's date and time and standing there, as the
    /// server's fixed reader does: windows are judged by its synchronized
    /// state, and timestamps read alike.
    async fn start(fixture: fn(&bacnet_objects::DeviceIdentity) -> ObjectDatabase) -> Self {
        Self::start_with(fixture, true, None).await
    }

    /// Both sides of `fixture`, the core's `Responder` with no clock: it
    /// selects only destinations open every day, all day, and refuses
    /// restricted Recipient_List entries.
    async fn start_clockless(
        fixture: fn(&bacnet_objects::DeviceIdentity) -> ObjectDatabase,
    ) -> Self {
        Self::start_with(fixture, false, None).await
    }

    /// Both sides of `fixture`, clocked, each running the Event Enrollment
    /// pass every `enrollment` seconds when given (#2319); without it the
    /// server keeps its default and the `Reporter` runs none.
    async fn start_with(
        fixture: fn(&bacnet_objects::DeviceIdentity) -> ObjectDatabase,
        clocked: bool,
        enrollment: Option<u64>,
    ) -> Self {
        let begun = runtime_clock::core_now();
        let interval = enrollment.unwrap_or(ServerConfig::default().event_enrollment_interval_secs);
        let mut parity = Parity::start_enrolling(
            ReadPropertyMultipleBudget::default(),
            Segmentation::NO_SEGMENTATION,
            fixture,
            interval,
        )
        .await;
        let settled = begun + bacnet_time::Duration::from_millis(50);
        assert_eq!(runtime_clock::core_now(), settled, "started at `begun`");
        let config = &parity.server.config;
        let budget = config.read_property_multiple_budget;
        let limits = Limits::new(budget.max_result_elements).with_rpm(budget);
        let identity = config
            .device
            .clone()
            .with_segmentation(Segmentation::NO_SEGMENTATION);
        let report = ReportConfig::new(config.cov_policy.clone())
            .with_confirmed(config.cov_flight_timing())
            .with_outbox(OutboxLimits {
                entries: 64,
                octets: 65_536,
            })
            .with_sweep(bacnet_time::Duration::from_secs(1));
        let report = match enrollment {
            Some(secs) => report.with_event_enrollment(bacnet_time::Duration::from_secs(secs)),
            None => report,
        };
        let mut responder = Responder::new(identity, limits).unwrap();
        if clocked {
            use bacnet_device_core::clock::{Basis, DeviceClock, Zone};
            let still = move || begun;
            let clock = DeviceClock::new(Zone::new(0, false).unwrap(), Arc::new(still));
            responder = responder.with_clock(clock, &mut parity.db);
            let frame = FixedClock.read_clock().unwrap();
            let (date, time) = (frame.local_date, frame.local_time);
            responder
                .set_time(&mut parity.db, Basis::Local, date, time)
                .unwrap();
        }
        parity.reporter = Reporter::new(responder, report).unwrap();
        // The server's task swept when it started.
        let mut ids = Ids::default();
        parity.reporter.on_deadline(&mut parity.db, &mut ids, begun);
        Self {
            parity,
            begun,
            invoke_id: 0,
            ids,
            bindings: Known::default(),
            outstanding: Vec::new(),
            compared: Vec::new(),
        }
    }

    fn instant(&self, millis: u64) -> bacnet_time::Instant {
        self.begun + bacnet_time::Duration::from_millis(millis)
    }

    async fn sleep_until(&self, at: bacnet_time::Instant) {
        let at = runtime_clock::core_clock().to_std(at).unwrap();
        tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await;
    }

    /// The `Reporter`'s notifications at `now`, each reported sent.
    fn core_sent(&mut self, now: bacnet_time::Instant) -> Vec<Wire> {
        let sent = self.core_sent_ids(now);
        sent.into_iter().map(|(wire, _)| wire).collect()
    }

    /// [`core_sent`](Self::core_sent), a confirmed one with its invoke ID
    /// and token, and left outstanding for its answer.
    fn core_sent_ids(&mut self, now: bacnet_time::Instant) -> Vec<(Wire, Option<(u8, Token)>)> {
        let mut sent = Vec::new();
        let reporter = &mut self.parity.reporter;
        loop {
            let mut out = BytesMut::new();
            let Some(notification) = reporter.pop(&self.bindings, &mut self.ids, now, &mut out)
            else {
                return sent;
            };
            let Destination::Recipient(to) = notification.to else {
                unreachable!("only event notifications here")
            };
            assert_eq!(to.network_number, 0, "a local recipient");
            let mut apdu = out.to_vec();
            let id = strip(&mut apdu);
            assert_eq!(id, notification.invoke_id);
            let token = notification.token;
            if id.is_none() {
                reporter.delivered(token, Delivery::Sent, &mut self.ids, now);
            }
            let wire = (to.mac_address.to_vec(), notification.priority, apdu);
            sent.push((wire, id.map(|id| (id, token))));
        }
    }

    /// Compare the server's `frames` with the `Reporter`'s notifications at
    /// `now`, leaving out any to `skip` (which the server must have sent);
    /// how many the server sent.
    fn same_events(
        &mut self,
        what: &str,
        frames: &[test_transport::SentFrame],
        now: bacnet_time::Instant,
        skip: Option<[u8; 6]>,
    ) -> usize {
        let mut server: Vec<(Wire, Option<u8>)> = frames
            .iter()
            .map(|frame| {
                let npdu = frame.decode_npdu();
                assert!(npdu.destination.is_none(), "{what}: local");
                let mut apdu = npdu.payload.to_vec();
                let id = strip(&mut apdu);
                ((frame.mac.to_vec(), npdu.priority, apdu), id)
            })
            .collect();
        let count = server.len();
        if let Some(skip) = skip {
            let before = server.len();
            server.retain(|sent| sent.0.0 != skip);
            assert_eq!(server.len() + 1, before, "{what}: the pinned recipient");
        }
        let mut core = self.core_sent_ids(now);
        // The unconfirmed ones go out in the Recipient_List's order from
        // both: one task sends them, the `Reporter`'s outbox keeps it.
        let (first, second) = (unconfirmed_ones(&server), unconfirmed_ones(&core));
        assert_eq!(
            first, second,
            "{what}: unconfirmed, in order, server then core"
        );
        // Between a confirmed one and another the server's order is its
        // scheduling, each leaving from a worker of its own, and between a
        // confirmed one and an unconfirmed one too: they are compared as a
        // set.
        server.sort_by_key(|sent| (sent.0.0.clone(), sent.0.2.clone()));
        core.sort_by_key(|sent| (sent.0.0.clone(), sent.0.2.clone()));
        assert_eq!(
            wires(&server),
            wires(&core),
            "{what}: the notifications, server then core"
        );
        self.compared.extend(wires(&server));
        for ((_, server), (wire, core)) in server.iter().zip(&core) {
            if let (Some(server), Some((core, token))) = (server, core) {
                // A resend is the same notification again: keep one record.
                self.outstanding.retain(|out| out.token != *token);
                self.outstanding.push(Outstanding {
                    mac: wire.0.clone(),
                    server: *server,
                    core: *core,
                    token: *token,
                });
            }
        }
        count
    }

    /// Run the `Reporter`'s work due at `now`, as the server's tasks run
    /// theirs.
    fn settle(&mut self, now: bacnet_time::Instant) {
        let parity = &mut self.parity;
        while parity
            .reporter
            .next_deadline()
            .is_some_and(|due| due <= now)
        {
            parity
                .reporter
                .on_deadline(&mut parity.db, &mut self.ids, now);
            if parity.reporter.next_deadline() == Some(now) {
                break;
            }
        }
    }

    /// Step to `millis` after the start through each of the `Reporter`'s
    /// deadlines, comparing what both sent at each; how many the server
    /// sent.
    async fn run_to(&mut self, what: &str, millis: u64) -> usize {
        let end = self.instant(millis);
        let mut sent = 0;
        loop {
            let due = self
                .parity
                .reporter
                .next_deadline()
                .filter(|due| *due < end)
                .unwrap_or(end);
            self.sleep_until(due).await;
            // Let the server's timers due at the same instant run.
            tokio::time::sleep(Duration::from_millis(1)).await;
            let frames = self.parity.sent.take();
            self.settle(due);
            sent += self.same_events(what, &frames, due, None);
            if due == end {
                return sent;
            }
        }
    }

    /// Send the confirmed `service` request `body` to both at `millis`
    /// after the start: the reply, which must agree, and the event
    /// notifications that followed it, compared but for any to `skip`;
    /// how many the server sent.
    async fn request(
        &mut self,
        what: &str,
        millis: u64,
        service: ConfirmedServiceChoice,
        body: &[u8],
        skip: Option<[u8; 6]>,
    ) -> (Apdu, usize) {
        self.request_sized(what, millis, (service, body), 1476, skip)
            .await
    }

    /// [`request`](Self::request), the requester accepting replies of up to
    /// `max_apdu` octets.
    async fn request_sized(
        &mut self,
        what: &str,
        millis: u64,
        (service, body): (ConfirmedServiceChoice, &[u8]),
        max_apdu: u16,
        skip: Option<[u8; 6]>,
    ) -> (Apdu, usize) {
        let now = self.instant(millis);
        self.sleep_until(now).await;
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let apdu = confirmed(self.invoke_id, service, body, max_apdu);
        let mut frames = self.parity.deliver(&apdu).await;
        let source = self.parity.source();
        let at = frames
            .iter()
            .position(|frame| frame.mac.as_slice() == source.as_slice())
            .unwrap_or_else(|| panic!("{what}: the server's reply"));
        let server = frames.remove(at).decode_npdu().payload;
        let peer = SubscriberEndpoint::new(&source, None);
        let request = Inbound::new(apdu, max_apdu).with_source(self.parity.origin());
        let mut out = BytesMut::new();
        let parity = &mut self.parity;
        let handled = parity
            .reporter
            .handle(&mut parity.db, &peer, request, now, &mut out);
        assert_eq!(handled.outcome, Outcome::Reply, "{what}");
        assert_eq!(server, out.freeze(), "{what}: the reply, server then core");
        let count = self.same_events(what, &frames, now, skip);
        (bacnet_encoding::apdu::decode_apdu(server).unwrap(), count)
    }

    async fn write(&mut self, what: &str, millis: u64, property: P, value: PropertyValue) -> usize {
        self.write_to(what, millis, (av(), property), value).await
    }

    /// Write `value` to `property` of `object` on both at `millis`; how
    /// many event notifications the server sent with the reply.
    async fn write_to(
        &mut self,
        what: &str,
        millis: u64,
        (object, property): (ObjectIdentifier, P),
        value: PropertyValue,
    ) -> usize {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value).unwrap();
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: None,
            property_value: encoded.to_vec(),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let (reply, count) = self.request(what, millis, service, &body, None).await;
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{what}: {reply:?}");
        count
    }

    async fn value(&mut self, what: &str, millis: u64, value: f32) -> usize {
        self.write(what, millis, P::PRESENT_VALUE, PropertyValue::Real(value))
            .await
    }

    /// The Event_State each side's database holds for the Analog Value.
    async fn event_states(&self) -> (PropertyValue, PropertyValue) {
        let read = |db: &ObjectDatabase| {
            db.get(&av())
                .unwrap()
                .read_property(P::EVENT_STATE, None)
                .unwrap()
        };
        let server = read(&*self.parity.server.database().read().await);
        (server, read(&self.parity.db))
    }

    async fn acknowledge(
        &mut self,
        what: &str,
        millis: u64,
        state: EventState,
        time: Time,
    ) -> (Apdu, usize) {
        let service = ConfirmedServiceChoice::ACKNOWLEDGE_ALARM;
        let body = acknowledgement(state, time);
        self.request(what, millis, service, &body, None).await
    }

    /// The server takes AcknowledgeAlarm for the Analog Value's latest
    /// HIGH_LIMIT at `millis`, the core `acknowledge_local`: the same
    /// notifications; how many.
    async fn acknowledge_locally(&mut self, what: &str, millis: u64) -> usize {
        let now = self.instant(millis);
        self.sleep_until(now).await;
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let service = ConfirmedServiceChoice::ACKNOWLEDGE_ALARM;
        let body = acknowledgement(EventState::HIGH_LIMIT, at(12, 0));
        let apdu = confirmed(self.invoke_id, service, &body, 1476);
        let mut frames = self.parity.deliver(&apdu).await;
        let source = self.parity.source();
        frames.retain(|frame| frame.mac.as_slice() != source.as_slice());
        let parity = &mut self.parity;
        let state = EventState::HIGH_LIMIT;
        let local = parity
            .reporter
            .acknowledge_local(&mut parity.db, av(), state, now);
        assert!(local.is_ok(), "{what}: {local:?}");
        self.same_events(what, &frames, now, None)
    }
}

/// An AcknowledgeAlarm of the Analog Value's transition to `state`,
/// stamped the fixed clock's date at `time`.
fn acknowledgement(state: EventState, time: Time) -> BytesMut {
    let stamp = BACnetTimeStamp::DateTime {
        date: Date {
            year: 126,
            month: 10,
            day: 30,
            day_of_week: 5,
        },
        time,
    };
    let mut body = BytesMut::new();
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: av(),
        event_state_acknowledged: state,
        timestamp: stamp.clone(),
        acknowledgment_source: "parity".into(),
        time_of_acknowledgment: stamp,
    }
    .encode(&mut body)
    .unwrap();
    body
}

fn high_limit() -> PropertyValue {
    PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
}

#[tokio::test(start_paused = true)]
async fn delays_recipients_and_event_enable_match() {
    let mut both = EventParity::start(delayed).await;
    // Seeded at 0.9 s with Time_Delay 2: due at 2.9 s, not at the 2 s sweep.
    assert_eq!(both.value("over the limit", 900, 90.0).await, 0);
    assert_eq!(both.run_to("before its delay ends", 2_899).await, 0);
    // To the destination open all day and the one open at noon; not
    // Mondays', the afternoon's, or TO_NORMAL's alone.
    assert_eq!(both.run_to("its delay ended", 2_950).await, 2);
    assert_eq!(both.value("back under", 4_500, 50.0).await, 0);
    assert_eq!(both.run_to("before Time_Delay_Normal", 5_450).await, 0);
    assert_eq!(both.run_to("back to normal", 5_550).await, 3);
    // A value that reverts inside the delay reports nothing.
    assert_eq!(both.value("over again", 6_200, 90.0).await, 0);
    assert_eq!(both.value("back inside the delay", 7_000, 50.0).await, 0);
    assert_eq!(both.run_to("past the delay", 9_500).await, 0);
    // Event_Enable cleared: the transition commits, and goes nowhere.
    let cleared = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![0],
    };
    assert_eq!(
        both.write("Event_Enable cleared", 9_600, P::EVENT_ENABLE, cleared)
            .await,
        0
    );
    assert_eq!(both.value("over, not enabled", 9_700, 90.0).await, 0);
    assert_eq!(both.run_to("committed, not sent", 12_000).await, 0);
    assert_eq!(both.event_states().await, (high_limit(), high_limit()));
}

#[tokio::test(start_paused = true)]
async fn acknowledgements_and_event_information_match() {
    let mut both = EventParity::start(delayed).await;
    assert_eq!(both.value("over the limit", 900, 90.0).await, 0);
    assert_eq!(both.run_to("its delay ended", 2_950).await, 2);
    let service = ConfirmedServiceChoice::GET_EVENT_INFORMATION;
    let (reply, _) = both.request("in alarm", 3_100, service, &[], None).await;
    assert!(matches!(reply, Apdu::ComplexAck(_)), "{reply:?}");
    let noon = at(12, 0);
    let (reply, sent) = both
        .acknowledge("the wrong state", 3_300, EventState::LOW_LIMIT, noon)
        .await;
    let wrong_state = |reply: &Apdu| matches!(reply, Apdu::Error(error) if error.error_code == ErrorCode::INVALID_EVENT_STATE);
    assert!(wrong_state(&reply), "{reply:?}");
    assert_eq!(sent, 0);
    let (reply, sent) = both
        .acknowledge(
            "the wrong timestamp",
            3_500,
            EventState::OFFNORMAL,
            at(11, 0),
        )
        .await;
    assert!(matches!(reply, Apdu::Error(_)), "{reply:?}");
    assert_eq!(sent, 0);
    let (reply, sent) = both
        .acknowledge("acknowledged", 3_700, EventState::HIGH_LIMIT, noon)
        .await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    // The acknowledgement notification, to the TO_OFFNORMAL recipients.
    assert_eq!(sent, 2);
    let (reply, _) = both
        .request("acknowledged", 3_900, service, &[], None)
        .await;
    assert!(matches!(reply, Apdu::ComplexAck(_)), "{reply:?}");
}

#[tokio::test(start_paused = true)]
async fn a_local_acknowledgement_notifies_as_the_service_does() {
    let mut both = EventParity::start(delayed).await;
    assert_eq!(both.value("over the limit", 900, 90.0).await, 0);
    assert_eq!(both.run_to("its delay ended", 2_950).await, 2);
    assert_eq!(
        both.acknowledge_locally("acknowledged locally", 3_100)
            .await,
        2
    );
    let service = ConfirmedServiceChoice::GET_EVENT_INFORMATION;
    let (reply, _) = both
        .request("acknowledged", 3_300, service, &[], None)
        .await;
    assert!(matches!(reply, Apdu::ComplexAck(_)), "{reply:?}");
}

#[tokio::test(start_paused = true)]
async fn without_a_clock_timestamps_are_sequence_numbers_and_windows_fail_closed() {
    let mut both = EventParity::start_clockless(undelayed).await;
    both.parity
        .server
        .database()
        .write()
        .await
        .set_clock_reader(None);
    both.parity.db.set_clock_reader(None);
    let what = DIFFER[0];
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &PropertyValue::Real(90.0)).unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: av(),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
        property_value: encoded.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    // The window has no day or time to open on: only the destination open
    // all day agrees; the server also sends to the other.
    let (reply, sent) = both
        .request(what, 300, service, &body, Some(NO_REAL_TIME))
        .await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(sent, 2, "{what}: the server's");
    assert_eq!(both.event_states().await, (high_limit(), high_limit()));
    // No clock: the transition is stamped with a sequence number on both.
    let stamp = |db: &ObjectDatabase| {
        let coordinate = EventTransition::for_target_state(EventState::HIGH_LIMIT);
        let object = db.get(&av()).unwrap();
        read_committed_history_snapshot(object, coordinate, MessageTexts::Optional)
            .unwrap()
            .timestamp
    };
    let server = stamp(&*both.parity.server.database().read().await);
    assert!(
        matches!(server, BACnetTimeStamp::SequenceNumber(_)),
        "{server:?}"
    );
    assert_eq!(server, stamp(&both.parity.db));
}

/// The core's `Responder` has no Device clock, so it refuses a
/// Recipient_List entry it couldn't honour; the server, which judges
/// windows by the system clock without one, stores it (decision 14).
#[tokio::test(start_paused = true)]
async fn a_restricted_recipient_list_is_stored_by_the_server_and_refused_by_the_core() {
    let mut both = EventParity::start_clockless(delayed).await;
    let what = "a restricted Recipient_List without a Device clock";
    let mornings = destination(
        ALWAYS,
        DaysOfWeek::all(),
        (at(0, 0), at(12, 0)),
        EventTransitionBits::all(),
    );
    let mut list = BytesMut::new();
    bacnet_encoding::constructed::encode_destination_list(&mut list, &[mornings]).unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::NOTIFICATION_CLASS, 1),
        property_identifier: P::RECIPIENT_LIST,
        property_array_index: None,
        property_value: list.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    let apdu = confirmed(1, ConfirmedServiceChoice::WRITE_PROPERTY, &body, 1476);
    let (server, core) = both.parity.differ(what, apdu, 1476).await;
    assert!(matches!(server, Some(Apdu::SimpleAck(_))), "{server:?}");
    let refused = |reply: &Option<Apdu>| {
        matches!(reply, Some(Apdu::Error(error))
            if error.error_class == ErrorClass::PROPERTY
                && error.error_code == ErrorCode::VALUE_OUT_OF_RANGE)
    };
    assert!(refused(&core), "{core:?}");
}

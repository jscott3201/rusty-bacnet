//! The background paths commit the event transition their change causes
//! with it (#2535, #2537): the fault-detection task, a Schedule's pass (the
//! 60-second tick, a local write of the Schedule, a peer's), the membership
//! work an application's removal leaves, a DeleteObject's, and the operation
//! task. Each COV report carries the settled Status_Flags, never the half
//! state; the event notification goes out at the change, not a sweep later;
//! a later write to the same object is told after it; the next sweeps send
//! nothing more; and a destination that holds its lane holds up neither the
//! task's next pass nor the sweep.
//!
//! Each test starts the clock at `t0`, before the server, so the tasks'
//! periods count from it: sweeps at whole seconds, fault-detection passes
//! every 10 s and Schedule ticks every 60 s.

use super::*;
use crate::server::event_recipient_routing_tests::destination_for;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_objects::traits::{CovReporting, EventReporting, Timed};
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use tokio::time::Instant;

/// Status_Flags IN_ALARM and FAULT, as the wire's first octet carries them.
const IN_ALARM: u8 = 0x80;
const FAULTED: u8 = 0x40;
const SCHEDULE_DEFAULT: PropertyIdentifier = PropertyIdentifier::SCHEDULE_DEFAULT;

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

fn sch1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap()
}

fn fault_detection() -> ServerConfig {
    ServerConfig {
        enable_fault_detection: true,
        ..ServerConfig::default()
    }
}

/// AV-`instance`, faulting outside 0 to 100 by its own reliability
/// evaluation, reporting every transition through Notification Class
/// `class`.
fn fault_value(db: &mut ObjectDatabase, instance: u32, class: u64) {
    let name = format!("AV-{instance}");
    let mut value =
        AnalogValueObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap();
    value.configure_fault_out_of_range(0.0, 100.0).unwrap();
    let enable = all_transitions();
    value
        .write_property(PropertyIdentifier::EVENT_ENABLE, None, enable, None)
        .unwrap();
    let class = PropertyValue::Unsigned(class);
    value
        .write_property(PropertyIdentifier::NOTIFICATION_CLASS, None, class, None)
        .unwrap();
    db.add(Box::new(value)).unwrap();
}

/// BV-1 reporting every transition, and SCH-1 commanding its Present_Value
/// at priority 16, inactive unless `active`.
fn scheduled(db: &mut ObjectDatabase, active: bool) {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let enable = all_transitions();
    bv.write_property(PropertyIdentifier::EVENT_ENABLE, None, enable, None)
        .unwrap();
    db.add(Box::new(bv)).unwrap();
    let default = PropertyValue::Enumerated(u32::from(active));
    let mut schedule = ScheduleObject::new(1, "SCH-1", default).unwrap();
    let reference = BACnetObjectPropertyReference::new(bv1(), PV.to_raw());
    schedule.add_object_property_reference(reference).unwrap();
    db.add(Box::new(schedule)).unwrap();
}

/// [`SUBSCRIBER`] watches every COV-reported property of `oid`.
async fn watch(rig: &Rig, oid: ObjectIdentifier) {
    let mut table = rig.server.cov_table.write().await;
    table
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&SUBSCRIBER),
            subscriber_network: None,
            subscriber_process_identifier: oid.instance_number(),
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
}

/// Set `oid`'s Present_Value at priority 16 as its application would, unseen
/// by any write path.
async fn set_unseen(rig: &Rig, oid: ObjectIdentifier, value: PropertyValue) {
    use bacnet_device_core::write::local::resolve_source;
    let mut db = rig.server.database().write().await;
    let origin = resolve_source(&db, crate::LocalCommandSource::ServerDevice).unwrap();
    let object = db.get_mut(&oid).unwrap();
    object
        .write_property_from(PV, None, value, Some(16), &origin)
        .unwrap();
}

/// What went out, in order: when (ms after `t0`), what, and for a COV
/// report its Status_Flags.
fn stamped(rig: &Rig, t0: Instant) -> Vec<(u64, Sent, u8)> {
    let stamped = rig.probe.stamped.lock().unwrap();
    let millis = |at: Instant| u64::try_from(at.duration_since(t0).as_millis()).unwrap();
    stamped
        .iter()
        .map(|(at, sent, flags)| (millis(*at), sent.clone(), *flags))
        .collect()
}

/// A COV report of `oid` carrying `flags`, at `millis`.
fn cov(millis: u64, oid: ObjectIdentifier, flags: u8) -> (u64, Sent, u8) {
    (millis, Sent::Cov(oid), flags)
}

/// A broadcast transition of `oid` to `to`, at `millis`.
fn event(millis: u64, oid: ObjectIdentifier, to: u32) -> (u64, Sent, u8) {
    (millis, ev(oid, to), 0)
}

async fn sleep_until(t0: Instant, millis: u64) {
    tokio::time::sleep_until(t0 + Duration::from_millis(millis)).await;
}

/// Move the sweeps half a second off the whole seconds the fault-detection
/// passes and the Schedule ticks fall on: called just after the rig starts,
/// the clock jumps past the 1 s sweep, which runs late, at 1.5 s, and keeps
/// that phase. A transition left to a sweep then shows half a second later,
/// whichever task runs first at an instant they share.
async fn shift_sweeps() {
    tokio::time::advance(Duration::from_millis(1_300)).await;
}

/// Whether the transport holds a send within `millis`, on the paused clock.
async fn held_within(rig: &Rig, millis: u64) -> bool {
    let held = rig.probe.held.acquire();
    match tokio::time::timeout(Duration::from_millis(millis), held).await {
        Ok(permit) => {
            permit.unwrap().forget();
            true
        }
        Err(_) => false,
    }
}

/// The fault-detection pass at 10 s finds AV-1 out of range and commits its
/// FAULT with the new Reliability: the notification goes out then, and the
/// one COV report carries FAULT and IN_ALARM together. The pass at 20 s
/// clears both together. The sweeps after each find nothing to send.
#[tokio::test(start_paused = true)]
async fn the_fault_task_commits_the_fault_and_its_return_with_the_reliability() {
    let t0 = Instant::now();
    let rig = Rig::start_with(fault_detection(), |db| fault_value(db, 1, 0)).await;
    shift_sweeps().await;
    watch(&rig, av(1)).await;
    set_unseen(&rig, av(1), PropertyValue::Real(150.0)).await;
    sleep_until(t0, 10_001).await;
    let faulted = [
        event(10_000, av(1), FAULT),
        cov(10_000, av(1), IN_ALARM | FAULTED),
    ];
    assert_eq!(stamped(&rig, t0), faulted);
    set_unseen(&rig, av(1), PropertyValue::Real(50.0)).await;
    sleep_until(t0, 20_001).await;
    let cleared = [event(20_000, av(1), NORMAL), cov(20_000, av(1), 0)];
    assert_eq!(stamped(&rig, t0)[2..], cleared);
    sleep_until(t0, 23_000).await;
    assert_eq!(stamped(&rig, t0).len(), 4, "nothing more");
}

/// A peer's write committed after the fault-detection pass's is told after
/// it: the pass's FAULT is held in its lane, the peer inhibits AV-1's
/// reliability evaluation, which returns it to NORMAL, and that waits behind
/// the FAULT.
#[tokio::test(start_paused = true)]
async fn a_peer_write_after_the_fault_task_is_told_after_its_fault() {
    let t0 = Instant::now();
    let rig = Rig::start_with(fault_detection(), |db| fault_value(db, 1, 0)).await;
    shift_sweeps().await;
    set_unseen(&rig, av(1), PropertyValue::Real(150.0)).await;
    rig.hold(true, FAULT);
    sleep_until(t0, 10_000).await;
    assert!(held_within(&rig, 1).await, "the pass's FAULT, at once");
    let inhibit = PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT;
    let id = rig
        .write(av(1), inhibit, PropertyValue::Boolean(true))
        .await;
    rig.acked(id).await;
    assert_eq!(rig.event_state(av(1)).await, NORMAL);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.probe.wire(), [], "the return waits behind the fault");
    rig.release();
    assert_eq!(
        rig.wait_wire(2).await,
        [ev(av(1), FAULT), ev(av(1), NORMAL)]
    );
}

/// AV-1 faults to Device 9, which is unbound and never answers, so the
/// 10 s pass's COV fanout waits in the task's tail (here up to 30 s). That
/// holds up neither the 20 s pass, whose AV-2 fault goes out at 20 s, nor
/// the sweep, whose AV-3 alarm goes out at 14.5 s (a sweep that finds a
/// condition no write told it of fires it at the next one).
#[tokio::test(start_paused = true)]
async fn a_silent_device_recipient_holds_up_neither_the_next_pass_nor_the_sweep() {
    let t0 = Instant::now();
    let config = ServerConfig {
        cov_retry_timeout_ms: 30_000,
        ..fault_detection()
    };
    let rig = Rig::start_with(config, |db| {
        let device_9 = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
        let mut nc1 = NotificationClass::new(1, "NC-1").unwrap();
        let to_device = destination_for(BACnetRecipient::Device(device_9), false);
        nc1.add_destination(to_device).unwrap();
        db.add(Box::new(nc1)).unwrap();
        fault_value(db, 1, 1);
        fault_value(db, 2, 0);
        // AV-3 alarms over 80, which the sweep finds.
        fault_value(db, 3, 0);
        let av3 = db.get_mut(&av(3)).unwrap();
        let high = PropertyValue::Real(80.0);
        av3.write_property(PropertyIdentifier::HIGH_LIMIT, None, high, None)
            .unwrap();
        let enable = PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0x40],
        };
        av3.write_property(PropertyIdentifier::LIMIT_ENABLE, None, enable, None)
            .unwrap();
    })
    .await;
    shift_sweeps().await;
    set_unseen(&rig, av(1), PropertyValue::Real(150.0)).await;
    sleep_until(t0, 12_700).await;
    set_unseen(&rig, av(3), PropertyValue::Real(90.0)).await;
    sleep_until(t0, 15_000).await;
    set_unseen(&rig, av(2), PropertyValue::Real(150.0)).await;
    sleep_until(t0, 20_001).await;
    let events: Vec<_> = stamped(&rig, t0)
        .into_iter()
        .filter(|(_, sent, _)| matches!(sent, Sent::Event { .. }))
        .collect();
    let high_limit = 3;
    assert_eq!(
        events,
        [
            event(14_500, av(3), high_limit),
            event(20_000, av(2), FAULT)
        ]
    );
}

/// `stop()` while the fault-detection task's tail waits on a held lane
/// returns, and nothing goes out after it: the held FAULT is dropped with
/// its lane.
#[tokio::test(start_paused = true)]
async fn stop_while_the_fault_tasks_tail_waits_returns() {
    let t0 = Instant::now();
    let mut rig = Rig::start_with(fault_detection(), |db| fault_value(db, 1, 0)).await;
    shift_sweeps().await;
    watch(&rig, av(1)).await;
    set_unseen(&rig, av(1), PropertyValue::Real(150.0)).await;
    rig.hold(true, FAULT);
    sleep_until(t0, 10_000).await;
    assert!(held_within(&rig, 1).await);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stopped = tokio::time::timeout(Duration::from_secs(30), rig.server.stop());
    stopped.await.expect("stop returns").unwrap();
    let sent = rig.probe.wire();
    rig.release();
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(rig.probe.wire(), sent, "nothing sent after stop");
    assert!(!sent.contains(&ev(av(1), FAULT)), "{sent:?}");
}

/// A local write of SCH-1's Schedule_Default runs its pass at once, and the
/// pass's write of BV-1 commits BV-1's OFFNORMAL with it: the notification
/// goes out at the write, and BV-1's report carries IN_ALARM with the new
/// value. The sweeps after it send nothing more.
#[tokio::test(start_paused = true)]
async fn a_schedule_written_locally_commits_its_targets_transition_at_once() {
    let t0 = Instant::now();
    let rig = Rig::start(|db| scheduled(db, false)).await;
    watch(&rig, bv1()).await;
    sleep_until(t0, 500).await;
    let (schedule, active) = (sch1(), PropertyValue::Enumerated(1));
    let source = crate::LocalCommandSource::ServerDevice;
    let write = rig
        .server
        .write_local(&schedule, SCHEDULE_DEFAULT, None, active, None, source);
    write.await.unwrap();
    let alarmed = [event(500, bv1(), OFFNORMAL), cov(500, bv1(), IN_ALARM)];
    assert_eq!(stamped(&rig, t0), alarmed);
    sleep_until(t0, 3_000).await;
    assert_eq!(stamped(&rig, t0).len(), 2, "nothing more");
}

/// A peer's WriteProperty of SCH-1 commits the transition its pass causes on
/// BV-1 under the request's guard, and a second peer write that returns
/// BV-1 to NORMAL before any sweep is told after it, while the first is
/// held. Left to the sweep, the alarm would never be told.
#[tokio::test(start_paused = true)]
async fn a_peer_write_after_a_schedules_pass_is_told_after_its_transition() {
    let rig = Rig::start(|db| scheduled(db, false)).await;
    rig.hold(true, OFFNORMAL);
    let id = rig
        .write(sch1(), SCHEDULE_DEFAULT, PropertyValue::Enumerated(1))
        .await;
    rig.acked(id).await;
    assert!(held_within(&rig, 1).await, "the pass's OFFNORMAL, at once");
    let id = rig.write(bv1(), PV, PropertyValue::Enumerated(0)).await;
    rig.acked(id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.probe.wire(), [], "the return waits behind the alarm");
    rig.release();
    assert_eq!(
        rig.wait_wire(2).await,
        [ev(bv1(), OFFNORMAL), ev(bv1(), NORMAL)]
    );
}

/// The 60-second Schedule tick writes BV-1 active and commits its OFFNORMAL
/// in the same pass: one COV report, IN_ALARM with the value, and nothing
/// more from the sweeps after it.
#[tokio::test(start_paused = true)]
async fn the_schedule_tick_commits_its_targets_transition() {
    let t0 = Instant::now();
    let rig = Rig::start(|db| scheduled(db, false)).await;
    shift_sweeps().await;
    watch(&rig, bv1()).await;
    {
        let mut db = rig.server.database().write().await;
        let schedule = db.get_mut(&sch1()).unwrap();
        let active = PropertyValue::Enumerated(1);
        schedule
            .write_property(SCHEDULE_DEFAULT, None, active, None)
            .unwrap();
    }
    sleep_until(t0, 60_001).await;
    let alarmed = [
        event(60_000, bv1(), OFFNORMAL),
        cov(60_000, bv1(), IN_ALARM),
    ];
    assert_eq!(stamped(&rig, t0), alarmed);
    sleep_until(t0, 63_000).await;
    assert_eq!(stamped(&rig, t0).len(), 2, "nothing more");
}

/// The application removes SCH-1, which held BV-1 active: the Schedule
/// task gives up its slot when the removal wakes it, and BV-1's return to
/// NORMAL commits with that write, its report with flags clear.
#[tokio::test(start_paused = true)]
async fn an_applications_removal_commits_the_relinquished_targets_return() {
    let t0 = Instant::now();
    let rig = Rig::start(|db| scheduled(db, true)).await;
    assert_eq!(rig.event_state(bv1()).await, OFFNORMAL, "the first tick");
    let before = stamped(&rig, t0).len();
    watch(&rig, bv1()).await;
    sleep_until(t0, 500).await;
    let removed = rig.server.database().write().await.remove(&sch1());
    drop(removed.unwrap());
    sleep_until(t0, 501).await;
    let returned = [event(500, bv1(), NORMAL), cov(500, bv1(), 0)];
    assert_eq!(stamped(&rig, t0)[before..], returned);
    sleep_until(t0, 3_000).await;
    assert_eq!(stamped(&rig, t0).len(), before + 2, "nothing more");
}

/// A peer's DeleteObject of SCH-1, which held BV-1 active, commits BV-1's
/// return to NORMAL with the relinquish its removal makes.
#[tokio::test(start_paused = true)]
async fn a_delete_object_commits_the_relinquished_targets_return() {
    let t0 = Instant::now();
    let rig = Rig::start(|db| scheduled(db, true)).await;
    let before = stamped(&rig, t0).len();
    watch(&rig, bv1()).await;
    sleep_until(t0, 500).await;
    let mut body = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: sch1(),
    }
    .encode(&mut body);
    let id = rig
        .request(ConfirmedServiceChoice::DELETE_OBJECT, body)
        .await;
    rig.acked(id).await;
    sleep_until(t0, 501).await;
    let returned = [event(500, bv1(), NORMAL), cov(500, bv1(), 0)];
    assert_eq!(stamped(&rig, t0)[before..], returned);
    sleep_until(t0, 3_000).await;
    assert_eq!(stamped(&rig, t0).len(), before + 2, "nothing more");
}

/// What a [`Pulsing`] value reports from a frozen copy: its Present_Value
/// and Status_Flags when the copy was taken.
struct Frozen {
    oid: ObjectIdentifier,
    present_value: PropertyValue,
    status_flags: PropertyValue,
}

impl BACnetObject for Frozen {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        "BV-1"
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::PRESENT_VALUE => Ok(self.present_value.clone()),
            PropertyIdentifier::STATUS_FLAGS => Ok(self.status_flags.clone()),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(&self) -> std::borrow::Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[PV, PropertyIdentifier::STATUS_FLAGS])
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        Some(self)
    }
}

impl CovReporting for Frozen {}

/// BV-1 driven by its own deadline, as an application's timed object: the
/// operation task's first pass arms it 1.5 s on, and the pass then turns
/// the value active. With `copies`, it reports that from a frozen copy.
struct Pulsing {
    value: BinaryValueObject,
    /// The local Device, which its commands name as their source.
    origin: bacnet_objects::command_source::CommandOrigin,
    due: Option<Duration>,
    armed: bool,
    copies: bool,
}

impl BACnetObject for Pulsing {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.value.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.value.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.value.read_property(property, array_index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.value
            .write_property(property, array_index, value, priority)
    }
    fn property_metadata(&self) -> std::borrow::Cow<'_, [PropertyMetadata]> {
        self.value.property_metadata()
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        self.value.cov()
    }
    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        self.value.event_reporting()
    }
    fn event_reporting_mut(&mut self) -> Option<&mut dyn EventReporting> {
        self.value.event_reporting_mut()
    }
    fn timed(&mut self) -> Option<&mut dyn Timed> {
        Some(self)
    }
}

impl Timed for Pulsing {
    fn advance_monotonic_time(&mut self, now: Duration) -> bool {
        if !self.armed {
            self.armed = true;
            self.due = Some(now + Duration::from_millis(1_500));
            return false;
        }
        if self.due.is_none_or(|due| now < due) {
            return false;
        }
        self.due = None;
        let active = PropertyValue::Enumerated(1);
        let origin = &self.origin;
        let written = self
            .value
            .write_property_from(PV, None, active, Some(16), origin);
        written.unwrap();
        true
    }
    fn next_monotonic_deadline(&self) -> Option<Duration> {
        self.due
    }
    fn cov_snapshot(&self) -> Option<Box<dyn BACnetObject>> {
        let read = |property| self.value.read_property(property, None).unwrap();
        self.copies.then(|| {
            Box::new(Frozen {
                oid: self.object_identifier(),
                present_value: read(PV),
                status_flags: read(PropertyIdentifier::STATUS_FLAGS),
            }) as Box<dyn BACnetObject>
        })
    }
}

/// The operation task turns BV-1 active at 1.5 s, between sweeps, and
/// commits its OFFNORMAL in the same pass: the notification goes then, and
/// the report, from a frozen copy taken after the commit or from the
/// database, carries IN_ALARM with the value.
async fn the_operation_task_commits_a_timed_objects_transition(copies: bool) {
    let t0 = Instant::now();
    let rig = Rig::start(|db| {
        let mut value = BinaryValueObject::new(1, "BV-1").unwrap();
        let enable = all_transitions();
        value
            .write_property(PropertyIdentifier::EVENT_ENABLE, None, enable, None)
            .unwrap();
        let source = crate::LocalCommandSource::ServerDevice;
        let pulsing = Pulsing {
            value,
            origin: bacnet_device_core::write::local::resolve_source(db, source).unwrap(),
            due: None,
            armed: false,
            copies,
        };
        db.add(Box::new(pulsing)).unwrap();
    })
    .await;
    watch(&rig, bv1()).await;
    sleep_until(t0, 1_501).await;
    let alarmed = [event(1_500, bv1(), OFFNORMAL), cov(1_500, bv1(), IN_ALARM)];
    assert_eq!(stamped(&rig, t0), alarmed);
    sleep_until(t0, 4_000).await;
    assert_eq!(stamped(&rig, t0).len(), 2, "nothing more");
}

#[tokio::test(start_paused = true)]
async fn the_operation_task_commits_a_timed_objects_transition_reported_from_a_copy() {
    the_operation_task_commits_a_timed_objects_transition(true).await;
}

#[tokio::test(start_paused = true)]
async fn the_operation_task_commits_a_timed_objects_transition_reported_from_the_database() {
    the_operation_task_commits_a_timed_objects_transition(false).await;
}

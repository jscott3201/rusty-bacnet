//! A pass with nothing else to send for a Schedule offers its value again to
//! the references that refused it (#1436), so a configuration fault clears
//! once its cause is gone, though the Schedule's value never changes. The
//! retry goes to those references alone, so a Command or Channel that took
//! the value is not run again. A target that still refuses keeps the fault
//! and logs nothing above debug; one that now fails otherwise ends it and
//! warns once, as it would have from the start.
//!
//! The databases come from `schedule_reference_reliability_tests`: Monday
//! 5 October 2026, 09:00, inside every Schedule's period. The clock is
//! paused.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectPropertyReference,
};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

use super::reference_reliability_tests::{
    database, health, indexed, oid, read, reference, FAULTED, HEALTHY,
};
use super::tests::SettableClock;
use super::*;

use bacnet_types::enums::EngineeringUnits;
use PropertyIdentifier as P;

fn msv(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_VALUE, instance)
}

fn ao(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ANALOG_OUTPUT, instance)
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

#[tokio::test(start_paused = true)]
async fn a_state_text_index_that_becomes_valid_clears_the_fault() {
    // MSV-1 has three states, so State_Text[5] is past the end.
    let db = database(text("Away"), vec![indexed(msv(1), P::STATE_TEXT, 5)]);
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED);

    // The application raises Number_Of_States to five, putting MSV-1 back
    // with the longer State_Text.
    db.write()
        .await
        .add(Box::new(MultiStateValueObject::new(1, "MSV-1", 5).unwrap()))
        .unwrap();
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
    let db = db.read().await;
    let state_text = db
        .get(&msv(1))
        .unwrap()
        .read_property(P::STATE_TEXT, Some(5))
        .unwrap();
    assert_eq!(state_text, text("Away"));
}

/// SCH-1 defaults to Unsigned 1 and writes CMD-1, CH-1 and MSV-9, which
/// doesn't exist yet. CMD-1's list 1 writes 50.0 to AO-1 at priority 8;
/// CH-1 passes its value to AO-2, at the priority it was written with (16).
fn command_and_channel() -> Arc<RwLock<ObjectDatabase>> {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(SettableClock::at(2026, 10, 5, 9, 0)));
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    for instance in [1, 2] {
        db.add(Box::new(
            AnalogOutputObject::new(
                instance,
                format!("AO-{instance}"),
                EngineeringUnits::DEGREES_CELSIUS,
            )
            .unwrap(),
        ))
        .unwrap();
    }
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![BACnetActionCommand {
                device_identifier: None,
                object_identifier: ao(1),
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
                property_value: PropertyValue::Real(50.0),
                priority: Some(8),
                post_delay: None,
                quit_on_failure: false,
                write_successful: true,
            }],
        }])
        .unwrap();
    db.add(Box::new(command)).unwrap();
    let mut channel = ChannelObject::new(1, "CH-1", 1).unwrap();
    channel
        .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
            ao(2),
            P::PRESENT_VALUE.to_raw(),
        )])
        .unwrap();
    channel.set_execution_delay(vec![0]).unwrap();
    db.add(Box::new(channel)).unwrap();
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Unsigned(1)).unwrap();
    schedule
        .set_object_property_references(vec![
            reference(oid(ObjectType::COMMAND, 1), P::PRESENT_VALUE),
            reference(oid(ObjectType::CHANNEL, 1), P::PRESENT_VALUE),
            reference(msv(9), P::PRESENT_VALUE),
        ])
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
    Arc::new(RwLock::new(db))
}

/// AO-1's priority 8 slot and AO-2's priority 16 slot.
async fn slots(db: &RwLock<ObjectDatabase>) -> [PropertyValue; 2] {
    let db = db.read().await;
    let slot = |object, priority| {
        db.get(&object)
            .unwrap()
            .read_property(P::PRIORITY_ARRAY, Some(priority))
            .unwrap()
    };
    [slot(ao(1), 8), slot(ao(2), 16)]
}

#[tokio::test(start_paused = true)]
async fn a_retry_runs_no_command_or_channel_member_again() {
    let db = command_and_channel();
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED, "MSV-9 is missing");
    assert_eq!(
        slots(&db).await,
        [PropertyValue::Real(50.0), PropertyValue::Real(1.0)]
    );

    // Something else takes over both slots. A Command or Channel run again
    // would put its value back.
    {
        let mut db = db.write().await;
        for (object, priority) in [(ao(1), 8), (ao(2), 16)] {
            db.get_mut(&object)
                .unwrap()
                .write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Real(7.0),
                    Some(priority),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
        }
    }
    let taken_over = [PropertyValue::Real(7.0), PropertyValue::Real(7.0)];

    // The retries go to MSV-9 alone, before and after it is created.
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED, "MSV-9 still missing");
    assert_eq!(slots(&db).await, taken_over);
    db.write()
        .await
        .add(Box::new(MultiStateValueObject::new(9, "MSV-9", 3).unwrap()))
        .unwrap();
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
    assert_eq!(
        read(&*db.read().await, msv(9), P::PRESENT_VALUE),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(slots(&db).await, taken_over);
}

/// Counts this module's events on the thread it is installed on: those at
/// debug, and those above it.
#[derive(Clone, Default)]
struct ScheduleLog {
    debug: Arc<AtomicUsize>,
    above_debug: Arc<AtomicUsize>,
}

impl ScheduleLog {
    /// Count until the returned guard drops. Under the test's current-thread
    /// runtime that covers the whole pass.
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        // As in the DCC trace tests: keep a second, disabled dispatcher
        // alive so a callsite another test registered first, with no
        // subscriber of its own, is not cached as never enabled.
        static REGISTRATION_PEER: OnceLock<tracing::Dispatch> = OnceLock::new();
        REGISTRATION_PEER
            .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        tracing::subscriber::set_default(self.clone())
    }

    /// `(debug, above debug)` counted since the last call.
    fn take(&self) -> (usize, usize) {
        (
            self.debug.swap(0, Ordering::SeqCst),
            self.above_debug.swap(0, Ordering::SeqCst),
        )
    }
}

impl Subscriber for ScheduleLog {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::DEBUG)
    }
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "bacnet_server::schedule"
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        match *event.metadata().level() {
            Level::DEBUG => self.debug.fetch_add(1, Ordering::SeqCst),
            Level::TRACE => 0,
            _ => self.above_debug.fetch_add(1, Ordering::SeqCst),
        };
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

#[tokio::test(start_paused = true)]
async fn a_reference_still_refused_keeps_the_fault_and_logs_only_at_debug() {
    let log = ScheduleLog::default();
    let _guard = log.install();
    // MSV-1 has three states, so State_Text[9] stays past the end.
    let db = database(text("Away"), vec![indexed(msv(1), P::STATE_TEXT, 9)]);
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED);
    let (_, warnings) = log.take();
    assert_eq!(warnings, 1, "the first refusal warns");

    for _ in 0..3 {
        tick_schedules(&db).await;
        assert_eq!(health(&db).await, FAULTED);
        let (debug, above_debug) = log.take();
        assert_eq!(above_debug, 0, "a retry logs nothing above debug");
        assert!(debug >= 2, "the retry and its refusal log at debug");
    }
}

#[tokio::test(start_paused = true)]
async fn a_retry_that_fails_otherwise_ends_the_fault_and_warns_once() {
    let log = ScheduleLog::default();
    let _guard = log.install();
    let five = || PropertyValue::Unsigned(5);
    let msv9 = || Box::new(MultiStateValueObject::new(9, "MSV-9", 3).unwrap());

    // With MSV-9 there from the start, state 5 of 3 is out of range: a
    // failure, not a configuration fault, and it warns.
    let db = database(five(), vec![reference(msv(9), P::PRESENT_VALUE)]);
    db.write().await.add(msv9()).unwrap();
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
    assert_eq!(log.take().1, 1);

    // Created after the Schedule found it missing, MSV-9 ends the same way.
    let db = database(five(), vec![reference(msv(9), P::PRESENT_VALUE)]);
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED, "MSV-9 is missing");
    log.take();
    db.write().await.add(msv9()).unwrap();
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
    assert_eq!(log.take().1, 1, "the retry's new failure warns once");
    assert_eq!(
        read(&*db.read().await, msv(9), P::PRESENT_VALUE),
        PropertyValue::Unsigned(1)
    );
    // MSV-9 no longer refuses, so no retry follows, and nothing is logged.
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
    assert_eq!(log.take(), (0, 0));
}

//! A `write_local` dropped once its write has committed still does all the
//! write owes (#1367). The commit hands its database guard, and what the
//! write owes, to a task of its own in the request task set, which the
//! caller only waits for: a caller that drops the future (a timeout, a
//! `select!`, a cancelled Python task) skips none of it. Subscribers hear of
//! the change, and a Command or Channel run the write started goes ahead
//! and reports its end. `stop()` aborts that task with the other request
//! tasks, so a run it hadn't started ends as if none of its writes were made
//! (#1324). `set_life_safety_operation_expected_local` hands its
//! timestamped capture and COV fanout to such a task the same way (#1520).
//!
//! Each test holds the COV table for writing, so the work after the commit
//! parks on it, at the timestamped capture under the write's guard; polls the
//! write once by hand, which commits it; and drops it there. The objects are
//! those of `command_run_stop_tests`: CMD-1's list 2 writes AO-2 with a
//! 5-second post delay, then AO-1, and CH-5 writes AO-2 200 ms after its
//! Present_Value is written. The clock is paused.
use super::channel_wire_tests::ch;
use super::command_action_wire_tests::{ao, cmd, idle, read_db, slot8};
use super::command_run_stop_tests::{channel_status, db_flags, db_state, objects, with_channel};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_types::enums::WriteStatus;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

type Write<'a> = Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>>;

/// `write_local` of `value` to `object`'s Present_Value, from the Device.
fn write_local<'a>(
    h: &'a Harness,
    object: &'a ObjectIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) -> Write<'a> {
    Box::pin(h.server.write_local(
        object,
        PV,
        None,
        value,
        priority,
        crate::LocalCommandSource::ServerDevice,
    ))
}

/// Commit `write`, polled once, and drop it at its first wait, the work it
/// owes parked on the COV table `h`'s caller holds.
fn commit_and_drop(h: &Harness, mut write: Write<'_>) {
    assert!(write
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    // Committed: the database is held for the work after the commit.
    assert!(h.server.database().try_read().is_err());
    drop(write);
}

/// SubscribeCOVProperty from process 7 to CMD-1's In_Process, and its first
/// notification's value.
async fn subscribe_in_process(h: &mut Harness) -> Vec<u8> {
    let mut body = BytesMut::new();
    bacnet_services::cov::SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 7,
        monitored_object_identifier: cmd(1),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: PropertyIdentifier::IN_PROCESS,
        monitored_property_array_index: None,
        cov_increment: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, body)
        .await;
    response(h).await.unwrap();
    in_process(h).await
}

/// The value of the next In_Process notification: `[0x11]` for TRUE,
/// `[0x10]` for FALSE.
async fn in_process(h: &Harness) -> Vec<u8> {
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, cmd(1));
    notification.list_of_values[0].value.clone()
}

#[tokio::test(start_paused = true)]
async fn write_local_dropped_after_its_commit_still_notifies_cov_subscribers() {
    let mut h = Harness::start(ServerConfig::default()).await;
    h.subscribe_cov().await;
    h.cov_notification().await;
    let table = Arc::clone(&h.server.cov_table).write_owned().await;
    let target = av1();
    commit_and_drop(
        &h,
        write_local(&h, &target, PropertyValue::Real(70.0), None),
    );

    drop(table);
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, av1());
    assert_eq!(notification.list_of_values[0].property_identifier, PV);
    assert_eq!(notification.list_of_values[0].value, real(70.0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn write_local_dropped_after_its_commit_still_runs_the_command_and_reports_its_end() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    assert_eq!(subscribe_in_process(&mut h).await, [0x10]);
    let table = Arc::clone(&h.server.cov_table).write_owned().await;
    let target = cmd(1);
    commit_and_drop(
        &h,
        write_local(&h, &target, PropertyValue::Unsigned(2), None),
    );

    drop(table);
    idle(&h, 1).await;
    // The subscriber hears the run start and, after its post delay, end,
    // with every write made.
    assert_eq!(in_process(&h).await, [0x11]);
    assert_eq!(in_process(&h).await, [0x10]);
    assert_eq!(db_state(&h, 1).await, (false, true));
    assert_eq!(db_flags(&h, 2).await, [true, true]);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(30.0));
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(70.0));
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn write_local_dropped_after_its_commit_still_runs_the_distribution() {
    let h = Harness::start_with(ServerConfig::default(), with_channel).await;
    let table = Arc::clone(&h.server.cov_table).write_owned().await;
    let target = ch(5);
    commit_and_drop(
        &h,
        write_local(&h, &target, PropertyValue::Real(12.0), Some(8)),
    );

    drop(table);
    assert_eq!(channel_status(&h).await, WriteStatus::IN_PROGRESS);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(channel_status(&h).await, WriteStatus::SUCCESSFUL);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(12.0));
}

#[tokio::test(start_paused = true)]
async fn stop_aborts_the_work_a_committed_write_owes_and_ends_its_run() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    let table = Arc::clone(&h.server.cov_table).write_owned().await;
    let target = cmd(1);
    commit_and_drop(
        &h,
        write_local(&h, &target, PropertyValue::Unsigned(2), None),
    );
    drop(table);
    assert!(
        !h.server.request_tasks.is_empty(),
        "the work the write owes is a request task"
    );

    // `stop()` closes the request task set before anything else, so the
    // task never runs: it is aborted as a request handler would be, and the
    // run it held ends unsuccessful with none of its writes made.
    h.server.stop().await.unwrap();
    assert_eq!(db_state(&h, 1).await, (false, false));
    assert_eq!(db_flags(&h, 2).await, [false, false]);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
    assert_eq!(
        read_db(&h, cmd(1), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Unsigned(2)
    );
}

/// An object whose writes succeed and whose event evaluation, which the work
/// after the commit runs, panics.
struct PanicsAfterCommit;

impl bacnet_objects::traits::BACnetObject for PanicsAfterCommit {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap()
    }

    fn object_name(&self) -> &str {
        "AFTER-99"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => {
                PropertyValue::ObjectIdentifier(self.object_identifier())
            }
            PropertyIdentifier::OBJECT_NAME => PropertyValue::CharacterString("AFTER-99".into()),
            _ => PropertyValue::Real(0.0),
        })
    }

    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        std::borrow::Cow::Borrowed(&[])
    }

    fn evaluate_intrinsic_reporting(&mut self) -> Option<bacnet_objects::event::TransitionOutcome> {
        panic!("AFTER-99 panics in the work after its write");
    }
}

#[tokio::test(start_paused = true)]
async fn a_panic_in_the_work_after_the_commit_reaches_the_caller() {
    use futures_util::FutureExt;
    let h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(PanicsAfterCommit)).unwrap();
    })
    .await;
    let target = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap();
    let write = write_local(&h, &target, PropertyValue::Real(1.0), None);
    let panicked = std::panic::AssertUnwindSafe(write).catch_unwind().await;
    assert!(
        panicked.is_err(),
        "the caller sees the panic, as it would were the work done in place"
    );
}

#[tokio::test(start_paused = true)]
async fn a_write_whose_work_is_aborted_returns_ok_and_stays_made() {
    let h = Harness::start(ServerConfig::default()).await;
    let table = Arc::clone(&h.server.cov_table).write_owned().await;
    let target = av1();
    let mut write = write_local(&h, &target, PropertyValue::Real(70.0), None);
    assert!(write
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
        .is_pending());
    // What `stop()` does first, while the caller still waits: the request
    // task set closes, aborting the work the committed write owes. (`stop()`
    // itself takes the server mutably, so it can't run beside the caller's
    // borrow here.)
    h.server.request_tasks.close();
    drop(table);
    assert!(matches!(write.await, Ok(())), "the write was made");
    assert_eq!(
        read_db(&h, av1(), PV, None).await,
        PropertyValue::Real(70.0)
    );
}

#[tokio::test(start_paused = true)]
async fn a_panic_after_the_caller_went_away_leaves_the_server_working() {
    let h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(PanicsAfterCommit)).unwrap();
    })
    .await;
    let target = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap();
    commit_and_drop(&h, write_local(&h, &target, PropertyValue::Real(1.0), None));
    // The work panics with nobody to raise it to: it is logged as this
    // write's, and goes no further.
    h.settle().await;
    let next = av1();
    write_local(&h, &next, PropertyValue::Real(42.0), None)
        .await
        .unwrap();
    assert_eq!(
        read_db(&h, av1(), PV, None).await,
        PropertyValue::Real(42.0)
    );
}

#[test]
fn a_local_write_outside_a_tokio_runtime_fails_and_writes_nothing() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut h = runtime.block_on(Harness::start(ServerConfig::default()));
    let target = av1();
    let before = runtime.block_on(read_db(&h, target, PV, None));
    // Polled with no runtime entered, as another executor would poll it.
    let mut write = write_local(&h, &target, PropertyValue::Real(70.0), None);
    let polled = write.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    assert!(
        matches!(polled, Poll::Ready(Err(Error::Encoding(_)))),
        "{polled:?}"
    );
    drop(write);
    assert_eq!(runtime.block_on(read_db(&h, target, PV, None)), before);
    runtime.block_on(h.server.stop()).unwrap();
}

/// LSP-1, whose Operation_Expected the application rearms.
fn life_safety_point(db: &mut ObjectDatabase) {
    db.add(Box::new(
        bacnet_objects::life_safety::LifeSafetyPointObject::new(1, "LSP-1").unwrap(),
    ))
    .unwrap();
}

fn lsp1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

/// `set_life_safety_operation_expected_local` of SILENCE on `point`.
fn silence<'a>(h: &'a Harness, point: &'a ObjectIdentifier) -> Write<'a> {
    Box::pin(
        h.server
            .set_life_safety_operation_expected_local(point, LifeSafetyOperation::SILENCE),
    )
}

#[tokio::test(start_paused = true)]
async fn life_safety_rearm_dropped_after_its_change_still_notifies_operation_expected() {
    let mut h = Harness::start_with(ServerConfig::default(), life_safety_point).await;
    let expected = PropertyIdentifier::OPERATION_EXPECTED;
    h.subscribe_specs(false, vec![(lsp1(), vec![(expected, true)])])
        .await;
    h.notification().await;
    h.set_clock(41);
    let table = Arc::clone(&h.server.cov_table).write_owned().await;
    // Changed, then dropped while the caller waits for the work it owes.
    // With that work in the caller's future, the held table would park the
    // timestamped capture there and the drop would skip it and the fanout;
    // in its own task, the work runs on once the table is free.
    let point = lsp1();
    commit_and_drop(&h, silence(&h, &point));

    drop(table);
    let report = h.notification().await;
    let rows: Vec<_> = report.list_of_cov_notifications[0]
        .list_of_values
        .iter()
        .filter(|row| row.property_identifier == expected)
        .collect();
    let mut value = BytesMut::new();
    let silence = PropertyValue::Enumerated(LifeSafetyOperation::SILENCE.to_raw());
    encode_property_value(&mut value, &silence).unwrap();
    assert_eq!(rows.len(), 1, "{report:?}");
    assert_eq!(rows[0].value, value.to_vec());
    assert_eq!(rows[0].time_of_change, Some(time(41)), "{report:?}");
    h.server.stop().await.unwrap();
}

/// AV-2, a custom object that takes a Life Safety rearm by changing its
/// Description, though it isn't a Life Safety object.
struct Rearmable(bacnet_objects::analog::AnalogValueObject);

impl bacnet_objects::traits::BACnetObject for Rearmable {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.0.object_name()
    }

    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        self.0.read_property(p, i)
    }

    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0.write_property(p, i, value, priority)
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        self.0.property_list()
    }

    fn supports_cov(&self) -> bool {
        true
    }

    fn set_life_safety_operation_expected_internal(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        self.0
            .set_description(format!("rearmed {}", operation.to_raw()));
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_rearm_of_an_object_that_isnt_life_safety_owes_no_cov() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        let inner = bacnet_objects::analog::AnalogValueObject::new(2, "AV-2", 62).unwrap();
        db.add(Box::new(Rearmable(inner))).unwrap();
    })
    .await;
    let av2 = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap();
    let description = PropertyIdentifier::DESCRIPTION;
    h.subscribe_specs(false, vec![(av2, vec![(description, false)])])
        .await;
    h.notification().await;
    silence(&h, &av2).await.unwrap();
    let rearmed = format!("rearmed {}", LifeSafetyOperation::SILENCE.to_raw());
    assert_eq!(
        read_db(&h, av2, description, None).await,
        PropertyValue::CharacterString(rearmed)
    );
    // As before #1520, only a Life Safety Point or Zone reports a rearm.
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[test]
fn a_life_safety_rearm_outside_a_tokio_runtime_fails_and_changes_nothing() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let start = Harness::start_with(ServerConfig::default(), life_safety_point);
    let mut h = runtime.block_on(start);
    let (point, expected) = (lsp1(), PropertyIdentifier::OPERATION_EXPECTED);
    let before = runtime.block_on(read_db(&h, point, expected, None));
    // Polled with no runtime entered, as another executor would poll it.
    let mut rearm = silence(&h, &point);
    let polled = rearm.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    assert!(
        matches!(polled, Poll::Ready(Err(Error::Encoding(_)))),
        "{polled:?}"
    );
    drop(rearm);
    assert_eq!(runtime.block_on(read_db(&h, point, expected, None)), before);
    runtime.block_on(h.server.stop()).unwrap();
}

/// The work a committed local change owes shares the server's config with
/// it, not a copy (#1521): while that work waits, it holds one more
/// reference to the server's own config.
#[tokio::test(start_paused = true)]
async fn the_work_a_local_change_owes_shares_the_server_config() {
    let h = Harness::start_with(ServerConfig::default(), life_safety_point).await;
    let shared = Arc::strong_count(&h.server.config);
    let (target, point) = (av1(), lsp1());
    let changes = [
        write_local(&h, &target, PropertyValue::Real(70.0), None),
        silence(&h, &point),
    ];
    for mut change in changes {
        let table = Arc::clone(&h.server.cov_table).write_owned().await;
        assert!(change
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending());
        assert_eq!(Arc::strong_count(&h.server.config), shared + 1);
        drop(table);
        change.await.unwrap();
        assert_eq!(Arc::strong_count(&h.server.config), shared);
    }
}

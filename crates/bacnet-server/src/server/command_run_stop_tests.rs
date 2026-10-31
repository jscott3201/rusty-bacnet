//! A Command run the server lets go of ends where it stood rather than
//! leaving the Command in process (#1252): one `stop()` cancels in a post
//! delay, one let go of while the database is busy, one no task ever started,
//! one nothing took, and one whose write panics. A Channel's distribution,
//! the other run kind, ends FAILED in the same cases.
//!
//! CMD-1 has three lists: AO-1 to 50.0 at priority 8; AO-2 to 30.0 at
//! priority 8 with a 5-second post delay, then AO-1 to 70.0; and AO-1 to 50.0
//! with a 5-second post delay. Every command starts with its write-successful
//! flag TRUE, so a FALSE is the run's doing. After `stop()` everything is read
//! from the database. The clock is paused.
use super::channel_wire_tests::{ch, channel, encoded, member};
use super::command_action_wire_tests::{
    ao, cmd, idle, outputs, read_db, slot8, write, write_property, write_pv,
};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::constructed::decode_action_list;
use bacnet_objects::command::CommandObject;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use bacnet_types::enums::WriteStatus;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex as StdMutex, OnceLock, Weak};
use std::task::{Context, Waker};
use tokio::sync::OwnedRwLockReadGuard;

pub(super) fn objects(db: &mut ObjectDatabase) {
    outputs(db);
    let delayed = BACnetActionCommand {
        post_delay: Some(5),
        ..write(ao(2), PropertyValue::Real(30.0), 8)
    };
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![
            BACnetActionList {
                commands: vec![write(ao(1), PropertyValue::Real(50.0), 8)],
            },
            BACnetActionList {
                commands: vec![delayed, write(ao(1), PropertyValue::Real(70.0), 8)],
            },
            BACnetActionList {
                commands: vec![BACnetActionCommand {
                    post_delay: Some(5),
                    ..write(ao(1), PropertyValue::Real(50.0), 8)
                }],
            },
        ])
        .unwrap();
    db.add(Box::new(command)).unwrap();
}

/// CMD-`instance`'s In_Process and All_Writes_Successful, from the database.
pub(super) async fn db_state(h: &Harness, instance: u32) -> (bool, bool) {
    let flag = |value| match value {
        PropertyValue::Boolean(flag) => flag,
        other => panic!("not a BOOLEAN: {other:?}"),
    };
    (
        flag(read_db(h, cmd(instance), PropertyIdentifier::IN_PROCESS, None).await),
        flag(
            read_db(
                h,
                cmd(instance),
                PropertyIdentifier::ALL_WRITES_SUCCESSFUL,
                None,
            )
            .await,
        ),
    )
}

/// The write-successful flags of CMD-1's Action element `index`, from the
/// database.
pub(super) async fn db_flags(h: &Harness, index: u32) -> Vec<bool> {
    command_flags(h, 1, index).await
}

/// The write-successful flags of CMD-`instance`'s Action element `index`,
/// from the database.
async fn command_flags(h: &Harness, instance: u32, index: u32) -> Vec<bool> {
    let PropertyValue::ApplicationData(element) =
        read_db(h, cmd(instance), PropertyIdentifier::ACTION, Some(index)).await
    else {
        panic!("an encoded action list");
    };
    let (list, _) = decode_action_list(&element, 0).unwrap();
    list.commands
        .iter()
        .map(|command| command.write_successful)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn stop_during_a_post_delay_ends_the_run_with_what_it_made() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    write_pv(&mut h, 1, 2).await.unwrap();
    h.settle().await;
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(30.0));
    assert_eq!(db_state(&h, 1).await, (true, false));

    h.server.stop().await.unwrap();
    assert_eq!(db_state(&h, 1).await, (false, false));
    // The first write was made; the one the delay held back never was.
    assert_eq!(db_flags(&h, 2).await, [true, false]);
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn stop_during_the_last_post_delay_ends_the_run_successful() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    write_pv(&mut h, 1, 3).await.unwrap();
    h.settle().await;
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(50.0));
    assert_eq!(db_state(&h, 1).await, (true, false));

    h.server.stop().await.unwrap();
    // Every write was made and succeeded; only the wait after the last one
    // was cut short.
    assert_eq!(db_state(&h, 1).await, (false, true));
    assert_eq!(db_flags(&h, 3).await, [true]);
}

#[tokio::test(start_paused = true)]
async fn stop_while_the_database_is_busy_ends_the_run_where_it_stood() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    write_pv(&mut h, 1, 2).await.unwrap();
    h.settle().await;
    // An application holds the database through `stop()`, so the run can't
    // be ended as it's let go of, and `stop()` doesn't wait for the database:
    // the run ends, keeping what it made, once the application lets go.
    let held = Arc::clone(h.server.database()).read_owned().await;
    h.server.stop().await.unwrap();
    drop(held);
    h.settle().await;
    assert_eq!(db_state(&h, 1).await, (false, false));
    assert_eq!(db_flags(&h, 2).await, [true, false]);
}

#[tokio::test(start_paused = true)]
async fn run_no_task_starts_ends_at_once_as_unsuccessful() {
    let h = Harness::start_with(ServerConfig::default(), objects).await;
    // A closed task set refuses the run's task, as it does once `stop()`
    // has begun.
    h.server.request_tasks.close();
    h.server
        .write_local(
            &cmd(1),
            PV,
            None,
            PropertyValue::Unsigned(2),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(db_state(&h, 1).await, (false, false));
    assert_eq!(db_flags(&h, 2).await, [false, false]);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
}

/// The objects plus CH-5, which writes AO-2 200 ms after its Present_Value
/// is written.
pub(super) fn with_channel(db: &mut ObjectDatabase) {
    objects(db);
    db.add(Box::new(channel(5, 5, vec![(member(ao(2), PV), 200)])))
        .unwrap();
}

/// CH-5's Write_Status, from the database.
pub(super) async fn channel_status(h: &Harness) -> WriteStatus {
    match read_db(h, ch(5), PropertyIdentifier::WRITE_STATUS, None).await {
        PropertyValue::Enumerated(raw) => WriteStatus::from_raw(raw),
        other => panic!("not an ENUMERATED: {other:?}"),
    }
}

/// Start CH-5's distribution over the wire; it waits out the member's delay.
async fn start_distribution(h: &mut Harness) {
    write_property(h, ch(5), encoded(&PropertyValue::Real(12.0)), Some(8))
        .await
        .unwrap();
    assert_eq!(channel_status(h).await, WriteStatus::IN_PROGRESS);
}

#[tokio::test(start_paused = true)]
async fn stop_during_a_channel_member_delay_ends_the_distribution_failed() {
    let mut h = Harness::start_with(ServerConfig::default(), with_channel).await;
    start_distribution(&mut h).await;
    h.server.stop().await.unwrap();
    assert_eq!(channel_status(&h).await, WriteStatus::FAILED);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn stop_while_the_database_is_busy_ends_a_channel_distribution_failed() {
    let mut h = Harness::start_with(ServerConfig::default(), with_channel).await;
    start_distribution(&mut h).await;
    let held = Arc::clone(h.server.database()).read_owned().await;
    h.server.stop().await.unwrap();
    drop(held);
    h.settle().await;
    assert_eq!(channel_status(&h).await, WriteStatus::FAILED);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn channel_run_no_task_starts_ends_at_once_failed() {
    let h = Harness::start_with(ServerConfig::default(), with_channel).await;
    h.server.request_tasks.close();
    h.server
        .write_local(
            &ch(5),
            PV,
            None,
            PropertyValue::Real(12.0),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(channel_status(&h).await, WriteStatus::FAILED);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn stop_ends_a_run_nothing_took() {
    let mut h = Harness::start_with(ServerConfig::default(), with_channel).await;
    // Written straight into the database, each run stays queued on its object
    // with nothing to take it.
    {
        let mut db = h.server.database().write().await;
        db.get_mut(&cmd(1))
            .unwrap()
            .write_property(PV, None, PropertyValue::Unsigned(2), None)
            .unwrap();
        db.get_mut(&ch(5))
            .unwrap()
            .write_property(PV, None, PropertyValue::Real(12.0), Some(8))
            .unwrap();
    }
    assert_eq!(db_state(&h, 1).await, (true, false));
    assert_eq!(channel_status(&h).await, WriteStatus::IN_PROGRESS);

    h.server.stop().await.unwrap();
    assert_eq!(db_state(&h, 1).await, (false, false));
    assert_eq!(db_flags(&h, 2).await, [false, false]);
    assert_eq!(channel_status(&h).await, WriteStatus::FAILED);
}

fn boom() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::from_raw(600), 7).unwrap()
}

/// A reader of the database, queued but not yet granted.
type QueuedReader = Pin<Box<dyn Future<Output = OwnedRwLockReadGuard<ObjectDatabase>> + Send>>;

/// What BOOM-7 needs to queue a reader for the database as it panics: the
/// database, once the server holds it, and where to leave the reader.
#[derive(Clone, Default)]
struct Contention {
    db: Arc<OnceLock<Weak<RwLock<ObjectDatabase>>>>,
    reader: Arc<StdMutex<Option<QueuedReader>>>,
}

impl Contention {
    /// Queue a reader behind the write in progress, so the database is
    /// granted to it, not free, the moment that write lets go.
    fn queue_reader(&self) {
        let Some(db) = self.db.get().and_then(Weak::upgrade) else {
            return;
        };
        let mut reader: QueuedReader = Box::pin(db.read_owned());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(reader.as_mut().poll(&mut cx).is_pending());
        *self.reader.lock().unwrap() = Some(reader);
    }
}

/// A vendor-type object whose every write panics, queueing a database
/// reader first when it has a [`Contention`].
#[derive(Default)]
struct Panicking(Option<Contention>);

impl BACnetObject for Panicking {
    fn object_identifier(&self) -> ObjectIdentifier {
        boom()
    }

    fn object_name(&self) -> &str {
        "BOOM-7"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => PropertyValue::ObjectIdentifier(boom()),
            PropertyIdentifier::OBJECT_NAME => PropertyValue::CharacterString("BOOM-7".into()),
            PropertyIdentifier::OBJECT_TYPE => PropertyValue::Enumerated(600),
            _ => PropertyValue::Real(0.0),
        })
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(contention) = &self.0 {
            contention.queue_reader();
        }
        panic!("BOOM-7 refuses every write by panicking");
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

/// The objects plus BOOM-7 and CMD-2, whose list 1 writes BOOM-7, then AO-1
/// to 50.0, and whose list 2 writes AO-2 to 30.0 with a 5-second post delay,
/// then BOOM-7, then AO-1 to 50.0.
fn with_panicking(db: &mut ObjectDatabase) {
    with_boom(db, Panicking::default());
}

fn with_boom(db: &mut ObjectDatabase, object: Panicking) {
    objects(db);
    db.add(Box::new(object)).unwrap();
    let to_boom = write(boom(), PropertyValue::Real(1.0), 8);
    let to_ao1 = write(ao(1), PropertyValue::Real(50.0), 8);
    let delayed = BACnetActionCommand {
        post_delay: Some(5),
        ..write(ao(2), PropertyValue::Real(30.0), 8)
    };
    let mut command = CommandObject::new(2, "CMD-2").unwrap();
    command
        .set_action(vec![
            BACnetActionList {
                commands: vec![to_boom.clone(), to_ao1.clone()],
            },
            BACnetActionList {
                commands: vec![delayed, to_boom, to_ao1],
            },
        ])
        .unwrap();
    db.add(Box::new(command)).unwrap();
}

#[tokio::test(start_paused = true)]
async fn run_whose_write_panics_ends_with_its_unmade_commands_unsuccessful() {
    let mut h = Harness::start_with(ServerConfig::default(), with_panicking).await;
    let mut body = BytesMut::new();
    bacnet_services::cov::SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 7,
        monitored_object_identifier: cmd(2),
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
    response(&h).await.unwrap();
    let in_process = |notification: bacnet_services::cov::COVNotificationRequest| {
        notification.list_of_values[0].value.clone()
    };
    assert_eq!(in_process(h.cov_notification().await), [0x10]);

    write_pv(&mut h, 2, 1).await.unwrap();
    assert_eq!(in_process(h.cov_notification().await), [0x11]);
    // The panic ends the run, and its subscriber hears In_Process fall.
    assert_eq!(in_process(h.cov_notification().await), [0x10]);
    assert_eq!(db_state(&h, 2).await, (false, false));
    assert_eq!(command_flags(&h, 2, 1).await, [false, false]);
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn run_whose_write_panics_while_the_database_is_busy_ends_where_it_stood() {
    let contention = Contention::default();
    let object = Panicking(Some(contention.clone()));
    let mut h = Harness::start_with(ServerConfig::default(), |db| with_boom(db, object)).await;
    contention
        .db
        .set(Arc::downgrade(h.server.database()))
        .unwrap();
    write_pv(&mut h, 2, 2).await.unwrap();
    // BOOM-7 queues a reader as it panics, so the database goes to that
    // reader as the panicking write lets go: the run can't be ended while it
    // unwinds, and waits for the panic path.
    tokio::time::timeout(Duration::from_secs(10), async {
        while contention.reader.lock().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("BOOM-7 was written");
    let reader = contention.reader.lock().unwrap().take().unwrap();
    let db = reader.await;
    let in_process = db
        .get(&cmd(2))
        .unwrap()
        .read_property(PropertyIdentifier::IN_PROCESS, None)
        .unwrap();
    assert_eq!(in_process, PropertyValue::Boolean(true));
    drop(db);

    idle(&h, 2).await;
    assert_eq!(db_state(&h, 2).await, (false, false));
    // AO-2 was written before the panic; BOOM-7 and AO-1 never were.
    assert_eq!(command_flags(&h, 2, 2).await, [true, false, false]);
    assert!(h.server.request_tasks.take_stranded().is_empty());
}

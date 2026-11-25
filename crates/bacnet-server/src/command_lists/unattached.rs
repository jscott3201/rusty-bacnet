//! Running Command lists and Channel distributions without a server, for
//! [`tick_schedules`](crate::schedule::tick_schedules) (#1178, #1151).
//!
//! Each write is made through
//! [`bacnet_device_core::write::write_property`], as a WriteProperty request
//! carrying it, with the Command or Channel as the initiating object. There
//! is no network here, so a command or Channel member naming another device
//! fails unsent, which a Channel reports as PROCESS_ERROR. There is no
//! COV table, notification path or task set either, so nothing is reported
//! and the runs go on inside one future: a run that writes another
//! Command's or Channel's Present_Value starts that run beside it, and the
//! future ends once every run has.
//!
//! `tick_schedules` polls that future in its caller's task until it has to
//! wait, then in a task on the runtime, so no wait of a run's is left in a
//! caller that stops polling (#2781). The future stays the caller's all
//! the same ([`SharedRuns`]): the task polls it through a slot the caller
//! empties as it is dropped, so a dropped tick ends its runs in the drop,
//! as it did when they lived in the caller's own future.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::command::CommandRun;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetActionCommand, BACnetDeviceObjectPropertyReference};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::sync::MutexExt;
use bytes::BytesMut;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::sync::{RwLock, mpsc};

use super::{RunHost, TakenRuns, end_when_free, execute};
use crate::caller_waits::HeldDatabase;
use crate::server::RemoteRequestError;
use bacnet_device_core::write::Unfinished;

/// The runs a tick started, shared by its caller, which owns them, and the
/// task that makes them once one has to wait (#2781).
///
/// The task polls them through a [`Driver`], holding the slot's lock for
/// one poll at a time; a poll waits for nothing but what a run's write
/// waited for in the caller before (a durable target's save, say). Dropped,
/// this takes the runs out of the slot without waiting for the lock and
/// drops them, ending each unfinished one as unsuccessful as their own
/// future's drop always has. Only a poll in progress on another thread as
/// it is dropped keeps them: they write nothing more once this is dropped,
/// but for a write that poll had begun, and end once it returns, when the
/// task sees its caller gone and drops the driver. Whatever drops them, the
/// database handle they hold goes off the runtime (#1513).
pub(crate) struct SharedRuns {
    slot: Slot,
    gone: Arc<AtomicBool>,
}

/// The runs' future, while neither its end nor a drop has taken it.
pub(crate) type Slot = Arc<Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>>;

impl SharedRuns {
    /// Run `runs`, and any runs their writes start, on `db`.
    pub(crate) fn new(db: HeldDatabase, runs: TakenRuns) -> Self {
        let gone = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&gone);
        let runs: Pin<Box<dyn Future<Output = ()> + Send>> =
            Box::pin(async move { run_unattached(&db, runs, &stop).await });
        Self {
            slot: Arc::new(Mutex::new(Some(runs))),
            gone,
        }
    }

    /// The task's side of the runs.
    pub(crate) fn driver(&self) -> Driver {
        Driver(Arc::clone(&self.slot))
    }

    /// The slot, for a test to stand for a poll in progress.
    #[cfg(test)]
    pub(crate) fn slot(&self) -> Slot {
        Arc::clone(&self.slot)
    }
}

impl Drop for SharedRuns {
    fn drop(&mut self) {
        self.gone.store(true, Ordering::Release);
        // `None` for a poll in progress: the task drops them once it returns.
        drop(take_unless_polled(&self.slot));
    }
}

/// Polls a tick's runs until they end, or until their caller has taken
/// them. Dropped first, as its task ends, is aborted or goes with its
/// runtime while the caller lives on, it drops what is left of them, so no
/// wait of theirs, a queued lock waiter among them, outlives the task.
pub(crate) struct Driver(Slot);

impl Future for Driver {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut runs = self.0.locked();
        let Some(future) = runs.as_mut() else {
            return Poll::Ready(());
        };
        let polled = future.as_mut().poll(cx);
        if polled.is_ready() {
            *runs = None;
        }
        polled
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        drop(take_unless_polled(&self.0));
    }
}

/// The runs in `slot`, taken out without waiting for its lock, or `None`
/// when a poll holds it or they are gone already. The caller drops them
/// once the lock is let go: ending them takes nothing of it.
fn take_unless_polled(slot: &Slot) -> Option<Pin<Box<dyn Future<Output = ()> + Send>>> {
    slot.try_locked()?.take()
}

/// Run `runs`, and any runs their writes start, to their ends, making no
/// write once `gone` is set.
///
/// Dropping the returned future first, unpolled included, ends each
/// unfinished run as unsuccessful, so none is left busy: a run being made
/// through its [`execute`] owner, and one queued but not yet started through
/// the queue's own guard.
fn run_unattached<'a>(
    db: &'a Arc<RwLock<ObjectDatabase>>,
    runs: TakenRuns,
    gone: &'a AtomicBool,
) -> impl Future<Output = ()> + Send + 'a {
    let (started, queued) = mpsc::unbounded_channel();
    runs.hand_over(|run| {
        // The receiver is in hand.
        let _ = started.send(run);
    });
    let mut queue = Queue { db, queued };
    async move {
        let host = Unattached { db, started, gone };
        let mut running = FuturesUnordered::new();
        loop {
            // A run is only started from inside another's write, so once none
            // is left running and none is queued, nothing more can start.
            while let Ok(run) = queue.queued.try_recv() {
                running.push(execute(&host, run));
            }
            if running.is_empty() {
                break;
            }
            // Queued runs first, so each gets its owner as soon as it can.
            tokio::select! {
                biased;
                Some(run) = queue.queued.recv() => running.push(execute(&host, run)),
                Some(()) = running.next() => {}
                else => break,
            }
        }
    }
}

/// Runs queued for [`run_unattached`] and not yet started. Dropping it ends
/// each as unsuccessful.
struct Queue<'a> {
    db: &'a Arc<RwLock<ObjectDatabase>>,
    queued: mpsc::UnboundedReceiver<CommandRun>,
}

impl Drop for Queue<'_> {
    fn drop(&mut self) {
        self.queued.close();
        while let Ok(run) = self.queued.try_recv() {
            end_when_free(self.db, Unfinished::start(&run));
        }
    }
}

struct Unattached<'a> {
    db: &'a Arc<RwLock<ObjectDatabase>>,
    started: mpsc::UnboundedSender<CommandRun>,
    /// Set once the tick that started the runs has been dropped.
    gone: &'a AtomicBool,
}

impl RunHost for Unattached<'_> {
    fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        self.db
    }

    async fn write(&self, run: &CommandRun, command: &BACnetActionCommand) -> Result<(), Error> {
        let request = request(command)?;
        let runs = {
            let mut db = self.db.write().await;
            // The tick was dropped while a poll of the task's on another
            // thread made the runs: write nothing more, and wait here for
            // the task to drop the runs as that poll returns (#2781).
            if self.gone.load(Ordering::Acquire) {
                drop(db);
                return std::future::pending().await;
            }
            // Under the write's own guard: a run whose object was deleted
            // meanwhile writes nothing (#2265).
            if !super::is_current(&db, run) {
                return Err(super::stale_run());
            }
            let origin = bacnet_device_core::write::local::resolve_source(
                &db,
                bacnet_device_core::write::local::LocalCommandSource::Object(run.source),
            )
            .ok();
            let (target, _) = bacnet_device_core::write::write_property(
                &mut db,
                &request,
                None,
                None,
                origin.as_ref(),
            )?;
            TakenRuns::take(self.db, &mut db, &[target])
        };
        super::admit(self, run, runs, |runs| {
            runs.hand_over(|run| {
                // The receiver lives as long as this host, but a run it
                // refuses still ends rather than going unowned.
                if let Err(refused) = self.started.send(run) {
                    end_when_free(self.db, Unfinished::start(&refused.0));
                }
            });
        })
        .await
    }

    async fn write_remote(
        &self,
        _run: &CommandRun,
        _device: ObjectIdentifier,
        _command: &BACnetActionCommand,
    ) -> Result<(), RemoteRequestError> {
        Err(RemoteRequestError::NoNetwork)
    }

    async fn read_remote(
        &self,
        _device: ObjectIdentifier,
        _reference: &BACnetDeviceObjectPropertyReference,
    ) -> Result<PropertyValue, RemoteRequestError> {
        Err(RemoteRequestError::NoNetwork)
    }

    fn remote_slots(&self) -> Option<&super::RemoteSlots> {
        None
    }

    async fn committed(&self, _db: &ObjectDatabase, _source: ObjectIdentifier) {}

    async fn report(&self, _source: ObjectIdentifier) {}

    fn abandoned(&self, left: Unfinished) {
        end_when_free(self.db, left);
    }
}

/// The WriteProperty request a command amounts to.
fn request(command: &BACnetActionCommand) -> Result<Vec<u8>, Error> {
    let mut value = BytesMut::new();
    encode_property_value(&mut value, &command.property_value)?;
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: command.object_identifier,
        property_identifier: command.property_identifier,
        property_array_index: command.property_array_index,
        property_value: value.to_vec(),
        priority: command.priority,
    }
    .encode(&mut request)?;
    Ok(request.to_vec())
}

#[cfg(test)]
#[path = "unattached_tests.rs"]
mod tests;

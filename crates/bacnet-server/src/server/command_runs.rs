//! Running the writes a Command or Channel object's Present_Value write
//! queues (Clauses 12.10 and 12.53, #1150, #1151).
//!
//! The write only queues the run: under the guard that commits it, the object
//! checks the value, marks itself busy and leaves a [`CommandRun`], which the
//! server takes there and owns from then on (`crate::command_lists`). Each
//! run then goes into the server's request task set as its own task, so
//! neither the request that wrote Present_Value nor `write_local` waits for
//! it, delays included, and `stop` cancels it with the other request work.
//! Until [`CommandRunner::start`] queues that task, the writer holds the run
//! in a [`TakenRuns`], so a writer dropped after its commit ends the run
//! instead of leaving its object busy (#1324). `write_local` hands what a
//! committed write owes, its runs included, to a request task, so its
//! caller's timeout no longer drops them (#1367); `stop()` aborting that
//! task does.
//!
//! A Command's writes are made one at a time, and a Channel's local members
//! in delay order beside its remote ones (`crate::command_lists`), each local
//! write through the same [`LocalWriter`] path as `write_local`: priorities,
//! command-source tracking, audit, COV and the post-write event pass all
//! apply as for any other local write. A command or member naming another
//! device goes out as a confirmed WriteProperty through [`RemoteWriter`]
//! (#1180), which also makes the ReadProperty a Channel learns a remote
//! member's datatype with (#1342). Every run on the server shares one set of
//! queues for those requests, kept in [`NotificationTransactions`] beside the
//! invoke IDs they lease: one at a time per device, a bounded number in all
//! (`crate::command_lists::RemoteSlots`). No database guard is held across a write's
//! notifications, an outstanding remote request or a delay. The
//! object's generation guards every report back, so a run whose object was
//! replaced or reconfigured stops.
//!
//! A run let go of before it ends (cancelled by `stop()`, refused by a closed
//! task set, or unwound by a panic) ends where it stood at once when the
//! database is free: the commands it hadn't made read unsuccessful, and it
//! counts as successful only if every write was made and succeeded. Otherwise
//! it waits in the request task set, and `stop()` ends it once those tasks are
//! joined and the database is free (#1252); a panic ends it straight away.

use super::local_writes::{LocalWrite, LocalWriter};
use super::remote_writes::{RemoteWrite, RemoteWriter};
use super::request_tasks::RequestTaskSpawner;
use super::*;
use crate::command_lists::{RunHost, TakenRuns, Unfinished};
use bacnet_objects::command::{CommandRun, WriteFailure};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::constructed::{BACnetActionCommand, BACnetDeviceObjectPropertyReference};

/// The server handles a run owns while it waits out post delays.
pub(super) struct CommandRunner<T: TransportPort + 'static> {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<T>>,
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    cov_in_flight: Arc<Semaphore>,
    notification_transactions: Arc<NotificationTransactions>,
    comm_state: Arc<CommState>,
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    event_suppressions: Arc<super::event_suppression::EventSuppressions>,
    config: Arc<ServerConfig>,
    tasks: RequestTaskSpawner,
}

impl<T: TransportPort + 'static> Clone for CommandRunner<T> {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            network: Arc::clone(&self.network),
            cov_table: Arc::clone(&self.cov_table),
            cov_in_flight: Arc::clone(&self.cov_in_flight),
            notification_transactions: Arc::clone(&self.notification_transactions),
            comm_state: Arc::clone(&self.comm_state),
            learned_routers: Arc::clone(&self.learned_routers),
            device_bindings: Arc::clone(&self.device_bindings),
            event_suppressions: Arc::clone(&self.event_suppressions),
            config: Arc::clone(&self.config),
            tasks: self.tasks.clone(),
        }
    }
}

impl<T: TransportPort + 'static> CommandRunner<T> {
    /// A runner over a request's handles, starting runs beside its task.
    pub(super) fn new(services: &RequestServices<T>, tasks: &RequestTaskSpawner) -> Self {
        Self {
            db: Arc::clone(&services.db),
            network: Arc::clone(&services.network),
            cov_table: Arc::clone(&services.cov_table),
            cov_in_flight: Arc::clone(&services.cov_in_flight),
            notification_transactions: Arc::clone(&services.notification_transactions),
            comm_state: Arc::clone(&services.comm_state),
            learned_routers: Arc::clone(&services.learned_routers),
            device_bindings: Arc::clone(&services.device_bindings),
            event_suppressions: Arc::clone(&services.event_suppressions),
            config: Arc::clone(&services.config),
            tasks: tasks.clone(),
        }
    }

    /// A runner over an unconfirmed request's handles, for WriteGroup.
    pub(super) fn for_unconfirmed(services: &UnconfirmedServices<T>) -> Self {
        Self {
            db: Arc::clone(&services.db),
            network: Arc::clone(&services.network),
            cov_table: Arc::clone(&services.cov_table),
            cov_in_flight: Arc::clone(&services.cov_in_flight),
            notification_transactions: Arc::clone(&services.notification_transactions),
            comm_state: Arc::clone(&services.comm_state),
            learned_routers: Arc::clone(&services.learned_routers),
            device_bindings: Arc::clone(&services.device_bindings),
            event_suppressions: Arc::clone(&services.event_suppressions),
            config: Arc::clone(&services.config),
            tasks: services.tasks.clone(),
        }
    }

    /// A runner over a running server's own handles, for `write_local`.
    pub(super) fn for_server(server: &BACnetServer<T>) -> Self {
        let writer = server.local_writer();
        Self {
            db: Arc::clone(writer.db),
            network: Arc::clone(writer.network),
            cov_table: Arc::clone(writer.cov_table),
            cov_in_flight: Arc::clone(writer.cov_in_flight),
            notification_transactions: Arc::clone(writer.notification_transactions),
            comm_state: Arc::clone(writer.comm_state),
            learned_routers: Arc::clone(writer.learned_routers),
            device_bindings: Arc::clone(writer.device_bindings),
            event_suppressions: Arc::clone(writer.event_suppressions),
            config: Arc::clone(writer.config),
            tasks: server.request_tasks.spawner(),
        }
    }

    /// Start each run as its own task. A panic in a target's write path
    /// ends that run as failed rather than leaving its object busy.
    pub(super) fn start(&self, runs: TakenRuns) {
        use futures_util::FutureExt;
        runs.hand_over(|run| {
            let mut queued = Queued {
                runner: self.clone(),
                left: Some(Unfinished::start(&run)),
            };
            self.tasks.spawn(async move {
                let (source, generation) = (run.source, run.generation);
                let execution = std::panic::AssertUnwindSafe(crate::command_lists::execute(
                    &queued.runner,
                    run,
                ));
                // The run's own owner holds it from here.
                queued.left = None;
                if execution.catch_unwind().await.is_err() {
                    warn!(source = %source, "run panicked; ending it as failed");
                    queued.runner.end_panicked(source, generation).await;
                }
            });
        });
    }

    /// End a run whose write panicked. Unwinding handed it to `abandoned`,
    /// which ended it if the database was free and stranded it otherwise; a
    /// stranded one is taken back and ended where it stood. Either way its
    /// subscribers hear of the end.
    async fn end_panicked(&self, source: ObjectIdentifier, generation: u64) {
        if let Some(left) = self.tasks.unstrand(source, generation) {
            let mut db = self.db.write().await;
            if left.end(&mut db) {
                self.committed(&db, source).await;
            }
        } else if crate::command_lists::complete(
            self,
            source,
            generation,
            Err(WriteFailure::Process),
        )
        .await
        {
            // Nothing had ended it, so `complete` did and reported it.
            return;
        }
        self.report(source).await;
    }

    fn remote_writer(&self) -> RemoteWriter<'_, T> {
        RemoteWriter {
            network: &self.network,
            transactions: &self.notification_transactions,
            bindings: &self.device_bindings,
            comm_state: &self.comm_state,
            timeout: Duration::from_millis(self.config.cov_retry_timeout_ms),
            retries: DEFAULT_APDU_RETRIES,
            max_apdu: self.config.max_apdu_length,
        }
    }

    /// The local write path over these handles.
    pub(super) fn writer(&self) -> LocalWriter<'_, T> {
        LocalWriter {
            db: &self.db,
            network: &self.network,
            cov_table: &self.cov_table,
            cov_in_flight: &self.cov_in_flight,
            notification_transactions: &self.notification_transactions,
            comm_state: &self.comm_state,
            learned_routers: &self.learned_routers,
            device_bindings: &self.device_bindings,
            event_suppressions: &self.event_suppressions,
            config: &self.config,
        }
    }
}

impl<T: TransportPort + 'static> RunHost for CommandRunner<T> {
    fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        &self.db
    }

    /// Write one value through the local write path, with `run`'s object as
    /// the initiating object.
    async fn write(&self, run: &CommandRun, command: &BACnetActionCommand) -> Result<(), Error> {
        // The value reaches the target as a WriteProperty carrying the same
        // octets would, so constructed values take the shape the object
        // expects.
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &command.property_value)?;
        // A read guard of its own: an object's array and list classification
        // is fixed, so it still holds when the write takes its guard, which
        // checks the index again (#1426).
        let value = {
            let db = self.db.read().await;
            let object = db.get(&command.object_identifier).ok_or(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
            })?;
            handlers::gate_and_decode_write(
                object,
                command.property_identifier,
                command.property_array_index,
                &encoded,
            )?
        };
        let runs = self
            .writer()
            .write(
                &command.object_identifier,
                LocalWrite::Property {
                    property: command.property_identifier,
                    array_index: command.property_array_index,
                    priority: command.priority,
                },
                value,
                Some(crate::LocalCommandSource::Object(run.source)),
            )
            .await?;
        // A write that starts another Command's or Channel's run starts that
        // run too, unless it would close a loop.
        crate::command_lists::admit(self, run, runs, |runs| self.start(runs)).await
    }

    /// Write one value in another device as a confirmed WriteProperty.
    async fn write_remote(
        &self,
        device: ObjectIdentifier,
        command: &BACnetActionCommand,
    ) -> Result<(), RemoteRequestError> {
        let write = RemoteWrite::for_command(device, command)?;
        self.remote_writer().write(&write).await
    }

    /// Read one property in another device as a confirmed ReadProperty.
    async fn read_remote(
        &self,
        device: ObjectIdentifier,
        reference: &BACnetDeviceObjectPropertyReference,
    ) -> Result<PropertyValue, RemoteRequestError> {
        let request = ReadPropertyRequest {
            object_identifier: reference.object_identifier,
            property_identifier: PropertyIdentifier::from_raw(reference.property_identifier),
            property_array_index: reference.property_array_index,
        };
        self.remote_writer().read(device, &request).await
    }

    /// The server's queues, shared by every run on it.
    fn remote_slots(&self) -> Option<&crate::command_lists::RemoteSlots> {
        Some(self.notification_transactions.run_slots())
    }

    /// Timestamped references capture the change under its guard (#856).
    async fn committed(&self, db: &ObjectDatabase, source: ObjectIdentifier) {
        let capture = self.cov_table.read().await.timed_capture(source);
        capture.run(db);
    }

    /// Property subscribers hear of In_Process, All_Writes_Successful and
    /// Write_Status.
    async fn report(&self, source: ObjectIdentifier) {
        BACnetServer::<T>::fire_cov_notifications(&self.writer().cov_context(), &source).await;
    }

    /// End the run where it stood if the database is free. Otherwise leave it
    /// with the request task set for `stop()`, or, once the server is gone,
    /// end it when the database frees up. Nothing is reported from here:
    /// `stop()` is the usual caller, and a panic reports in [`Self::start`].
    fn abandoned(&self, left: Unfinished) {
        let source = left.source();
        match self.db.try_write() {
            Ok(mut db) => {
                // The COV table comes after the database in lock order, so a
                // busy one only costs the timestamped capture.
                if left.end(&mut db) {
                    if let Ok(table) = self.cov_table.try_read() {
                        table.timed_capture(source).run(&db);
                    }
                }
            }
            Err(_) => {
                if let Err(left) = self.tasks.strand(left) {
                    crate::command_lists::end_when_free(&self.db, left);
                }
            }
        }
    }
}

/// A run taken for a task that hasn't started it yet. Dropping it first (a
/// task set that refused the task, or an abort before its first poll) hands
/// the run to [`RunHost::abandoned`], so its object isn't left busy.
struct Queued<T: TransportPort + 'static> {
    runner: CommandRunner<T>,
    left: Option<Unfinished>,
}

impl<T: TransportPort + 'static> Drop for Queued<T> {
    fn drop(&mut self) {
        if let Some(left) = self.left.take() {
            self.runner.abandoned(left);
        }
    }
}

use super::*;
use bacnet_types::error::{Lifecycle, Refusal};

#[cfg(test)]
#[path = "request_tasks_tests.rs"]
mod request_tasks_tests;

#[cfg(test)]
#[path = "producer_shutdown_tests.rs"]
mod producer_shutdown_tests;

#[cfg(test)]
#[path = "transport_shutdown_tests.rs"]
mod transport_shutdown_tests;

#[cfg(test)]
#[path = "owned_shutdown_tests.rs"]
mod owned_shutdown_tests;

#[cfg(test)]
#[path = "log_persistence_tests.rs"]
mod log_persistence_tests;

async fn stop_producer(slot: &mut Option<JoinHandle<()>>) {
    // Borrow across the join so cancellation leaves the handle recoverable.
    // Clear synchronously after completion: a later stop must not poll it twice.
    if let Some(task) = slot.as_mut() {
        task.abort();
        let _ = task.await;
    }
    *slot = None;
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Seal egress, join admitted work, and stop the owned transport.
    ///
    /// Call it before dropping the server. A server dropped without it in
    /// async code aborts its tasks and lets the object database go on Tokio's
    /// blocking pool once they have let go of it (#1409), so the drop returns
    /// at once, but nothing waits for the objects' last saves. Storage may
    /// still change after such a drop returns, as those saves and the
    /// put-back of a staged write (#1363) land, so `stop().await` before
    /// building another server on the same storage.
    ///
    /// Once its requests are joined, a write a Notification Forwarder,
    /// Notification Class, Access Rights object or Audit Log still holds
    /// staged for one of them is dropped, and stop waits until every save
    /// those objects have queued has run, so storage holds the state each
    /// object serves (#1363); see
    /// [`DurableWrites::settle_forgotten_writes`]. A Trend Log, Trend Log
    /// Multiple or Event Log kept in storage commits once more there, if it
    /// changed since its last commit, and stop waits for that commit too
    /// (#1541). That wait has no limit:
    /// storage that stalls holds stop up, and a warning naming the objects
    /// still saving is logged after 5 s and every 30 s after that. Stop does
    /// not wait while the application holds the database, but for the
    /// bounded wait for release tasks described below: the objects then
    /// settle from a task once it lets go, and put storage back when they
    /// are dropped. That task lets go of its handle through
    /// [`drop_database_off_runtime`], so if it holds the last one the
    /// objects drop on Tokio's blocking pool.
    ///
    /// A request's abort lands when its task next yields, so a request
    /// already running when stop begins may find its save done and make its
    /// write first. On a transport of its own its answer is sealed; on a
    /// node's seat, whose shared layer stays open for the node's other
    /// roles, an answer the request sends before its abort lands may still
    /// go out. Either way the object serves that write, and storage holds
    /// it (#1457).
    ///
    /// A local write aborted with its request (a Command or Channel run
    /// writing) or dropped by the application lets go of its stage as it
    /// drops, at once when the database guard is free (#2493). One that finds
    /// the guard taken leaves the release to a task, and stop waits for such
    /// tasks before it settles, for at most 5 s (#2495). A task still waiting
    /// then is behind a guard the application holds: the objects settle from
    /// a task once it is free (see above), and storage is put back after stop
    /// returns. No local write can drop its stage after that wait: one the
    /// application makes borrows the server, which stop holds exclusively,
    /// and fails before it stages once stop has sealed the server, and a
    /// run's write is a request task, joined before the wait.
    ///
    /// [`DurableWrites::settle_forgotten_writes`]:bacnet_objects::durable::DurableWrites::settle_forgotten_writes
    ///
    /// Cancelling this waiter leaves cleanup owned by the server. A later stop
    /// joins it; after transport cleanup begins, dropping the server lets that
    /// cleanup finish. Local mutation and broadcasts are rejected from the first
    /// stop poll; local reads and database inspection remain available.
    ///
    /// A server on a node's seat ([`ServerBuilder::seat`]) does all of this
    /// but the link's part: the node's other roles answer through the shared
    /// layer, and the node stops it (#2617).
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(network) = self.network.as_ref().filter(|_| self.owns_link()) {
            network.seal_responses();
        }
        // On a seat, the node initiates again (#2617).
        self.comm_state.release_node();
        self.broadcaster.seal();
        self.request_tasks.close();
        stop_producer(&mut self.network_number_task).await;
        if let Some(runtime) = &self.target_audit {
            // Only the target drain retains ingress for ACK/control progress.
            self.notification_transactions
                .application_sealed
                .store(true, Ordering::Release);
            runtime.seal();
            runtime.batches.begin_stop();
            for task in [
                &self.fault_detection_task,
                &self.event_enrollment_task,
                &self.trend_log_task,
                &self.schedule_tick_task,
                &self.intrinsic_reporting_task,
                &self.binary_lighting_operation_task,
                &self.cov_purge_task,
                &self.cov_revisit_task,
            ]
            .into_iter()
            .flatten()
            {
                task.abort();
            }
            {
                // Held across the timer's join, and waited for here, which
                // holds up nothing the server serves, a stop left unpolled
                // included (#2781): the set closed above admits no DCC or
                // ReinitializeDevice request, the only requests that take
                // the slot, and aborted those it held; dispatch never takes
                // it. The timer, aborted once stop holds the slot, gives up
                // its own wait for it without the guard. One queued ahead of
                // stop, as an aborted request lets go, takes the slot first
                // and makes its expiry, as it would have a moment before;
                // stop then joins a timer that has ended. A drop's join
                // (`join_released`, below) meets the same order.
                let mut timer = self.dcc_timer.lock().await;
                super::dcc_timer::cancel(&mut timer).await;
            }
            loop {
                let finished = runtime.batches.finished.notified();
                tokio::pin!(finished);
                finished.as_mut().enable();
                if runtime.batches.stopped() {
                    break;
                }
                finished.await;
            }
        } else {
            // Other profiles retain their sequential, cancellation-safe shutdown.
            self.notification_transactions.close();
        }
        // Keep the handle in self until joined: cancellation must not detach
        // dispatch and allow a later stop to race its join consumer.
        if let Some(task) = self.dispatch_task.as_mut() {
            task.abort();
            let _ = task.await;
        }
        self.dispatch_task = None;
        while let Some(result) = self.request_tasks.join_next().await {
            super::request_tasks::RequestTasks::observe(Some(result));
        }
        // No request is left to push an event notification: seal the lanes
        // and abort their tasks, dropping what they hold (#2537). A producer
        // stopped below finds them sealed.
        self.notification_transactions.shut_lanes().await;
        {
            // Every request is joined: only the timer, which the join
            // aborts, can want the slot held across it (#2781).
            let mut timer = self.dcc_timer.lock().await;
            super::dcc_timer::cancel(&mut timer).await;
        }
        stop_producer(&mut self.fault_detection_task).await;
        stop_producer(&mut self.event_enrollment_task).await;
        stop_producer(&mut self.trend_log_task).await;
        stop_producer(&mut self.schedule_tick_task).await;
        stop_producer(&mut self.intrinsic_reporting_task).await;
        stop_producer(&mut self.binary_lighting_operation_task).await;
        stop_producer(&mut self.cov_purge_task).await;
        stop_producer(&mut self.cov_revisit_task).await;
        // Nothing looks at a reference owed room any more: its wait goes
        // with the task (#2671).
        self.cov_table.read().await.revisits().clear();
        // A local write dropped while the guard was taken lets go of its
        // stage from a task of its own (#2493). Let those finish, so none
        // holds the guard as the objects settle (#2495), but for a bounded
        // time: one behind an application's guard must not hold stop up.
        self.request_tasks.stage_releases().wait_at_stop().await;
        // No request is left to take or release a staged save, and the
        // operation task that would drop one is gone: put storage back to
        // what each object serves before stop returns (#1363).
        super::durable_writes::settle_forgotten(&self.db).await;
        // Nothing can own a Command or Channel run any more. End the runs let
        // go of while the database was busy where they stood, then any run no
        // task took up, so none is left in progress (#1252). An application
        // holding the database doesn't hold up stop: they end once it's free.
        crate::command_lists::end_unowned(&self.db, self.request_tasks.take_stranded());
        // Dispatch has relinquished the sole join-consumer role. Retain the
        // set in self across await so a cancelled stop can finish this drain.
        while let Some(result) = self.notification_transactions.join_next().await {
            NotificationTransactions::observe(Some(result));
        }
        if let Some(runtime) = &self.target_audit {
            runtime.uninstall(&mut *self.db.write().await);
        }
        self.target_audit = None;
        if let Some(error) = &self.transport_cleanup_error {
            return Err(cleanup_failed(error.clone()));
        }
        if !self.owns_link() {
            // The node's layer, which the node stops: let go of it (#2617).
            // The seat is free already, its queue's reader joined above.
            self.network = None;
            return Ok(());
        }
        if self.transport_cleanup.is_none()
            && let Some(network) = self.network.take()
        {
            // The layer stops through a shared reference (#2617), whoever
            // else still holds it. Own the complete future, including
            // cancellation-unsafe custom transport cleanup. Drop
            // deliberately leaves this task running.
            self.transport_cleanup = Some(tokio::spawn(async move {
                let result = network.stop().await;
                (network, result)
            }));
        }
        let Some(cleanup) = self.transport_cleanup.as_mut() else {
            return Ok(());
        };
        let outcome = cleanup.await;
        self.transport_cleanup = None;
        match outcome {
            Ok((network, result)) => {
                if result.is_err() {
                    self.network = Some(network);
                }
                result
            }
            Err(error) => {
                let message = format!("transport cleanup task failed: {error}");
                self.transport_cleanup_error = Some(message.clone());
                Err(cleanup_failed(message))
            }
        }
    }
}

impl<T: TransportPort> Drop for BACnetServer<T> {
    /// Seal the server and abort its tasks. In async code the object
    /// database then drops on Tokio's blocking pool (#1409): the drop returns
    /// before the objects' last saves land, so storage may still change after
    /// it. Call [`stop`](BACnetServer::stop) first to wait for them. A clone
    /// of [`database`](BACnetServer::database) the application still holds
    /// keeps the objects alive; let go of it with
    /// [`drop_database_off_runtime`] (#1513).
    fn drop(&mut self) {
        let tasks = self.seal_and_abort();
        // A node that took the handle back left nothing to let go of.
        if !self.database_handed_back {
            self.let_database_go(tasks);
        }
    }
}

impl<T: TransportPort> BACnetServer<T> {
    /// Seal the server, as a drop and the node's
    /// [`__endpoint_into_database`](BACnetServer::__endpoint_into_database)
    /// both do first.
    fn seal(&mut self) {
        if let Some(network) = self.network.as_ref().filter(|_| self.owns_link()) {
            network.seal_responses();
        }
        self.comm_state.release_node();
        self.broadcaster.seal();
        self.notification_transactions
            .application_sealed
            .store(true, Ordering::Release);
        if let Some(runtime) = &self.target_audit {
            runtime.seal();
            drop(runtime.batches.close());
        }
        self.request_tasks.close();
        self.notification_transactions.close();
        // The lanes' tasks hold the database and the network through the
        // transactions they belong to; sealing aborts them (#2537), and
        // they are joined, or with no runtime dropped, before the database
        // goes.
        super::event_send::lanes(&self.notification_transactions).seal();
    }

    /// The server's own tasks, each in its slot.
    fn task_slots(&mut self) -> [&mut Option<JoinHandle<()>>; 10] {
        [
            &mut self.dispatch_task,
            &mut self.network_number_task,
            &mut self.cov_purge_task,
            &mut self.fault_detection_task,
            &mut self.event_enrollment_task,
            &mut self.trend_log_task,
            &mut self.schedule_tick_task,
            &mut self.intrinsic_reporting_task,
            &mut self.binary_lighting_operation_task,
            &mut self.cov_revisit_task,
        ]
    }

    /// What a drop does first: seal the server and abort its tasks, which
    /// it returns to be joined before the database goes.
    fn seal_and_abort(&mut self) -> Vec<JoinHandle<()>> {
        self.seal();
        let mut tasks: Vec<JoinHandle<()>> = self
            .task_slots()
            .into_iter()
            .filter_map(Option::take)
            .collect();
        if let Ok(mut timer) = self.dcc_timer.try_lock() {
            tasks.extend(timer.take());
        }
        for task in &tasks {
            task.abort();
        }
        tasks
    }

    /// Let the object database go off the async runtime as the server is
    /// dropped (#1409).
    ///
    /// Dropping the database drops its objects, and a durable object waits
    /// for the saves it has queued, after putting storage back to the state
    /// it serves when a write is still staged (#1363). A server dropped in
    /// async code without `stop()` would block a Tokio worker for as long as
    /// storage takes. So when a runtime is current, the server's handle goes
    /// to a task that waits until `tasks`, aborted, and the server's
    /// request, notification and event lane tasks have let go of theirs;
    /// then it lets go of the handle through [`drop_database_off_runtime`],
    /// so the database drops on the blocking pool if nothing else holds it,
    /// as DeleteObject drops a removed object. The server's drop returns
    /// first, so storage may still change after it. An application still
    /// holding the database lets go of the last handle itself, the same way
    /// (#1513). With no runtime current, the database drops here, as it
    /// always has, and the sealed lanes' tasks are dropped, which aborts
    /// them (#2537).
    ///
    /// A DCC timer the drop could not take, because a request or the timer's
    /// own expiry held its slot, is cancelled here once the requests are
    /// joined: with Audit reporting, it holds the database too (#1560).
    fn let_database_go(&mut self, tasks: Vec<JoinHandle<()>>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            // Dropping the sealed lanes' set aborts what is left of them.
            if let Some(mut lanes) = super::event_send::made_lanes(&self.notification_transactions)
            {
                drop(lanes.take_tasks());
            }
            return;
        };
        let db = std::mem::take(&mut self.db);
        let joined = join_released(
            tasks,
            Arc::clone(&self.request_tasks),
            Arc::clone(&self.notification_transactions),
            Arc::clone(&self.dcc_timer),
        );
        runtime.spawn(async move {
            joined.await;
            drop(drop_database_off_runtime(db));
        });
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// The node's (#2766): a server on a node's seat that the node has
    /// stopped hands back its handle on the database once what a drop
    /// would join has let go of theirs, where a drop leaves that to a task.
    /// The node's own handle is then the only one, so its refused start can
    /// give the database to its caller. Cancelled before it returns, the
    /// server drops as any server does.
    #[doc(hidden)]
    pub async fn __endpoint_into_database(mut self) -> Arc<RwLock<ObjectDatabase>> {
        self.retire().await;
        self.database_handed_back = true;
        Arc::clone(&self.db)
    }

    /// Seal the server, abort its tasks and join what a drop would, each
    /// task kept in its slot until it has ended, so a call cancelled part
    /// way leaves what it hasn't joined to the drop, which joins it before
    /// the database goes (#2766).
    pub(super) async fn retire(&mut self) {
        self.seal();
        for task in self.task_slots().into_iter().flatten() {
            task.abort();
        }
        for slot in self.task_slots() {
            stop_producer(slot).await;
        }
        while let Some(result) = self.request_tasks.join_next().await {
            super::request_tasks::RequestTasks::observe(Some(result));
        }
        // No request is left to hold the slot, as in a drop's join: only the
        // timer, which the join aborts, can want it across the join (#2781).
        {
            let mut timer = self.dcc_timer.lock().await;
            super::dcc_timer::cancel(&mut timer).await;
        }
        while let Some(result) = self.notification_transactions.join_next().await {
            NotificationTransactions::observe(Some(result));
        }
        super::event_send::join_sealed(&self.notification_transactions).await;
    }
}

/// What a dropped server's handle on the database waits for: `tasks`,
/// aborted, then its request, DCC timer, notification and event lane tasks,
/// each of which may hold a handle of its own.
async fn join_released(
    tasks: Vec<JoinHandle<()>>,
    requests: Arc<super::request_tasks::RequestTasks>,
    notifications: Arc<NotificationTransactions>,
    dcc_timer: Arc<Mutex<super::dcc_timer::TimerSlot>>,
) {
    for task in tasks {
        let _ = task.await;
    }
    while let Some(result) = requests.join_next().await {
        super::request_tasks::RequestTasks::observe(Some(result));
    }
    // No request is left to hold the slot. The guard goes before the join,
    // as the timer may be queued for the slot itself. A timer queued ahead of
    // this wait, as an aborted request let go, makes its expiry first, and
    // the join finds it ended (#2781).
    let timer = dcc_timer.lock().await.take();
    if let Some(timer) = timer {
        timer.abort();
        let _ = timer.await;
    }
    while let Some(result) = notifications.join_next().await {
        NotificationTransactions::observe(Some(result));
    }
    // The lanes were sealed at the drop; their aborted tasks let go of their
    // database handles as they are joined.
    super::event_send::join_sealed(&notifications).await;
}

/// Let go of a handle on an object database without parking an async
/// runtime's worker thread (#1513).
///
/// The last handle to go drops every object, and a durable object (a
/// Notification Forwarder, Notification Class, Access Rights object or Audit
/// Log with storage) waits as it drops for the saves it has queued, after
/// putting storage back to the state it serves if a write is still staged
/// (#1363). Dropped in async code, that wait would hold a Tokio worker for as
/// long as storage takes. So if `db` is the last handle and a Tokio runtime is
/// current, the database drops on the runtime's blocking pool, and the
/// returned task finishes once it has. Any other handle just lets go and
/// returns `None`, as does a last one with no runtime current, which drops
/// the database on this thread. A runtime that is shutting down runs nothing
/// more on its blocking pool: Tokio shuts the new task down at once, and the
/// database drops on this thread then too.
///
/// A dropped server lets go of its own handle this way once its tasks are
/// done, whether or not it was stopped, and so do the tasks a `stop()`
/// leaves to settle staged writes and end runs once the application lets
/// go of the database. Any of them may still hold a handle when the
/// application lets go of its clone of [`BACnetServer::database`], so
/// `None` is the usual outcome. What waits until storage holds the state
/// each object serves is [`stop`](BACnetServer::stop), not this. This keeps
/// the application's own handle, should it be the last, from dropping the
/// objects on a runtime worker: in async code, release it here rather than
/// dropping it.
///
/// ```no_run
/// # use bacnet_transport::bip::BipTransport;
/// # async fn demo(mut server: bacnet_server::server::BACnetServer<BipTransport>) {
/// use bacnet_server::server::drop_database_off_runtime;
///
/// let db = std::sync::Arc::clone(server.database());
/// // ... use the database while the server runs ...
/// // stop() waits until storage holds what each object serves.
/// let _ = server.stop().await;
/// drop(server);
/// // The server's own tasks may still hold the database; whichever handle
/// // goes last, it drops off the runtime.
/// drop(drop_database_off_runtime(db));
/// # }
/// ```
pub fn drop_database_off_runtime(db: Arc<RwLock<ObjectDatabase>>) -> Option<JoinHandle<()>> {
    // `Arc::into_inner` hands the database to exactly one of two handles let
    // go of at once, where a count check followed by a drop could leave both
    // thinking the other was last.
    let db = Arc::into_inner(db)?;
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        drop(db);
        return None;
    };
    Some(runtime.spawn_blocking(move || drop(db)))
}

/// A stop whose transport cleanup task panicked or was aborted: the server
/// is shut for good, with the reason in the text (#1833).
fn cleanup_failed(message: String) -> Error {
    Error::State(Lifecycle::Closed, Refusal::text(message))
}

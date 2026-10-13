use super::*;

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
    /// [`DurableWrites::settle_forgotten_writes`]. That wait has no limit:
    /// storage that stalls holds stop up, and a warning naming the objects
    /// still saving is logged after 5 s and every 30 s after that. Stop does
    /// not wait while the application holds the database: the objects then
    /// settle from a task once it lets go, and put storage back when they
    /// are dropped. That task lets go of its handle through
    /// [`drop_database_off_runtime`], so if it holds the last one the
    /// objects drop on Tokio's blocking pool.
    ///
    /// A request's abort lands when its task next yields, so a request
    /// already running when stop begins may find its save done and make its
    /// write first. Its answer is sealed, but the object serves that write,
    /// and storage holds it (#1457).
    ///
    /// [`DurableWrites::settle_forgotten_writes`]: bacnet_objects::durable::DurableWrites::settle_forgotten_writes
    ///
    /// Cancelling this waiter leaves cleanup owned by the server. A later stop
    /// joins it; after transport cleanup begins, dropping the server lets that
    /// cleanup finish. Local mutation and broadcasts are rejected from the first
    /// stop poll; local reads and database inspection remain available.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(network) = &self.network {
            network.seal_responses();
        }
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
        {
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
        // No request is left to take or release a staged save, and the
        // operation task that would drop one is gone: put storage back to
        // what each object serves before stop returns (#1363). A
        // `write_local` still in flight isn't joined; if it staged, it finds
        // its stage gone and saves in place under the guard.
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
            return Err(Error::Encoding(error.clone()));
        }
        if self.transport_cleanup.is_none() {
            if let Some(network) = self.network.take() {
                let mut network = match Arc::try_unwrap(network) {
                    Ok(network) => network,
                    Err(network) => {
                        self.network = Some(network);
                        return Err(Error::Encoding(
                            "server network still has an internal owner".into(),
                        ));
                    }
                };
                // Own the complete future, including cancellation-unsafe custom
                // transport cleanup. Drop deliberately leaves this task running.
                self.transport_cleanup = Some(tokio::spawn(async move {
                    let result = network.stop().await;
                    (network, result)
                }));
            }
        }
        let Some(cleanup) = self.transport_cleanup.as_mut() else {
            return Ok(());
        };
        let outcome = cleanup.await;
        self.transport_cleanup = None;
        match outcome {
            Ok((network, result)) => {
                if result.is_err() {
                    self.network = Some(Arc::new(network));
                }
                result
            }
            Err(error) => {
                let message = format!("transport cleanup task failed: {error}");
                self.transport_cleanup_error = Some(message.clone());
                Err(Error::Encoding(message))
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
        if let Some(network) = &self.network {
            network.seal_responses();
        }
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
        let mut tasks: Vec<JoinHandle<()>> = [
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
        .into_iter()
        .filter_map(Option::take)
        .collect();
        if let Ok(mut timer) = self.dcc_timer.try_lock() {
            tasks.extend(timer.take());
        }
        for task in &tasks {
            task.abort();
        }
        self.let_database_go(tasks);
    }
}

impl<T: TransportPort> BACnetServer<T> {
    /// Let the object database go off the async runtime as the server is
    /// dropped (#1409).
    ///
    /// Dropping the database drops its objects, and a durable object waits
    /// for the saves it has queued, after putting storage back to the state
    /// it serves when a write is still staged (#1363). A server dropped in
    /// async code without `stop()` would block a Tokio worker for as long as
    /// storage takes. So when a runtime is current, the server's handle goes
    /// to a task that waits until `tasks`, aborted, and the server's request
    /// and notification tasks have let go of theirs; then it lets go of the
    /// handle through [`drop_database_off_runtime`], so the database drops
    /// on the blocking pool if nothing else holds it, as DeleteObject drops a
    /// removed object. The server's drop returns first, so storage may still
    /// change after it. An application still holding the database lets go
    /// of the last handle itself, the same way (#1513). With no runtime
    /// current, the database drops here, as it always has.
    ///
    /// A DCC timer the drop could not take, because a request or the timer's
    /// own expiry held its slot, is cancelled here once the requests are
    /// joined: with Audit reporting, it holds the database too (#1560).
    fn let_database_go(&mut self, tasks: Vec<JoinHandle<()>>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let db = std::mem::take(&mut self.db);
        let requests = Arc::clone(&self.request_tasks);
        let notifications = Arc::clone(&self.notification_transactions);
        let dcc_timer = Arc::clone(&self.dcc_timer);
        runtime.spawn(async move {
            for task in tasks {
                let _ = task.await;
            }
            while let Some(result) = requests.join_next().await {
                super::request_tasks::RequestTasks::observe(Some(result));
            }
            // No request is left to hold the slot. The guard goes before the
            // join, as the timer may be queued for the slot itself.
            let timer = dcc_timer.lock().await.take();
            if let Some(timer) = timer {
                timer.abort();
                let _ = timer.await;
            }
            while let Some(result) = notifications.join_next().await {
                NotificationTransactions::observe(Some(result));
            }
            drop(drop_database_off_runtime(db));
        });
    }
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

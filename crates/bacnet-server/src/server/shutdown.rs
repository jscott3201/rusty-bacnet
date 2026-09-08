use super::*;

#[cfg(test)]
#[path = "request_tasks_tests.rs"]
mod request_tasks_tests;

#[cfg(test)]
#[path = "producer_shutdown_tests.rs"]
mod producer_shutdown_tests;

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
    /// Stop the server.
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.request_tasks.close();
        // Seal both reservations and worker admission before quiescing any
        // producer; abort also interrupts workers suspended in async send.
        self.notification_transactions.close();
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
        // Dispatch has relinquished the sole join-consumer role. Retain the
        // set in self across await so a cancelled stop can finish this drain.
        while let Some(result) = self.notification_transactions.join_next().await {
            NotificationTransactions::observe(Some(result));
        }
        Ok(())
    }
}

impl<T: TransportPort> Drop for BACnetServer<T> {
    fn drop(&mut self) {
        self.notification_transactions.close();
    }
}

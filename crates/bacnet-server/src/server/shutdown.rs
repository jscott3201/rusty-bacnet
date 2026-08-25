use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Stop the server.
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.notification_transactions.close();
        if let Some(task) = self.fault_detection_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.event_enrollment_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.trend_log_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.schedule_tick_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.intrinsic_reporting_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.cov_purge_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.dispatch_task.take() {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }
}

impl<T: TransportPort> Drop for BACnetServer<T> {
    fn drop(&mut self) {
        self.notification_transactions.close();
    }
}

use super::*;
use std::sync::Mutex as StdMutex;
use tokio::sync::oneshot;
use tokio::time::{sleep_until, Instant as TokioInstant};

const DEFAULT_COV_RENEWAL_MARGIN: Duration = Duration::from_secs(30);
const DEFAULT_COV_RENEWAL_EVENT_CAPACITY: usize = 16;

/// Options for a managed finite COV subscription.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ManagedCOVSubscriptionOptions {
    /// How long before the known remaining lifetime the helper should renew.
    ///
    /// The helper schedules renewal at `time_remaining - renewal_margin`, using
    /// the requested lifetime until inbound notifications provide a fresher
    /// `time_remaining` value.
    pub renewal_margin: Duration,
    /// Capacity of the renewal event broadcast channel.
    ///
    /// Accepted range is `1..=MAX_COV_CHANNEL_CAPACITY`.
    pub event_channel_capacity: usize,
}

impl Default for ManagedCOVSubscriptionOptions {
    fn default() -> Self {
        Self {
            renewal_margin: DEFAULT_COV_RENEWAL_MARGIN,
            event_channel_capacity: DEFAULT_COV_RENEWAL_EVENT_CAPACITY,
        }
    }
}

impl ManagedCOVSubscriptionOptions {
    /// Set the renewal margin.
    pub fn with_renewal_margin(mut self, margin: Duration) -> Self {
        self.renewal_margin = margin;
        self
    }

    /// Set the renewal event broadcast channel capacity.
    pub fn with_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.event_channel_capacity = capacity;
        self
    }

    fn validate(&self, lifetime: u32) -> Result<(), Error> {
        if !(1..=MAX_COV_CHANNEL_CAPACITY).contains(&self.event_channel_capacity) {
            return Err(Error::Encoding(format!(
                "invalid managed COV subscription event channel capacity {}; expected 1..={}",
                self.event_channel_capacity, MAX_COV_CHANNEL_CAPACITY
            )));
        }
        if self.renewal_margin >= Duration::from_secs(u64::from(lifetime)) {
            return Err(Error::Encoding(
                "managed COV subscription renewal margin must be shorter than lifetime".into(),
            ));
        }
        Ok(())
    }

    fn renewal_delay(&self, time_remaining: u32) -> Duration {
        Duration::from_secs(u64::from(time_remaining)).saturating_sub(self.renewal_margin)
    }
}

/// Events emitted by a managed finite COV subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManagedCOVSubscriptionEvent {
    /// A matching COV notification supplied a new lifetime observation.
    NotificationObserved {
        /// The `time_remaining` field from the inbound notification.
        time_remaining: u32,
        /// Delay until the next renewal after applying the configured margin.
        renew_after: Duration,
    },
    /// The observed lifetime is inside the configured renewal margin.
    ImpendingExpiry {
        /// The `time_remaining` field from the inbound notification.
        time_remaining: u32,
    },
    /// The helper successfully renewed the subscription.
    Renewed {
        /// Lifetime requested on the renewed SubscribeCOV request.
        requested_lifetime: u32,
        /// Delay until the next scheduled renewal after applying the margin.
        renew_after: Duration,
    },
    /// A renewal SubscribeCOV request failed and the helper stopped.
    RenewalFailed {
        /// Display form of the protocol, reject, abort, transport, or timeout error.
        error: String,
    },
}

/// Handle for a managed finite COV subscription renewal task.
pub struct ManagedCOVSubscription {
    events_tx: broadcast::Sender<ManagedCOVSubscriptionEvent>,
    last_event: Arc<StdMutex<Option<ManagedCOVSubscriptionEvent>>>,
    stop_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl ManagedCOVSubscription {
    /// Subscribe to lifetime observations and renewal outcomes.
    pub fn events(&self) -> broadcast::Receiver<ManagedCOVSubscriptionEvent> {
        self.events_tx.subscribe()
    }

    /// Return the latest observed renewal event, if any.
    pub fn last_event(&self) -> Option<ManagedCOVSubscriptionEvent> {
        self.last_event.lock().ok().and_then(|event| event.clone())
    }

    /// Stop renewing and wait for the renewal task to exit.
    pub async fn stop(mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    /// Return whether the renewal task has already finished.
    pub fn is_finished(&self) -> bool {
        match &self.task {
            Some(task) => task.is_finished(),
            None => true,
        }
    }
}

impl Drop for ManagedCOVSubscription {
    fn drop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Start a managed finite SubscribeCOV renewal task for a directly reachable device.
    ///
    /// The helper sends the initial SubscribeCOV request, then renews the same
    /// subscription before expiry. It also listens to [`Self::cov_notifications`]
    /// and moves the renewal earlier when matching notifications report a lower
    /// `time_remaining` than the requested lifetime-based schedule.
    ///
    /// This method takes `Arc<Self>` so the renewal task can keep the client alive.
    /// Call it as `Arc::clone(&client).manage_cov_subscription(...)`.
    pub async fn manage_cov_subscription(
        self: Arc<Self>,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: bacnet_types::primitives::ObjectIdentifier,
        confirmed: bool,
        lifetime: u32,
        options: ManagedCOVSubscriptionOptions,
    ) -> Result<ManagedCOVSubscription, Error> {
        if lifetime == 0 {
            return Err(Error::Encoding(
                "managed COV subscriptions require a finite non-zero lifetime".into(),
            ));
        }
        options.validate(lifetime)?;

        let destination_mac = MacAddr::from_slice(destination_mac);
        let mut notifications = self.cov_notifications();

        self.subscribe_cov(
            &destination_mac,
            subscriber_process_identifier,
            monitored_object_identifier,
            confirmed,
            Some(lifetime),
        )
        .await?;

        let (events_tx, _) = broadcast::channel(options.event_channel_capacity);
        let events_tx_for_task = events_tx.clone();
        let last_event = Arc::new(StdMutex::new(None));
        let last_event_for_task = Arc::clone(&last_event);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let client = Arc::clone(&self);
        let mut renew_at = TokioInstant::now() + options.renewal_delay(lifetime);

        let task = tokio::spawn(async move {
            loop {
                let renew_sleep = sleep_until(renew_at);
                tokio::pin!(renew_sleep);
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = &mut renew_sleep => {
                        let renewal = client.subscribe_cov(
                            &destination_mac,
                            subscriber_process_identifier,
                            monitored_object_identifier,
                            confirmed,
                            Some(lifetime),
                        );
                        tokio::pin!(renewal);
                        tokio::select! {
                            _ = &mut stop_rx => break,
                            result = &mut renewal => match result {
                                Ok(()) => {
                                    let renew_after = options.renewal_delay(lifetime);
                                    publish_managed_cov_event(
                                        &events_tx_for_task,
                                        &last_event_for_task,
                                        ManagedCOVSubscriptionEvent::Renewed {
                                            requested_lifetime: lifetime,
                                            renew_after,
                                        },
                                    );
                                    renew_at = TokioInstant::now() + renew_after;
                                }
                                Err(error) => {
                                    publish_managed_cov_event(
                                        &events_tx_for_task,
                                        &last_event_for_task,
                                        ManagedCOVSubscriptionEvent::RenewalFailed {
                                            error: error.to_string(),
                                        },
                                    );
                                    break;
                                }
                            },
                        }
                    }
                    received = notifications.recv() => {
                        let received = match received {
                            Ok(received) => received,
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        };
                        if !matches_managed_cov_notification(
                            &received,
                            &destination_mac,
                            subscriber_process_identifier,
                            monitored_object_identifier,
                        ) {
                            continue;
                        }

                        let time_remaining = received.notification.time_remaining;
                        let renew_after = options.renewal_delay(time_remaining);
                        publish_managed_cov_event(
                            &events_tx_for_task,
                            &last_event_for_task,
                            ManagedCOVSubscriptionEvent::NotificationObserved {
                                time_remaining,
                                renew_after,
                            },
                        );

                        let observed_renew_at = TokioInstant::now() + renew_after;
                        if renew_after.is_zero() {
                            publish_managed_cov_event(
                                &events_tx_for_task,
                                &last_event_for_task,
                                ManagedCOVSubscriptionEvent::ImpendingExpiry {
                                    time_remaining,
                                },
                            );
                            renew_at = observed_renew_at;
                        } else if observed_renew_at < renew_at {
                            renew_at = observed_renew_at;
                        }
                    }
                }
            }
        });

        Ok(ManagedCOVSubscription {
            events_tx,
            last_event,
            stop_tx: Some(stop_tx),
            task: Some(task),
        })
    }
}

fn publish_managed_cov_event(
    events_tx: &broadcast::Sender<ManagedCOVSubscriptionEvent>,
    last_event: &StdMutex<Option<ManagedCOVSubscriptionEvent>>,
    event: ManagedCOVSubscriptionEvent,
) {
    if let Ok(mut last_event) = last_event.lock() {
        *last_event = Some(event.clone());
    }
    let _ = events_tx.send(event);
}

fn matches_managed_cov_notification(
    received: &ReceivedCOVNotification,
    destination_mac: &[u8],
    subscriber_process_identifier: u32,
    monitored_object_identifier: bacnet_types::primitives::ObjectIdentifier,
) -> bool {
    received.notification.subscriber_process_identifier == subscriber_process_identifier
        && received.notification.monitored_object_identifier == monitored_object_identifier
        && received.source_network.is_none()
        && received.source_address.is_none()
        && &received.source_mac[..] == destination_mac
}

//! Owned handle bundles the request dispatch path passes around.
use super::*;

/// The server handles a confirmed-request task owns: object database,
/// network, COV state, segmented-send state, notification and DCC state, the
/// record of received confirmed event notifications, the mutation decision
/// log and the server config. Cloning clones each `Arc`, so
/// a spawned request task takes exactly the handles it needs to outlive
/// dispatch.
pub(super) struct RequestServices<T: TransportPort + 'static> {
    pub(super) db: Arc<RwLock<ObjectDatabase>>,
    pub(super) network: Arc<NetworkLayer<T>>,
    pub(super) cov_table: Arc<RwLock<CovSubscriptionTable>>,
    pub(super) seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    pub(super) seg_send_permits: Arc<Semaphore>,
    pub(super) cov_in_flight: Arc<Semaphore>,
    pub(super) learned_routers: Arc<Mutex<LearnedRouterCache>>,
    pub(super) notification_transactions: Arc<NotificationTransactions>,
    pub(super) device_bindings: Arc<RwLock<DeviceBindingTable>>,
    pub(super) comm_state: Arc<CommState>,
    pub(super) dcc_timer: Arc<Mutex<dcc_timer::TimerSlot>>,
    pub(super) dcc_outcomes: Arc<dcc_outcomes::DccOutcomes>,
    pub(super) event_suppressions: Arc<super::event_suppression::EventSuppressions>,
    pub(super) confirmed_event_repeats: Arc<super::event_forwarding_repeats::ConfirmedEventRepeats>,
    pub(super) received_event_log: Arc<super::received_event_log::ReceivedEventLog>,
    pub(super) mutation_decisions: Arc<crate::mutation::MutationDecisions>,
    pub(super) config: Arc<ServerConfig>,
}

impl<T: TransportPort + 'static> Clone for RequestServices<T> {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            network: Arc::clone(&self.network),
            cov_table: Arc::clone(&self.cov_table),
            seg_ack_senders: Arc::clone(&self.seg_ack_senders),
            seg_send_permits: Arc::clone(&self.seg_send_permits),
            cov_in_flight: Arc::clone(&self.cov_in_flight),
            learned_routers: Arc::clone(&self.learned_routers),
            notification_transactions: Arc::clone(&self.notification_transactions),
            device_bindings: Arc::clone(&self.device_bindings),
            comm_state: Arc::clone(&self.comm_state),
            dcc_timer: Arc::clone(&self.dcc_timer),
            dcc_outcomes: Arc::clone(&self.dcc_outcomes),
            event_suppressions: Arc::clone(&self.event_suppressions),
            confirmed_event_repeats: Arc::clone(&self.confirmed_event_repeats),
            received_event_log: Arc::clone(&self.received_event_log),
            mutation_decisions: Arc::clone(&self.mutation_decisions),
            config: Arc::clone(&self.config),
        }
    }
}

/// The server handles an unconfirmed-request task owns. The COV handles and
/// the task spawner are WriteGroup's: its Channel writes notify like any
/// local write, and the distributions they queue run beside the request.
pub(super) struct UnconfirmedServices<T: TransportPort + 'static> {
    pub(super) db: Arc<RwLock<ObjectDatabase>>,
    pub(super) network: Arc<NetworkLayer<T>>,
    pub(super) config: Arc<ServerConfig>,
    pub(super) clock: Option<Arc<ServerClock>>,
    pub(super) comm_state: Arc<CommState>,
    pub(super) device_bindings: Arc<RwLock<DeviceBindingTable>>,
    pub(super) discovery_limiter: Arc<DiscoveryLimiter>,
    pub(super) time_sync_limiter: Arc<TimeSyncLimiter>,
    pub(super) notification_transactions: Arc<NotificationTransactions>,
    pub(super) learned_routers: Arc<Mutex<LearnedRouterCache>>,
    pub(super) event_suppressions: Arc<super::event_suppression::EventSuppressions>,
    pub(super) received_event_log: Arc<super::received_event_log::ReceivedEventLog>,
    pub(super) cov_table: Arc<RwLock<CovSubscriptionTable>>,
    pub(super) cov_in_flight: Arc<Semaphore>,
    pub(super) tasks: request_tasks::RequestTaskSpawner,
    /// Counts the WriteGroup Channel-write decisions (#1319).
    pub(super) mutation_decisions: Arc<crate::mutation::MutationDecisions>,
}

/// Everything the dispatch loop hands to [`BACnetServer::dispatch`]: the
/// confirmed-request handles, plus the state only admission needs (the
/// confirmed-request tracker, clock, request limiters and request task set).
pub(super) struct DispatchContext<T: TransportPort + 'static> {
    pub(super) services: RequestServices<T>,
    pub(super) confirmed_request_tracker: Arc<ConfirmedRequestTracker>,
    pub(super) clock: Option<Arc<ServerClock>>,
    pub(super) discovery_limiter: Arc<DiscoveryLimiter>,
    pub(super) time_sync_limiter: Arc<TimeSyncLimiter>,
    pub(super) request_tasks: Arc<request_tasks::RequestTasks>,
}

impl<T: TransportPort + 'static> DispatchContext<T> {
    /// Clone the handles an unconfirmed-request task needs.
    pub(super) fn unconfirmed_services(&self) -> UnconfirmedServices<T> {
        UnconfirmedServices {
            db: Arc::clone(&self.services.db),
            network: Arc::clone(&self.services.network),
            config: Arc::clone(&self.services.config),
            clock: self.clock.clone(),
            comm_state: Arc::clone(&self.services.comm_state),
            device_bindings: Arc::clone(&self.services.device_bindings),
            discovery_limiter: Arc::clone(&self.discovery_limiter),
            time_sync_limiter: Arc::clone(&self.time_sync_limiter),
            notification_transactions: Arc::clone(&self.services.notification_transactions),
            learned_routers: Arc::clone(&self.services.learned_routers),
            event_suppressions: Arc::clone(&self.services.event_suppressions),
            received_event_log: Arc::clone(&self.services.received_event_log),
            cov_table: Arc::clone(&self.services.cov_table),
            cov_in_flight: Arc::clone(&self.services.cov_in_flight),
            tasks: self.request_tasks.spawner(),
            mutation_decisions: Arc::clone(&self.services.mutation_decisions),
        }
    }
}

impl<T: TransportPort + 'static> UnconfirmedServices<T> {
    /// The EventNotification delivery view of these handles, as
    /// [`RequestServices::event_delivery`] gives it.
    pub(super) fn event_delivery(&self) -> super::event_delivery::EventDelivery<'_, T> {
        super::event_delivery::EventDelivery {
            db: &self.db,
            network: &self.network,
            comm_state: &self.comm_state,
            learned_routers: &self.learned_routers,
            notification_transactions: &self.notification_transactions,
            device_bindings: &self.device_bindings,
            suppressions: &self.event_suppressions,
            retry_timeout_ms: self.config.cov_retry_timeout_ms,
            local_apdu_capacity: self.config.max_apdu_length,
        }
    }
}

/// Who sent a confirmed request and how to reply: the requester's MAC, its
/// network address when routed, and the route the request arrived on.
pub(super) struct RequestOrigin<'a> {
    pub(super) mac: &'a [u8],
    pub(super) network: Option<NpduAddress>,
    pub(super) route: bacnet_network::response_route::ResponseRoute,
}

impl<T: TransportPort + 'static> RequestServices<T> {
    /// The EventNotification delivery view of these handles, with the
    /// confirmed retry timeout and APDU capacity taken from the config.
    pub(super) fn event_delivery(&self) -> super::event_delivery::EventDelivery<'_, T> {
        super::event_delivery::EventDelivery {
            db: &self.db,
            network: &self.network,
            comm_state: &self.comm_state,
            learned_routers: &self.learned_routers,
            notification_transactions: &self.notification_transactions,
            device_bindings: &self.device_bindings,
            suppressions: &self.event_suppressions,
            retry_timeout_ms: self.config.cov_retry_timeout_ms,
            local_apdu_capacity: self.config.max_apdu_length,
        }
    }

    /// The tables a read samples the Device's server-owned lists from.
    pub(super) fn live_tables(&self) -> super::requests::confirmed_response::LiveTables<'_> {
        super::requests::confirmed_response::LiveTables {
            cov: &self.cov_table,
            bindings: &self.device_bindings,
        }
    }
}

#[cfg(test)]
impl<T: TransportPort + 'static> RequestServices<T> {
    /// Fresh, empty handles around `network` and `config`. Tests overwrite the
    /// fields whose state they share or inspect.
    pub(super) fn for_test(
        network: Arc<NetworkLayer<T>>,
        config: impl Into<Arc<ServerConfig>>,
    ) -> Self {
        Self {
            db: Arc::new(RwLock::new(ObjectDatabase::new())),
            network,
            cov_table: Arc::new(RwLock::new(CovSubscriptionTable::new())),
            seg_ack_senders: Arc::new(segmented_send::SegmentedSendRegistry::default()),
            seg_send_permits: Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
            cov_in_flight: Arc::new(Semaphore::new(1)),
            learned_routers: Arc::new(Mutex::new(LearnedRouterCache::new())),
            notification_transactions: NotificationTransactions::new(),
            device_bindings: Arc::new(RwLock::new(DeviceBindingTable::new())),
            comm_state: Arc::default(),
            dcc_timer: Arc::new(Mutex::new(dcc_timer::TimerSlot::default())),
            dcc_outcomes: Arc::new(dcc_outcomes::DccOutcomes::default()),
            event_suppressions: Arc::default(),
            confirmed_event_repeats: Arc::default(),
            received_event_log: Arc::default(),
            mutation_decisions: Arc::new(crate::mutation::MutationDecisions::default()),
            config: config.into(),
        }
    }
}

#[cfg(test)]
impl<T: TransportPort + 'static> DispatchContext<T> {
    /// Dispatch context around `services` with no clock, default limiters, a
    /// fresh confirmed-request tracker and a fresh request task set.
    pub(super) fn for_test(services: RequestServices<T>) -> Self {
        Self {
            services,
            confirmed_request_tracker: Arc::new(ConfirmedRequestTracker::default()),
            clock: None,
            discovery_limiter: Arc::new(DiscoveryLimiter::new(DiscoveryPolicy::default(), None)),
            time_sync_limiter: Arc::new(TimeSyncLimiter::new(TimeSyncPolicy::default())),
            request_tasks: Arc::new(request_tasks::RequestTasks::default()),
        }
    }
}

#[cfg(test)]
impl<T: TransportPort + 'static> UnconfirmedServices<T> {
    /// Fresh, empty handles around `network` and `config`, with no clock and
    /// default limiters. Tests overwrite the fields they share or tune. The
    /// task spawner's owner is gone, so a WriteGroup's distributions don't
    /// start.
    pub(super) fn for_test(
        network: Arc<NetworkLayer<T>>,
        config: impl Into<Arc<ServerConfig>>,
    ) -> Self {
        Self {
            db: Arc::new(RwLock::new(ObjectDatabase::new())),
            network,
            config: config.into(),
            clock: None,
            comm_state: Arc::default(),
            device_bindings: Arc::new(RwLock::new(DeviceBindingTable::new())),
            discovery_limiter: Arc::new(DiscoveryLimiter::new(DiscoveryPolicy::default(), None)),
            time_sync_limiter: Arc::new(TimeSyncLimiter::new(TimeSyncPolicy::default())),
            mutation_decisions: Arc::default(),
            notification_transactions: NotificationTransactions::new(),
            learned_routers: Arc::new(Mutex::new(LearnedRouterCache::new())),
            event_suppressions: Arc::default(),
            received_event_log: Arc::default(),
            cov_table: Arc::new(RwLock::new(CovSubscriptionTable::new())),
            cov_in_flight: Arc::new(Semaphore::new(1)),
            tasks: Arc::new(request_tasks::RequestTasks::default()).spawner(),
        }
    }
}

#[cfg(test)]
impl<T: TransportPort + 'static> BACnetServer<T> {
    /// The server's own config, for a test to change once the server runs.
    /// It is shared (#1521), so the first change copies it: the server's
    /// local writes and the handles taken from it afterwards see the change,
    /// and dispatch and the tasks already started keep the config they were
    /// started with.
    pub(super) fn config_mut(&mut self) -> &mut ServerConfig {
        Arc::make_mut(&mut self.config)
    }

    /// The retained server's own confirmed-request handles.
    pub(super) fn test_services(&self) -> RequestServices<T> {
        RequestServices {
            db: Arc::clone(&self.db),
            network: Arc::clone(self.test_network()),
            cov_table: Arc::clone(&self.cov_table),
            seg_ack_senders: Arc::clone(&self.seg_ack_senders),
            seg_send_permits: Arc::clone(&self.seg_send_permits),
            cov_in_flight: Arc::clone(&self.cov_in_flight),
            learned_routers: Arc::clone(&self.learned_routers),
            notification_transactions: Arc::clone(&self.notification_transactions),
            device_bindings: Arc::clone(&self.device_bindings),
            comm_state: Arc::clone(&self.comm_state),
            dcc_timer: Arc::clone(&self.dcc_timer),
            dcc_outcomes: Arc::clone(&self.dcc_outcomes),
            event_suppressions: Arc::clone(&self.event_suppressions),
            // The server keeps no handle on the dispatch loop's record, so
            // these handles start their own.
            confirmed_event_repeats: Arc::default(),
            received_event_log: Arc::default(),
            mutation_decisions: Arc::clone(&self.mutation_decisions),
            config: Arc::clone(&self.config),
        }
    }

    /// The retained server's own unconfirmed-request handles.
    pub(super) fn test_unconfirmed_services(&self) -> UnconfirmedServices<T> {
        UnconfirmedServices {
            db: Arc::clone(&self.db),
            network: Arc::clone(self.test_network()),
            config: Arc::clone(&self.config),
            clock: self._clock.clone(),
            comm_state: Arc::clone(&self.comm_state),
            device_bindings: Arc::clone(&self.device_bindings),
            discovery_limiter: Arc::clone(&self.discovery_limiter),
            time_sync_limiter: Arc::clone(&self.time_sync_limiter),
            notification_transactions: Arc::clone(&self.notification_transactions),
            learned_routers: Arc::clone(&self.learned_routers),
            event_suppressions: Arc::clone(&self.event_suppressions),
            received_event_log: Arc::default(),
            cov_table: Arc::clone(&self.cov_table),
            cov_in_flight: Arc::clone(&self.cov_in_flight),
            tasks: self.request_tasks.spawner(),
            mutation_decisions: Arc::clone(&self.mutation_decisions),
        }
    }

    /// The retained server's own dispatch context.
    pub(super) fn test_dispatch_context(&self) -> DispatchContext<T> {
        DispatchContext {
            services: self.test_services(),
            confirmed_request_tracker: Arc::clone(&self.confirmed_request_tracker),
            clock: self._clock.clone(),
            discovery_limiter: Arc::clone(&self.discovery_limiter),
            time_sync_limiter: Arc::clone(&self.time_sync_limiter),
            request_tasks: Arc::clone(&self.request_tasks),
        }
    }
}

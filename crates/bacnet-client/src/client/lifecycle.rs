use super::dispatch_context::{DispatchContext, InboundApdu};
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const DEVICE_PURGE_INTERVAL: Duration = Duration::from_secs(300);
#[cfg(not(test))]
const DEVICE_MAX_AGE: Duration = Duration::from_secs(600);
// Unit tests use Tokio's paused clock, while DeviceTable timestamps use
// std::time::Instant. Use an immediate threshold so the purge timer is
// observable without constructing an unrepresentable old Instant on Windows.
#[cfg(test)]
const DEVICE_MAX_AGE: Duration = Duration::ZERO;

impl<T: TransportPort> BACnetClient<T> {
    fn abort_owned_tasks(&self) {
        for task in [&self.dispatch_task, &self.network_number_task]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
    }
}

async fn join_owned_task(slot: &mut Option<JoinHandle<()>>) {
    // A canceled stop retains the join and its network Arc until it completes.
    if let Some(task) = slot.as_mut() {
        let _ = task.await;
    }
    *slot = None;
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Start the client: bind transport, start network layer, spawn dispatch.
    pub async fn start(config: ClientConfig, transport: T) -> Result<Self, Error> {
        Self::start_with_options(config, transport, ClientOptions::default()).await
    }

    /// Start the client with additional startup options.
    pub async fn start_with_options(
        mut config: ClientConfig,
        transport: T,
        options: ClientOptions,
    ) -> Result<Self, Error> {
        validate_max_apdu_length(config.max_apdu_length)?;
        validate_max_segments(config.max_segments)?;
        pacing::validate_interval_ms(config.min_request_interval_ms)?;
        config.max_apdu_length =
            cap_max_apdu_to_transport(config.max_apdu_length, transport.egress_apdu_limit())?;
        if !(1..=127).contains(&config.proposed_window_size) {
            return Err(Error::Encoding(format!(
                "invalid proposed-window-size {}; expected 1..=127",
                config.proposed_window_size
            )));
        }
        options.validate()?;

        let mut network = NetworkLayer::new(transport);
        let mut network_control_rx = network.enable_network_control_receiver()?;
        let mut apdu_rx = network.start().await?;
        config.max_apdu_length = cap_max_apdu_to_transport(
            config.max_apdu_length,
            network.transport().egress_apdu_limit(),
        )?;
        let local_mac = MacAddr::from_slice(network.local_mac());

        let network = Arc::new(network);
        let (number_tx, network_number_task) = if network
            .transport()
            .supports_local_nonrouter_number_controls()
        {
            let (tx, task) = network_number::spawn(&network);
            (Some(tx), Some(task))
        } else {
            (None, None)
        };

        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let tsm = Arc::new(Mutex::new(new_coordinated_tsm(&config, coordinator)));
        let tsm_dispatch = Arc::clone(&tsm);
        let device_table = Arc::new(Mutex::new(DeviceTable::new()));
        let device_table_dispatch = Arc::clone(&device_table);
        let network_dispatch = Arc::clone(&network);
        let (cov_tx, _) =
            broadcast::channel::<ReceivedCOVNotification>(options.cov_channel_capacity);
        let cov_tx_dispatch = cov_tx.clone();
        let (event_tx, _) =
            broadcast::channel::<ReceivedEventNotification>(options.event_channel_capacity);
        let event_tx_dispatch = event_tx.clone();
        let confirmed_cov_ack_policy = options.confirmed_cov_notification_ack_policy.clone();
        let (device_tx, _) = broadcast::channel::<DeviceEvent>(DEVICE_EVENT_CHANNEL_CAPACITY);
        let device_tx_dispatch = device_tx.clone();
        let (device_collision_tx, _) =
            broadcast::channel::<DeviceCollisionEvent>(DEVICE_EVENT_CHANNEL_CAPACITY);
        let device_collision_tx_dispatch = device_collision_tx.clone();
        let seg_ack_senders: Arc<Mutex<HashMap<SegAckKey, SegmentAckRoute>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seg_ack_senders_dispatch = Arc::clone(&seg_ack_senders);
        let (cleanup_tx, mut cleanup_rx) = mpsc::unbounded_channel::<TransactionCleanup>();
        #[cfg(test)]
        let segmented_cleanup = Arc::new(SegmentedCleanupHook::default());
        #[cfg(test)]
        let segmented_cleanup_dispatch = Arc::clone(&segmented_cleanup);
        let response_limits = ResponseLimits::from_config(&config);
        let routed_path_limits = Arc::new(RoutedPathLimits::new(routed_path_quarantine_horizon(
            &config,
        )));
        let routed_path_limits_dispatch = Arc::clone(&routed_path_limits);
        let group_source_request_drops = Arc::new(AtomicU64::new(0));
        let group_source_drops_dispatch = Arc::clone(&group_source_request_drops);

        let dispatch_task = tokio::spawn(async move {
            let mut seg_state: HashMap<SegKey, SegmentedReceiveState> = HashMap::new();
            let mut device_purge_interval = tokio::time::interval(DEVICE_PURGE_INTERVAL);
            let mut network_control_open = true;
            // Tokio intervals tick immediately; consume that tick so discovery purges
            // run on the configured cadence instead of racing the first APDU.
            device_purge_interval.tick().await;

            loop {
                tokio::select! {
                    control = network_control_rx.recv(), if network_control_open => {
                        match control {
                            Some(control) => {
                                if let Some((tx, parsed)) = number_tx
                                    .as_ref()
                                    .zip(bacnet_network::network_number::NumberControl::parse(&control))
                                {
                                    if tx.try_send(parsed).is_err() {
                                        debug!("client Number control queue full or closed; dropping control");
                                    }
                                } else {
                                    // Preserve the original envelope and ingress sequence for Reject correlation.
                                    routed_path_limits_dispatch
                                        .handle_network_control(&tsm_dispatch, control)
                                        .await;
                                }
                            }
                            None => network_control_open = false,
                        }
                    }
                    cleanup = cleanup_rx.recv() => {
                        let Some(cleanup) = cleanup else {
                            break;
                        };
                        if cleanup.cancel_tsm {
                            tsm_dispatch.lock().await.cancel_transaction_for_owner(
                                &cleanup.mac,
                                cleanup.invoke_id,
                                &cleanup.owner,
                            );
                        }
                        // Cleanup is provenance-agnostic: drop any reassembly
                        // snapshot for this (mac, invoke) with a matching
                        // owner, regardless of trust context (fail-closed).
                        let found = seg_state
                            .keys()
                            .find(|key| {
                                key.0 == cleanup.mac
                                    && key.1 == cleanup.invoke_id
                                    && seg_state
                                        .get(*key)
                                        .is_some_and(|state| {
                                            state.owner.same_as(&cleanup.owner)
                                        })
                            })
                            .cloned();
                        #[cfg(test)]
                        let removed = found.is_some();
                        if let Some(found) = found {
                            seg_state.remove(&found);
                        }
                        if let Some(expected_sender) = cleanup.seg_ack_sender {
                            let key = (cleanup.mac, cleanup.invoke_id);
                            let mut senders = seg_ack_senders_dispatch.lock().await;
                            if senders
                                .get(&key)
                                .is_some_and(|route| {
                                    route.owner.same_as(&cleanup.owner)
                                        && route.sender.same_channel(&expected_sender)
                                })
                            {
                                senders.remove(&key);
                            }
                        }
                        #[cfg(test)]
                        segmented_cleanup_dispatch.record_processed(removed);
                    }
                    _ = device_purge_interval.tick() => {
                        let lost_devices = device_table_dispatch
                            .lock()
                            .await
                            .purge_stale_collect(DEVICE_MAX_AGE);
                        for device in lost_devices {
                            let _ = device_tx_dispatch.send(DeviceEvent {
                                kind: DeviceEventKind::Lost,
                                device,
                            });
                        }
                    }
                    received = apdu_rx.recv() => {
                        let Some(received) = received else {
                            break;
                        };
                        match apdu::decode_apdu(received.apdu.clone()) {
                            // Its answer would go back to a group address,
                            // to every node there (#1504).
                            Ok(Apdu::ConfirmedRequest(_))
                                if network_dispatch
                                    .transport()
                                    .is_group_destination(&received.source_mac) =>
                            {
                                group_source_drops_dispatch.fetch_add(1, Ordering::Relaxed);
                                debug!("Ignoring a ConfirmedRequest from a group address");
                            }
                            Ok(decoded) => {
                                Self::dispatch_apdu(
                                    DispatchContext {
                                        tsm: &tsm_dispatch,
                                        device_table: &device_table_dispatch,
                                        network: &network_dispatch,
                                        cov_tx: &cov_tx_dispatch,
                                        event_tx: &event_tx_dispatch,
                                        confirmed_cov_ack_policy: &confirmed_cov_ack_policy,
                                        device_tx: &device_tx_dispatch,
                                        device_collision_tx: &device_collision_tx_dispatch,
                                        seg_ack_senders: &seg_ack_senders_dispatch,
                                    },
                                    &mut seg_state,
                                    InboundApdu {
                                        source_mac: &received.source_mac,
                                        source_network: &received.source_network,
                                        provenance: received.provenance,
                                        direct_response: received.direct_response,
                                        is_group: received.is_group,
                                        reply_tx: received.reply_tx,
                                    },
                                    decoded,
                                    response_limits,
                                )
                                .await;
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to decode received APDU");
                            }
                        }
                    }
                }
            }
        });

        // Shared: each paced request's guard keeps the pacer it reports to.
        let pacer = Arc::new(pacing::RequestPacer::new(Duration::from_millis(
            config.min_request_interval_ms,
        )));
        Ok(Self {
            config,
            network,
            tsm,
            device_table,
            cov_tx,
            event_tx,
            device_tx,
            device_collision_tx,
            dispatch_task: Some(dispatch_task),
            network_number_task,
            seg_ack_senders,
            cleanup_tx,
            #[cfg(test)]
            segmented_post_wait_cleanup: Arc::new(SegmentedPostWaitCleanupHook::default()),
            #[cfg(test)]
            segmented_cleanup,
            local_mac,
            routed_path_limits,
            group_source_request_drops,
            pacer,
        })
    }

    /// Confirmed requests ignored since start because the link-layer address
    /// they came from is a group destination of the transport
    /// ([`TransportPort::is_group_destination`]), such as a B/IP broadcast or
    /// multicast address (#1504). The answer, such as the acknowledgment of
    /// a confirmed COV notification, would go back to that address and reach
    /// every node in the group, so the request is neither handled nor
    /// answered. No built-in transport hands one up; a custom one can.
    pub fn group_source_request_drops(&self) -> u64 {
        self.group_source_request_drops.load(Ordering::Relaxed)
    }
    /// Get the client's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// Current BACnet max-APDU bucket advertised by this client.
    ///
    /// Returns `0` if the live transport budget is below the smallest BACnet
    /// max-APDU bucket and a confirmed request would currently fail before
    /// encoding.
    pub fn max_apdu_length(&self) -> u16 {
        max_apdu_bucket_at_or_below(
            self.config
                .max_apdu_length
                .min(self.network.transport().egress_apdu_limit()),
        )
        .unwrap_or(0)
    }

    /// Current APDU payload budget reported by the underlying transport.
    ///
    /// For BACnet/SC this reflects the negotiated hub envelope after connect
    /// and may be lower than the encoded BACnet max-APDU bucket advertised by
    /// the client.
    pub fn transport_max_apdu_length(&self) -> u16 {
        self.network.transport().egress_apdu_limit()
    }

    /// Stop the client, aborting and joining dispatch and local Number work.
    /// Canceling this waiter retains task joins for a subsequent stop.
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.abort_owned_tasks();
        join_owned_task(&mut self.network_number_task).await;
        join_owned_task(&mut self.dispatch_task).await;
        self.tsm.lock().await.cancel_all_transactions();
        let network = Arc::get_mut(&mut self.network).ok_or_else(|| {
            Error::Encoding("cannot stop BACnetClient while network references remain".into())
        })?;
        network.stop().await?;
        Ok(())
    }
}

impl<T: TransportPort> Drop for BACnetClient<T> {
    fn drop(&mut self) {
        self.abort_owned_tasks();
        if let Ok(mut tsm) = self.tsm.try_lock() {
            tsm.cancel_all_transactions();
        }
    }
}

use super::*;

const DEVICE_PURGE_INTERVAL: Duration = Duration::from_secs(300);
#[cfg(not(test))]
const DEVICE_MAX_AGE: Duration = Duration::from_secs(600);
// Unit tests use Tokio's paused clock, while DeviceTable timestamps use
// std::time::Instant. Use an immediate threshold so the purge timer is
// observable without constructing an unrepresentable old Instant on Windows.
#[cfg(test)]
const DEVICE_MAX_AGE: Duration = Duration::ZERO;

impl<T: TransportPort> BACnetClient<T> {
    fn abort_dispatch_task(&mut self) -> Option<JoinHandle<()>> {
        let task = self.dispatch_task.take()?;
        task.abort();
        Some(task)
    }
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
        config.max_apdu_length =
            cap_max_apdu_to_transport(config.max_apdu_length, transport.max_apdu_length())?;
        if !(1..=127).contains(&config.proposed_window_size) {
            return Err(Error::Encoding(format!(
                "invalid proposed-window-size {}; expected 1..=127",
                config.proposed_window_size
            )));
        }
        options.validate()?;

        let mut network = NetworkLayer::new(transport);
        let mut apdu_rx = network.start().await?;
        config.max_apdu_length = cap_max_apdu_to_transport(
            config.max_apdu_length,
            network.transport().max_apdu_length(),
        )?;
        let local_mac = MacAddr::from_slice(network.local_mac());

        let network = Arc::new(network);

        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let tsm = Arc::new(Mutex::new(new_coordinated_tsm(&config, coordinator)));
        let tsm_dispatch = Arc::clone(&tsm);
        let device_table = Arc::new(Mutex::new(DeviceTable::new()));
        let device_table_dispatch = Arc::clone(&device_table);
        let network_dispatch = Arc::clone(&network);
        let (cov_tx, _) =
            broadcast::channel::<ReceivedCOVNotification>(options.cov_channel_capacity);
        let cov_tx_dispatch = cov_tx.clone();
        let confirmed_cov_ack_policy = options.confirmed_cov_notification_ack_policy.clone();
        let (device_tx, _) = broadcast::channel::<DeviceEvent>(DEVICE_EVENT_CHANNEL_CAPACITY);
        let device_tx_dispatch = device_tx.clone();
        let seg_ack_senders: Arc<Mutex<HashMap<SegKey, SegmentAckRoute>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seg_ack_senders_dispatch = Arc::clone(&seg_ack_senders);
        let (cleanup_tx, mut cleanup_rx) = mpsc::unbounded_channel::<TransactionCleanup>();
        #[cfg(test)]
        let segmented_cleanup = Arc::new(SegmentedCleanupHook::default());
        #[cfg(test)]
        let segmented_cleanup_dispatch = Arc::clone(&segmented_cleanup);
        let response_limits = ResponseLimits::from_config(&config);

        let dispatch_task = tokio::spawn(async move {
            let mut seg_state: HashMap<SegKey, SegmentedReceiveState> = HashMap::new();
            let mut device_purge_interval = tokio::time::interval(DEVICE_PURGE_INTERVAL);
            // Tokio intervals tick immediately; consume that tick so discovery purges
            // run on the configured cadence instead of racing the first APDU.
            device_purge_interval.tick().await;

            loop {
                tokio::select! {
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
                        let key = (cleanup.mac, cleanup.invoke_id);
                        let removed = seg_state
                            .get(&key)
                            .is_some_and(|state| state.owner.same_as(&cleanup.owner));
                        if removed {
                            seg_state.remove(&key);
                        }
                        if let Some(expected_sender) = cleanup.seg_ack_sender {
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
                            Ok(decoded) => {
                                Self::dispatch_apdu(
                                    &tsm_dispatch,
                                    &device_table_dispatch,
                                    &network_dispatch,
                                    &cov_tx_dispatch,
                                    &confirmed_cov_ack_policy,
                                    &device_tx_dispatch,
                                    &mut seg_state,
                                    &seg_ack_senders_dispatch,
                                    &received.source_mac,
                                    &received.source_network,
                                    received.is_group,
                                    received.reply_tx,
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

        Ok(Self {
            config,
            network,
            tsm,
            device_table,
            cov_tx,
            device_tx,
            dispatch_task: Some(dispatch_task),
            seg_ack_senders,
            cleanup_tx,
            #[cfg(test)]
            segmented_post_wait_cleanup: Arc::new(SegmentedPostWaitCleanupHook::default()),
            #[cfg(test)]
            segmented_cleanup,
            local_mac,
        })
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
                .min(self.network.transport().max_apdu_length()),
        )
        .unwrap_or(0)
    }

    /// Current APDU payload budget reported by the underlying transport.
    ///
    /// For BACnet/SC this reflects the negotiated hub envelope after connect
    /// and may be lower than the encoded BACnet max-APDU bucket advertised by
    /// the client.
    pub fn transport_max_apdu_length(&self) -> u16 {
        self.network.transport().max_apdu_length()
    }

    /// Stop the client, aborting the dispatch task.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.abort_dispatch_task() {
            let _ = task.await;
        }
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
        let _ = self.abort_dispatch_task();
        if let Ok(mut tsm) = self.tsm.try_lock() {
            tsm.cancel_all_transactions();
        }
    }
}

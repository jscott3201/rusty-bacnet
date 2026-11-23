use super::dispatch_context::{DispatchContext, InboundApdu};
use super::dispatch_turn::{Turn, TurnOrder};
use super::seat::{Inbound, Own};
use super::*;
use bacnet_network::layer::ReceivedNetworkControl;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc::error::TryRecvError;

pub(super) const DEVICE_PURGE_INTERVAL: Duration = Duration::from_secs(300);
/// How long a device stays in the table without an I-Am or an answer to a
/// confirmed request. Rows age by Tokio's clock, so a test's paused time
/// ages them too.
pub(super) const DEVICE_MAX_AGE: Duration = Duration::from_secs(600);

impl ClientConfig {
    pub(super) fn validate_request_settings(&self) -> Result<(), Error> {
        validate_max_apdu_length(self.max_apdu_length)?;
        validate_max_segments(self.max_segments)?;
        pacing::validate_interval_ms(self.min_request_interval_ms)
    }

    pub(super) fn validate_window(&self) -> Result<(), Error> {
        if !(1..=127).contains(&self.proposed_window_size) {
            return Err(Error::Encoding(format!(
                "invalid proposed-window-size {}; expected 1..=127",
                self.proposed_window_size
            )));
        }
        Ok(())
    }
}

impl<T: TransportPort> BACnetClient<T> {
    fn abort_owned_tasks(&self) {
        for task in [
            &self.dispatch_task,
            &self.network_number_task,
            &self.device_purge_task,
        ]
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

/// The dispatch loop's network controls, read without waiting: see the
/// [`dispatch_turn`] docs for why they need no waker.
pub(super) struct Controls {
    rx: mpsc::Receiver<ReceivedNetworkControl>,
    pub(super) open: bool,
    /// The layer's ingress sequence when the queue was last found empty.
    drained_at: u64,
}

impl Controls {
    pub(super) fn new(rx: mpsc::Receiver<ReceivedNetworkControl>, admitted: u64) -> Self {
        Self {
            rx,
            open: true,
            drained_at: admitted,
        }
    }

    /// Take a queued control, given the layer's ingress sequence now. It
    /// advances for each control the layer admits, which happens only in
    /// this task's APDU receive, so while it stands where the queue was last
    /// found empty there is nothing to take and the queue isn't read. At
    /// `u64::MAX` it stops advancing, and the queue is read every time.
    pub(super) fn take(&mut self, admitted: u64) -> Option<ReceivedNetworkControl> {
        if !self.open || (admitted == self.drained_at && admitted != u64::MAX) {
            return None;
        }
        match self.rx.try_recv() {
            Ok(control) => Some(control),
            Err(TryRecvError::Empty) => {
                self.drained_at = admitted;
                None
            }
            Err(TryRecvError::Disconnected) => {
                self.open = false;
                None
            }
        }
    }
}

/// Every [`DEVICE_PURGE_INTERVAL`], drop the devices not heard from in
/// [`DEVICE_MAX_AGE`] and announce each as lost, then each contest that
/// lost a claimant. A task of its own, so the dispatch loop doesn't poll
/// this timer on every turn (#1978 item 8); the loop ends it when it ends
/// itself, and `stop()` and drop abort both.
fn spawn_device_purge(
    device_table: Arc<network_number::Discovery>,
    device_tx: broadcast::Sender<DeviceEvent>,
    device_collision_tx: broadcast::Sender<DeviceCollisionEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(DEVICE_PURGE_INTERVAL);
        // Tokio intervals tick immediately; consume that tick so purges run
        // on the configured cadence.
        interval.tick().await;
        loop {
            interval.tick().await;
            // The events go out under the table's lock, as a broadcast send
            // never waits. The dispatch task adds a device and announces it
            // in that order, so a device it adds back after this purge is
            // announced after its Lost event, never before.
            let mut table = device_table.lock();
            let purged = table.purge_stale_collect(DEVICE_MAX_AGE);
            for device in purged.lost {
                let _ = device_tx.send(DeviceEvent {
                    kind: DeviceEventKind::Lost,
                    device,
                });
            }
            for instance in purged.contested {
                let _ = device_collision_tx.send(DeviceCollisionEvent::of(&table, instance));
            }
        }
    })
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Start the client: bind transport, start network layer, spawn dispatch.
    pub async fn start(config: ClientConfig, transport: T) -> Result<Self, Error> {
        Self::start_with_options(config, transport, ClientOptions::default()).await
    }

    /// Start the client with additional startup options.
    pub async fn start_with_options(
        config: ClientConfig,
        transport: T,
        options: ClientOptions,
    ) -> Result<Self, Error> {
        config.validate_request_settings()?;
        // `config.max_apdu_length` stays the configured maximum: each request
        // takes the minimum with the transport's live limit, so it follows a
        // limit that rises after start as well as one that falls (#2086).
        // These checks only refuse a transport that leaves no APDU bucket.
        let _bucket =
            cap_max_apdu_to_transport(config.max_apdu_length, transport.egress_apdu_limit())?;
        config.validate_window()?;
        options.validate()?;

        let mut network = NetworkLayer::new(transport);
        let network_control_rx = network.enable_network_control_receiver()?;
        let apdu_rx = network.start().await?;
        let _bucket = cap_max_apdu_to_transport(
            config.max_apdu_length,
            network.transport().egress_apdu_limit(),
        )?;
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let inbound = Own::new(Arc::new(network), apdu_rx, network_control_rx);
        Ok(Self::assemble(config, options, inbound, coordinator))
    }

    /// Build the client on a started network layer: its state, its dispatch
    /// task and its Number work. `inbound` is where its APDUs and controls
    /// come from: its own layer's receivers, or a seat's queues (#2617).
    /// Nothing here waits, and the caller has checked `config` and `options`.
    pub(super) fn assemble<S: Inbound<T>>(
        config: ClientConfig,
        options: ClientOptions,
        mut inbound: S,
        coordinator: Arc<OutboundTransactionCoordinator>,
    ) -> Self {
        let network = Arc::clone(inbound.network());
        let local_mac = MacAddr::from_slice(network.local_mac());
        // On a link that takes the local Number controls, the client asks
        // this network's number on first need, and again when the link comes
        // back while it is still unknown (#2572); nothing goes out at start.
        let numbers = network
            .transport()
            .supports_local_nonrouter_number_controls();
        let link_returns = numbers
            .then(|| network.transport().link_returns())
            .flatten()
            .map(|mut returns| {
                returns.mark_unchanged();
                returns
            });

        let tsm = Arc::new(SyncMutex::new(new_coordinated_tsm(&config, coordinator)));
        let tsm_dispatch = Arc::clone(&tsm);
        let device_table = Arc::new(network_number::Discovery::new(numbers, S::SEATED));
        let device_table_dispatch = Arc::clone(&device_table);
        inbound.bind(&tsm, &device_table);
        let seat = inbound.seat();
        let (cov_tx, _) = broadcast::channel::<COVEvent>(options.cov_channel_capacity);
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
        let (number_tx, network_number_task) = if numbers {
            let owner = network_number::Owner {
                discovery: Arc::clone(&device_table),
                device_tx: device_tx.clone(),
                collision_tx: device_collision_tx.clone(),
                link_returns,
            };
            let (tx, task) = inbound.spawn_number_owner(owner);
            (Some(tx), Some(task))
        } else {
            (None, None)
        };
        let (i_have_tx, _) = broadcast::channel::<ReceivedIHave>(I_HAVE_CHANNEL_CAPACITY);
        let i_have_tx_dispatch = i_have_tx.clone();
        let (network_report_tx, _) = broadcast::channel(NETWORK_REPORT_CHANNEL_CAPACITY);
        let network_report_tx_dispatch = network_report_tx.clone();
        let (routing_end, routing_ended) = tokio::sync::watch::channel(());
        let routing_query = Arc::new(routing_table::RoutingQueries::new(routing_ended));
        let routing_query_dispatch = Arc::clone(&routing_query);
        let seg_ack_senders: Arc<SyncMutex<HashMap<SegAckKey, SegmentAckRoute>>> =
            Arc::new(SyncMutex::new(HashMap::new()));
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
        // One of each per client: a peer's flood of undecodable APDUs, or of
        // responses refused, warns once per interval, the rest at debug
        // (#1950).
        let decode_warning = Arc::new(WarnThrottle::default());
        let response_warning = Arc::new(WarnThrottle::default());
        #[cfg(test)]
        let decode_warning_hook = Arc::clone(&decode_warning);
        #[cfg(test)]
        let response_warning_hook = Arc::clone(&response_warning);

        let device_purge_task = spawn_device_purge(
            Arc::clone(&device_table),
            device_tx.clone(),
            device_collision_tx.clone(),
        );
        let device_purge = device_purge_task.abort_handle();
        let dispatch_task = tokio::spawn(async move {
            // Dropped when this task ends, however it ends (#2612).
            let _routing_end = routing_end;
            let mut seg_state: HashMap<SegKey, SegmentedReceiveState> = HashMap::new();
            let mut order = TurnOrder::default();

            loop {
                let turn = std::future::poll_fn(|cx| {
                    inbound.poll_turn(&mut order, cx, |cx| cleanup_rx.poll_recv(cx))
                })
                .await;
                match turn {
                    Turn::Control(control) => {
                        // Read once; subscribers first, and a broadcast
                        // send never waits.
                        let number = bacnet_network::network_number::NumberControl::parse(&control);
                        network_reports::publish(
                            &network_report_tx_dispatch,
                            &control,
                            number,
                            &routing_query_dispatch.outstanding,
                        );
                        if let Some((tx, parsed)) = number_tx.as_ref().zip(number) {
                            if tx.try_send(parsed).is_err() {
                                debug!(
                                    "client Number control queue full or closed; dropping control"
                                );
                            }
                        } else {
                            // Preserve the original envelope and ingress sequence for Reject correlation.
                            routed_path_limits_dispatch
                                .handle_network_control(&tsm_dispatch, control)
                                .await;
                        }
                    }
                    Turn::Cleanup(cleanup) => {
                        let Some(cleanup) = cleanup else {
                            break;
                        };
                        if cleanup.cancel_tsm {
                            tsm_dispatch.lock().cancel_transaction_for_owner(
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
                                        .is_some_and(|state| state.owner.same_as(&cleanup.owner))
                            })
                            .cloned();
                        #[cfg(test)]
                        let removed = found.is_some();
                        if let Some(found) = found {
                            seg_state.remove(&found);
                        }
                        if let Some(expected_sender) = cleanup.seg_ack_sender {
                            let key = (cleanup.mac, cleanup.invoke_id);
                            let mut senders = seg_ack_senders_dispatch.lock();
                            if senders.get(&key).is_some_and(|route| {
                                route.owner.same_as(&cleanup.owner)
                                    && route.sender.same_channel(&expected_sender)
                            }) {
                                senders.remove(&key);
                            }
                        }
                        #[cfg(test)]
                        segmented_cleanup_dispatch.record_processed(removed);
                    }
                    Turn::Apdu(received) => {
                        let Some(received) = received else {
                            break;
                        };
                        let network_dispatch = inbound.network();
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
                            // Beside a server role, which answers for the
                            // device, only a confirmed COV notification, of
                            // either form, is the client's to answer (#2617).
                            Ok(Apdu::ConfirmedRequest(request))
                                if !inbound.answers_for_device()
                                    && !dispatch::seated_client_answers(&request) =>
                            {
                                dispatch::heard_for_the_server(
                                    &event_tx_dispatch,
                                    &received,
                                    &request,
                                    &decode_warning,
                                );
                            }
                            Ok(decoded) => {
                                Self::dispatch_apdu(
                                    DispatchContext {
                                        tsm: &tsm_dispatch,
                                        device_table: &device_table_dispatch,
                                        network: network_dispatch,
                                        cov_tx: &cov_tx_dispatch,
                                        event_tx: &event_tx_dispatch,
                                        confirmed_cov_ack_policy: &confirmed_cov_ack_policy,
                                        device_tx: &device_tx_dispatch,
                                        device_collision_tx: &device_collision_tx_dispatch,
                                        i_have_tx: &i_have_tx_dispatch,
                                        seg_ack_senders: &seg_ack_senders_dispatch,
                                        decode_warning: &decode_warning,
                                        response_warning: &response_warning,
                                    },
                                    &mut seg_state,
                                    InboundApdu {
                                        source_mac: &received.source_mac,
                                        source_network: &received.source_network,
                                        provenance: received.provenance.clone(),
                                        response_route: received.response_route(),
                                        is_group: received.is_group,
                                        reply_tx: received.reply_tx,
                                    },
                                    decoded,
                                    response_limits,
                                )
                                .await;
                            }
                            Err(e) => {
                                warn_throttled!(
                                    decode_warning,
                                    error = %e,
                                    "Failed to decode received APDU"
                                );
                            }
                        }
                    }
                }
            }
            // The receive queue closed: no device can be heard from again,
            // so the purge ends with the loop, as it did inside it.
            device_purge.abort();
        });

        // Shared: each paced request's guard keeps the pacer it reports to.
        let pacer = Arc::new(pacing::RequestPacer::new(Duration::from_millis(
            config.min_request_interval_ms,
        )));
        // On a seat, the node aborts these tasks when it stops, and takes
        // another client as soon as this one, which alone keeps the pacer,
        // is dropped (#2617).
        if let Some(seat) = seat {
            seat.abort_on_stop(dispatch_task.abort_handle());
            seat.abort_on_stop(device_purge_task.abort_handle());
            if let Some(task) = &network_number_task {
                seat.abort_on_stop(task.abort_handle());
            }
            let alive: Arc<dyn std::any::Any + Send + Sync> = pacer.clone();
            seat.alive_while(Arc::downgrade(&alive));
        }
        Self {
            config,
            network,
            tsm,
            device_table,
            cov_tx,
            event_tx,
            device_tx,
            device_collision_tx,
            i_have_tx,
            network_report_tx,
            routing_query,
            dispatch_task: Some(dispatch_task),
            network_number_task,
            device_purge_task: Some(device_purge_task),
            seg_ack_senders,
            cleanup_tx,
            #[cfg(test)]
            segmented_post_wait_cleanup: Arc::new(SegmentedPostWaitCleanupHook::default()),
            #[cfg(test)]
            segmented_cleanup,
            #[cfg(test)]
            decode_warning: decode_warning_hook,
            #[cfg(test)]
            response_warning: response_warning_hook,
            local_mac,
            routed_path_limits,
            group_source_request_drops,
            pacer,
        }
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

    /// Reject-Message-To-Network messages with reason 4 (message too long)
    /// dropped since start because they could answer a frame of an earlier
    /// request on the same router and network rather than the request
    /// active there (#2651). A request the device answered after more than
    /// one frame, a retry or segments, frees its path at once; for the
    /// window it would have held the path before (the APDU timeout times the
    /// attempts), a reason 4 fails a later request on that path only when
    /// that request sent a frame at least as large as the answered one's
    /// largest. A smaller request runs on to its own answer, retry or
    /// timeout, and the reject is counted here. This clears by itself;
    /// see [`below_minimum_reject_drops`](Self::below_minimum_reject_drops)
    /// for the drop that may need action.
    pub fn too_long_reject_drops(&self) -> u64 {
        self.routed_path_limits.dropped_rejects()
    }

    /// Reason-4 rejects dropped since start because the request they landed
    /// on had sent nothing above 228 octets of NPDU, the smallest size every
    /// router must forward (Table 6-1, #2665). A conforming router never
    /// refuses a frame that small as too long, so the reject belongs to
    /// other traffic, and learning from it would pin the path below any
    /// link's minimum. A rise can also mean a router that really limits the
    /// path below 228 octets: its requests then end by timeout, and the limit
    /// is set by hand with
    /// [`configure_routed_path_max_npdu`](Self::configure_routed_path_max_npdu).
    /// The first drop on each path logs a warning.
    pub fn below_minimum_reject_drops(&self) -> u64 {
        self.routed_path_limits.below_minimum_drops()
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

    /// Stop the client, aborting and joining dispatch, the device purge and
    /// local Number work, then stopping its network layer and transport,
    /// whoever else still holds the layer: their sends fail from then on
    /// (#2617). A client on a seat ([`Self::start_on_seat`]) stops only
    /// itself: the node that owns the link stops the transport, and this
    /// client's calls fail once it has.
    /// Canceling this waiter retains task joins for a subsequent stop.
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.abort_owned_tasks();
        join_owned_task(&mut self.network_number_task).await;
        join_owned_task(&mut self.dispatch_task).await;
        join_owned_task(&mut self.device_purge_task).await;
        self.tsm.lock().cancel_all_transactions();
        // On a seat the node owns the network layer and stops it (#2617).
        if self.device_table.asking.seated {
            return Ok(());
        }
        self.network.stop().await
    }
}

impl<T: TransportPort> Drop for BACnetClient<T> {
    fn drop(&mut self) {
        // A runner shares its client's transactions and owns no task: its
        // own request ends with the request's future (#2617).
        if self
            .pacer
            .audit
            .get()
            .is_some_and(super::source_audit::Role::is_runner)
        {
            return;
        }
        self.abort_owned_tasks();
        if let Some(mut tsm) = self.tsm.try_lock() {
            tsm.cancel_all_transactions();
        }
    }
}

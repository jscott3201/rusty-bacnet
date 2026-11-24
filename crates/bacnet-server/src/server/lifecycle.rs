use super::*;
#[path = "background_lifecycle.rs"]
mod background;
#[path = "intrinsic_reporting_lifecycle.rs"]
mod intrinsic;
#[path = "fanout_tail.rs"]
pub(super) mod tail;

#[path = "lifecycle_period.rs"]
mod period;
use super::binary_lighting_lifecycle::spawn_binary_lighting_operation_task as spawn_operation;
use super::heap_futures::boxed;
use super::server_seat::{Apdus as _, Link, Own, Started};
use super::{audit_recipient::spawn_owned, audit_recipient_routes::AuditRoutes};
use bacnet_endpoint_core::segmented_request::SegmentedRequests;

#[path = "start_settings.rs"]
pub(super) mod settings;
#[path = "start_outputs.rs"]
mod start_outputs;
pub(super) use period::{MonotonicClocks, event_enrollment_period};

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Start a server: every public start and build path ends here. The
    /// startup future is on the heap, which keeps theirs small (#953).
    pub(super) fn start_with_clock_mode_and_bindings(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> std::pin::Pin<Box<impl std::future::Future<Output = Result<Self, StartError>>>> {
        Self::start_on_link(
            config,
            db,
            Own(transport),
            clock_config,
            configured_device_bindings,
        )
    }

    /// [`Self::start_with_clock_mode_and_bindings`] on any [`Link`]: a
    /// transport of the server's own, or a node's seat (#2617).
    pub(super) fn start_on_link<L: Link<T>>(
        config: ServerConfig,
        db: ObjectDatabase,
        link: L,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> std::pin::Pin<Box<impl std::future::Future<Output = Result<Self, StartError>>>> {
        boxed(|| Self::start_on_heap(config, db, link, clock_config, configured_device_bindings))
    }

    /// Every refusal hands the database back ([`StartError`], #2752).
    async fn start_on_heap<L: Link<T>>(
        mut config: ServerConfig,
        mut db: ObjectDatabase,
        link: L,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> Result<Self, StartError> {
        let transport = link.transport();
        // Every setting, and every configured route against the concrete
        // transport, before mutating the database or starting network work.
        // No binding takes a group address (#1493). A node checks the same
        // before its own link starts (#2617).
        let checked = settings::check(&mut config, &db, configured_device_bindings, transport);
        let settings::StartSettings {
            device_bindings,
            segment_timeout,
            request_tasks,
        } = match checked {
            Ok(settled) => settled,
            Err(error) => return Err(StartError::new(error, Some(db))),
        };
        crate::local_device::bind_database(&mut db, &config.device);
        let audit_routes = match AuditRoutes::prepare(&mut db, &config, &device_bindings, transport)
        {
            Ok(routes) => routes,
            Err(error) => return Err(StartError::new(error, Some(db))),
        };
        super::audit_forwarder::initialize(&db, &config, &device_bindings, transport);
        // Settled now: every owner from here on shares it (#1521).
        let config = Arc::new(config);

        if config.device.vendor_id() == 0 {
            warn!("vendor_id is 0 (ASHRAE reserved); set a valid vendor ID for production use");
        }

        let (clock, monotonic) =
            period::install_database_clocks(&mut db, clock_config, config.clock_zone_writable);
        // The device is starting: colour outputs go to their defaults
        // (#1526). The database isn't shared yet, so no lock is taken. This
        // runs before `network_port::start`, which can still fail; that's
        // harmless, since the drive is what any start does, and a later
        // start over the same objects drives them the same way again. What
        // a start began elsewhere, such as a restored Load Control shed, is
        // released if the start fails or its future is dropped (#2479).
        let mut outputs = start_outputs::StartedOutputs::start(db);
        let membership = crate::membership::install_waker(outputs.db());

        let Started {
            network,
            apdus: mut apdu_rx,
            audit_routes,
            controls: network_controls,
            notification_transactions,
            seated,
            execution,
            dcc: node_dcc,
            number: seat_number,
        } = match boxed(|| link.start(outputs.db(), &config, audit_routes)).await {
            Ok(started) => started,
            Err(error) => return Err(StartError::new(error, Some(outputs.refuse()))),
        };
        notification_transactions.set_retries(config.notification_retries);
        let local_mac = MacAddr::from_slice(network.local_mac());
        // Before the database is shared, so a refusal hands it back once the
        // link is let go of (#2752).
        let target_audit = super::audit_recipient::TargetAudit::install(
            outputs.db(),
            &config,
            audit_routes,
            &network,
            &notification_transactions,
        );
        let target_audit = match target_audit {
            Ok(runtime) => runtime,
            Err(error) => {
                let error = L::abandon(network).await.err().unwrap_or(error);
                return Err(StartError::new(error, Some(outputs.refuse())));
            }
        };

        // The limiter keeps this startup identity; changing Device membership
        // later does not rebind it.
        let device_instance = Some(config.device.instance());
        let (discovery_limiter, time_sync_limiter) = request_limiters(&config, device_instance);
        let db = outputs.share();
        let cov_counters = Arc::new(crate::cov::AtomicCovCounters::default());
        let cov_table = Arc::new(RwLock::new(
            CovSubscriptionTable::with_policy(config.cov_policy.clone(), Arc::clone(&cov_counters))
                .with_max_apdu_length(u32::from(config.device.max_apdu_length()) as usize)
                .with_flight_timing(config.cov_flight_timing()),
        ));
        let seg_ack_senders = Arc::new(segmented_send::SegmentedSendRegistry::default());
        let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));

        let cov_in_flight = Arc::new(Semaphore::new(255));
        let learned_routers = Arc::new(Mutex::new(LearnedRouterCache::new()));
        let confirmed_request_tracker = Arc::new(ConfirmedRequestTracker::default());
        let device_bindings = Arc::new(RwLock::new(device_bindings));
        // ENABLE at every start, the node's too on a seat (#2617).
        let comm_state = Arc::new(CommState::new(node_dcc));
        let dcc_timer: Arc<Mutex<crate::server::dcc_timer::TimerSlot>> =
            Arc::new(Mutex::new(Default::default()));
        let dcc_outcomes = Arc::new(dcc_outcomes::DccOutcomes::default());
        let event_suppressions = Arc::new(super::event_suppression::EventSuppressions::default());
        let mutation_decisions = Arc::new(crate::mutation::MutationDecisions::default());

        let network_number_task = match (network_controls, seat_number) {
            (Some(controls), _) => Some(super::network_port::spawn_number_worker(
                &network,
                &db,
                config.registered_network_port,
                controls,
                target_audit.as_ref().map(Arc::downgrade),
            )),
            // On a seat the node's Number owner publishes; target Audit
            // follows its notice (#2617).
            (None, Some(number)) => target_audit
                .as_ref()
                .map(|audit| super::server_seat::follow_number(number, Arc::downgrade(audit))),
            (None, None) => None,
        };
        let network_dispatch = Arc::clone(&network);
        let config_dispatch = Arc::clone(&config);
        let notification_transactions_dispatch = Arc::clone(&notification_transactions);

        let audit_owner = target_audit
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.owner));
        if let Some(owner) = &audit_owner {
            request_tasks.set_audit_owner(owner);
        }
        let requests = Arc::clone(&request_tasks);
        let dispatch_context = DispatchContext {
            services: Arc::new(RequestServices {
                db: Arc::clone(&db),
                network: Arc::clone(&network_dispatch),
                cov_table: Arc::clone(&cov_table),
                seg_ack_senders: Arc::clone(&seg_ack_senders),
                seg_send_permits: Arc::clone(&seg_send_permits),
                cov_in_flight: Arc::clone(&cov_in_flight),
                learned_routers: Arc::clone(&learned_routers),
                notification_transactions: Arc::clone(&notification_transactions_dispatch),
                device_bindings: Arc::clone(&device_bindings),
                comm_state: Arc::clone(&comm_state),
                dcc_timer: Arc::clone(&dcc_timer),
                dcc_outcomes: Arc::clone(&dcc_outcomes),
                event_suppressions: Arc::clone(&event_suppressions),
                confirmed_event_repeats: Arc::default(),
                received_event_log: Arc::default(),
                mutation_decisions: Arc::clone(&mutation_decisions),
                warnings: Arc::default(),
                config: Arc::clone(&config_dispatch),
                execution,
            }),
            confirmed_request_tracker: Arc::clone(&confirmed_request_tracker),
            clock: clock.clone(),
            discovery_limiter: discovery_limiter.clone(),
            time_sync_limiter: time_sync_limiter.clone(),
            request_tasks: Arc::clone(&request_tasks),
        };
        // A Schedule tick that writes a Command's Present_Value starts its list.
        let command_runs = super::command_runs::CommandRunner::new(
            &dispatch_context.services,
            &request_tasks.spawner(),
        );
        let timer_runner = command_runs.clone(); // A Timer's expiry starts its writes (#2206).
        let dispatch_task = spawn_owned(audit_owner.clone(), move || async move {
            let mut seg_receivers = SegmentedRequests::new(segment_timeout);
            let mut notifications_open = true;
            let mut ingress_open = true;
            // A peer's flood warns once per interval, the rest at debug (#1950).
            let warnings = Arc::clone(&dispatch_context.services.warnings);
            let (decode_warning, segment_warning) = (&warnings.decode, &warnings.segment);

            // While ingress is open, finished handlers are joined at the top
            // of each turn, not awaited below (IdleReap, #1713).
            let mut idle_reap = super::request_tasks::IdleReap::new();
            loop {
                requests.reap_finished();
                // Drain all selected ownership before the first asynchronous Abort.
                // With no reassembly open nothing can expire, so the clock
                // isn't read (#1978 item 6).
                if !seg_receivers.is_empty() {
                    Self::reap_expired_requests(
                        &network_dispatch,
                        &mut seg_receivers,
                        runtime_clock::now(),
                    )
                    .await;
                }
                let received = tokio::select! {
                    () = super::segmented_receive::wait_receive_deadline(&seg_receivers), if !seg_receivers.is_empty() => continue,
                    result = notification_transactions_dispatch.join_next(), if notifications_open => {
                        notifications_open = result.is_some();
                        NotificationTransactions::observe(result);
                        continue;
                    }
                    result = requests.join_next(), if !ingress_open && !requests.is_empty() => {
                        super::request_tasks::RequestTasks::observe(result);
                        continue;
                    }
                    () = idle_reap.tick(), if ingress_open && !requests.is_empty() => continue,
                    received = apdu_rx.recv(), if ingress_open => match received {
                        Some(received) => received,
                        None => {
                            // Local/periodic notification producers outlive ingress.
                            // Keep both join consumers active, but never poll EOF again.
                            ingress_open = false;
                            seg_receivers.clear();
                            continue;
                        }
                    },
                    else => break,
                };
                // Input and the timer can become ready together. Apply expiry
                // before interpreting this segment in either scheduling order.
                if !seg_receivers.is_empty() {
                    Self::reap_expired_requests(
                        &network_dispatch,
                        &mut seg_receivers,
                        runtime_clock::now(),
                    )
                    .await;
                }

                match apdu::decode_apdu(received.apdu.clone()) {
                    Ok(decoded) => {
                        if notification_transactions_dispatch
                            .application_sealed
                            .load(Ordering::Acquire)
                        {
                            seg_receivers.clear();
                            if matches!(
                                decoded,
                                Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)
                            ) {
                                continue;
                            }
                        }
                        // A confirmed request may only be addressed to one
                        // device (Clause 6.3). One that arrives by local,
                        // remote or global broadcast, or by multicast, leaves
                        // the server TSM idle (Clause 5.4.5.1): it is neither
                        // executed nor answered, whatever the service, and
                        // no reassembly starts for it.
                        if received.is_group && matches!(decoded, Apdu::ConfirmedRequest(_)) {
                            debug!("Ignoring a ConfirmedRequest addressed to a group");
                            continue;
                        }
                        // Nor one sent from a group address, where its
                        // answer would go (#1504).
                        if matches!(decoded, Apdu::ConfirmedRequest(_))
                            && dispatch_context
                                .confirmed_request_tracker
                                .refuse_group_source(&received.source_mac, |mac| {
                                    network_dispatch.transport().is_group_destination(mac)
                                })
                        {
                            continue;
                        }
                        let source_mac = MacAddr::from_slice(&received.source_mac);

                        // Clause 5.4.5.2 AbortPDU_Received: a peer's Abort
                        // ('server' = FALSE) ends any reassembly session for
                        // its transaction. A side effect, not a short
                        // circuit — the PDU still reaches `dispatch`, whose
                        // Abort arm cancels in-flight segmented response
                        // senders and records server-TSM results (#377).
                        super::segmented_receive::remove_peer_aborted_request(
                            &mut seg_receivers,
                            &received,
                            &decoded,
                        );

                        if let Apdu::ConfirmedRequest(ref req) = decoded
                            && req.segmented
                        {
                            // The shared receiver takes it (#2742); the plain
                            // path below is untouched.
                            Self::receive_request_segment(
                                &network_dispatch,
                                &dispatch_context,
                                &config_dispatch,
                                &mut seg_receivers,
                                segment_warning,
                                &source_mac,
                                req,
                                received,
                            )
                            .await;
                        } else {
                            Self::dispatch(&dispatch_context, &source_mac, decoded, received).await;
                        }
                    }
                    Err(e) => {
                        warn_throttled!(
                            decode_warning,
                            error = %e,
                            "Server failed to decode received APDU"
                        );
                    }
                }
            }
        });

        let cov_table_for_purge = Arc::clone(&cov_table);
        let cov_purge_task = spawn_owned(audit_owner.clone(), move || async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let mut table = cov_table_for_purge.write().await;
                let purged = table.purge_expired();
                if purged > 0 {
                    debug!(purged, "Purged expired COV subscriptions");
                }
            }
        });

        // Background commits fan COV out like a network write (#889).
        let cov_fanout = super::cov_fanout::CovFanout::new(
            &super::cov_notify_context::CovNotifyContext {
                db: &db,
                network: &network,
                cov_table: &cov_table,
                cov_in_flight: &cov_in_flight,
                notification_transactions: &notification_transactions,
                comm_state: &comm_state,
                config: &config,
            },
            &event_suppressions,
            &learned_routers,
            &device_bindings,
        );

        // Each pass commits the transitions its changes cause with them
        // (#2535); see `background`.
        let fault_detection_task = if config.enable_fault_detection {
            let fanout = cov_fanout.clone();
            Some(spawn_owned(audit_owner.clone(), move || {
                background::fault_detection(fanout)
            }))
        } else {
            None
        };

        let event_enrollment_task = if config.enable_event_enrollment {
            let ee_period = event_enrollment_period(config.event_enrollment_interval_secs);
            // The delay countdown converts seconds to passes with
            // `ceil(delay / period)`, so the evaluator needs the actual,
            // clamped interval — not the raw config value.
            Some(
                super::event_enrollment_lifecycle::spawn_event_enrollment_task(
                    super::event_enrollment_lifecycle::EventEnrollmentTask {
                        db: Arc::clone(&db),
                        network: Arc::clone(&network),
                        comm_state: Arc::clone(&comm_state),
                        learned_routers: Arc::clone(&learned_routers),
                        notification_transactions: Arc::clone(&notification_transactions),
                        device_bindings: Arc::clone(&device_bindings),
                        suppressions: Arc::clone(&event_suppressions),
                        period: ee_period,
                        config: Arc::clone(&config),
                    },
                ),
            )
        } else {
            None
        };

        let trend_log_task = Some(spawn_owned(audit_owner.clone(), || {
            crate::trend_log::run(Arc::clone(&db))
        }));

        let schedule_fanout = cov_fanout.clone();
        // The 60-second Schedule pass, and the work an application's own
        // `add` or `remove` queued on the database (`crate::membership`).
        let schedule_tick_task = Some(spawn_owned(audit_owner.clone(), move || {
            background::schedules(schedule_fanout, membership, command_runs)
        }));

        // Intrinsic-reporting task: once a second, and when a pending
        // transition's `Time_Delay` ends, it fires the transitions that are due
        // and sends their EventNotifications. The per-write path only *seeds* a
        // pending transition, due its delay after the write (see
        // `fire_event_notifications`); this task is the sole confirmer, so
        // repeated writes cannot shorten the delay (ASHRAE 135-2020 Clause
        // 13.3). Runs unconditionally like the trend-log task — a no-pending
        // sweep is a cheap empty iteration.
        //
        // It is also what carries Reliability into event-state-detection when
        // no writer evaluated the change. Per Clause 13.2.2 the FAULT
        // determination is a standing condition, so each sweep re-derives it
        // from the object's current `Reliability`. A network or local write and
        // the fault detector's object-owned hooks above commit the transition
        // with the change (#2535), so the sweep finds it done; a setter that
        // bypasses them reaches detection here. `enable_fault_detection`
        // governs only whether those hooks run every 10 seconds, never whether
        // an existing Reliability is honored.
        //
        // Six of the nine wired object types have no route that can set
        // Reliability, so the fault path is correct but inert on them (#218).
        let intrinsic_reporting_task = Some(spawn_owned(audit_owner.clone(), || {
            intrinsic::run(cov_fanout.clone())
        }));

        // Reports what changed while a confirmed report was outstanding, once
        // it is acknowledged (#896).
        let cov_revisit_task = Some(spawn_owned(audit_owner.clone(), || {
            cov_fanout.clone().run_revisits()
        }));

        let binary_lighting_operation_task =
            Some(spawn_operation(cov_fanout, monotonic, timer_runner));

        let broadcaster = super::broadcaster::BroadcasterState::new(
            &network,
            &request_tasks,
            &config,
            &db,
            &comm_state,
        );
        let server = Self {
            target_audit,
            config,
            discovery_limiter,
            time_sync_limiter,
            clock,
            network: Some(network),
            broadcaster,
            transport_cleanup: None,
            transport_cleanup_error: None,
            network_number_task,
            db,
            cov_table,
            cov_counters,
            seg_ack_senders,
            seg_send_permits,
            cov_in_flight,
            learned_routers,
            notification_transactions,
            confirmed_request_tracker,
            device_bindings,
            comm_state,
            dcc_timer,
            dcc_outcomes,
            event_suppressions,
            mutation_decisions,
            dispatch_task: Some(dispatch_task),
            request_tasks,
            cov_purge_task: Some(cov_purge_task),
            fault_detection_task,
            event_enrollment_task,
            trend_log_task,
            schedule_tick_task,
            intrinsic_reporting_task,
            binary_lighting_operation_task,
            cov_revisit_task,
            local_mac,
            seated,
            execution,
        };
        boxed(|| server.execute_initial_staging_plans()).await;
        outputs.complete();
        Ok(server)
    }
}

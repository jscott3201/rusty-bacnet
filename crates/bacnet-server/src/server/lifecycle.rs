use super::*;
#[path = "background_lifecycle.rs"]
mod background;
#[path = "intrinsic_reporting_lifecycle.rs"]
mod intrinsic;
#[path = "fanout_tail.rs"]
pub(super) mod tail;

#[path = "lifecycle_period.rs"]
mod period;
#[path = "start_tasks.rs"]
mod tasks;
use super::audit_recipient_routes::AuditRoutes;
use super::binary_lighting_lifecycle::spawn_binary_lighting_operation_task as spawn_operation;
use super::heap_futures::boxed;
use super::server_seat::{Link, Own, Started};

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
            apdus: apdu_rx,
            audit_routes,
            controls: network_controls,
            notification_transactions,
            seated,
            execution,
            dcc: node_dcc,
            number: seat_number,
            client_copies,
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
                // The refusal is the installation's; a cleanup that fails
                // too is logged beside it.
                if let Err(cleanup) = L::abandon(network).await {
                    warn!(%cleanup, "startup cleanup after a refused Audit installation failed");
                }
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

        let network_number_task = tasks::spawn_network_number(
            network_controls,
            seat_number,
            &network,
            &db,
            &config,
            &target_audit,
        );
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
                client_copies,
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
        let dispatch_task = Self::spawn_dispatch_loop(
            audit_owner.clone(),
            tasks::DispatchLoop {
                segment_timeout,
                dispatch_context,
                requests,
                network_dispatch,
                config_dispatch,
                notification_transactions_dispatch,
                apdu_rx,
            },
        );

        let cov_table_for_purge = Arc::clone(&cov_table);
        let cov_purge_task = tasks::spawn_cov_purge(audit_owner.clone(), cov_table_for_purge);

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
            Some(tasks::spawn_fault_detection(audit_owner.clone(), fanout))
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

        let trend_log_task = Some(tasks::spawn_trend_log(audit_owner.clone(), &db));

        let schedule_fanout = cov_fanout.clone();
        let schedule_tick_task = Some(tasks::spawn_schedules(
            audit_owner.clone(),
            schedule_fanout,
            membership,
            command_runs,
        ));

        let intrinsic_reporting_task = Some(tasks::spawn_intrinsic_reporting(
            audit_owner.clone(),
            &cov_fanout,
        ));

        let cov_revisit_task = Some(tasks::spawn_cov_revisits(audit_owner.clone(), &cov_fanout));

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
            database_handed_back: false,
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
        // The one wait since the database was shared, with dispatch running:
        // its work runs in a task of the server's (#2781).
        boxed(|| server.execute_initial_staging_plans()).await;
        outputs.complete();
        Ok(server)
    }
}

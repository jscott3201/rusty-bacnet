//! A server's start, phase by phase (#2558): what each phase must follow, the
//! locks it takes, and what a refusal or a dropped start leaves behind.
//! [`BACnetServer::start_on_heap`] runs the phases and owns every wait; each
//! phase hands the next what only it makes, by value ([`Prepared`],
//! `Started`, [`Linked`]), so none can run before the one it follows.
//!
//! 1. **Prepare** ([`BACnetServer::prepare`], no wait). In order: the
//!    settings are checked against the link's transport before anything
//!    changes the database or starts the link (a node checks the same before
//!    its own link starts); the database is bound to the identity's Device;
//!    the Audit routes are prepared and checked against the database; the
//!    Audit Log forwarder's configured state is set from that binding while
//!    no network number is known; the settled config is shared from here on;
//!    the clocks are bound to the timed objects, so each starts its output on
//!    the server's clocks; the outputs start ([`StartedOutputs`]); the
//!    membership waker is armed. A refused setting hands the database back
//!    untouched, a refused Audit route bound to the Device; neither has
//!    started an output or the link.
//! 2. **Link** (`Link::start`, the first wait). It takes the bound database
//!    (a registered Network Port is reserved and published there), the
//!    settled config and the prepared Audit routes. Refused, the start
//!    releases what the outputs began and hands the database back
//!    ([`StartedOutputs::refuse`], which also withdraws a published port).
//!    Dropped here, the link's start goes first, with the cleanup it owns,
//!    then [`StartedOutputs`] releases the outputs, and the database goes
//!    with the start.
//! 3. **Target Audit** (`TargetAudit::install`, no wait unless refused). The
//!    notification retries are set first, on the transactions the
//!    installation takes. It installs while the database is still the start's
//!    own, so a refusal can hand it back. The link is let go of first, so a
//!    Network Port it published can be withdrawn once its lease has gone
//!    (`Link::abandon`, the second wait: a transport of the server's own is
//!    stopped, which needs the network layer's only handle, so nothing may
//!    clone it before this point; a seat's link is the node's and stays up);
//!    then the outputs are released and the database handed back. A failed
//!    `abandon` is only logged. Dropped during `abandon`, the stop goes no
//!    further, the outputs are released, and the database goes with the
//!    start.
//! 4. **Assemble** ([`BACnetServer::assemble`], no wait, no early return).
//!    The request limiters keep the start's identity. The database is shared
//!    ([`StartedOutputs::share`]); nothing after this can refuse, so nothing
//!    can hand it back. Then the server's handles; the network-number task;
//!    the target Audit owner set on the request set, before any request task
//!    can be spawned; the dispatch context and the command runner, made
//!    before the dispatch loop takes the context. The tasks are spawned in
//!    this order ([`tasks`]): network number, dispatch loop, COV purge, fault
//!    detection (opt-in), Event Enrollment (opt-in), trend logs, Schedules,
//!    intrinsic reporting, COV revisits, Binary Lighting operation. From the
//!    first spawn to the server that holds every handle, nothing waits or
//!    returns. A known limit, older than this split: a panic in that stretch
//!    leaves the tasks already spawned running, since dropping a `JoinHandle`
//!    detaches its task rather than aborting it.
//! 5. **Staging** (`execute_initial_staging_plans`, the last wait). The
//!    Staging plans due at start-up run in a task of the server's request
//!    set, which the start waits for and then joins (#2781): the dispatch
//!    loop already serves requests, so a start left unpolled holds none up.
//!    Dropped here, the server drops first (it seals, aborts its tasks, the
//!    Staging task among them, and lets the database go once they have let go
//!    of it), then the start's clone of the target Audit owner, then
//!    [`StartedOutputs`], which releases the outputs under a free guard, or
//!    from a task once the guard is free. Completed, it releases nothing.
//!
//! **Locks.** Phases 1 to 4 wait for no lock: the database is the start's own
//! until it is shared, and the other handles are new, so nothing else holds
//! them. The Staging task takes its locks in a task of its own, in the
//! server's order (object database before COV table).
//!
//! **Drops.** The start binds each phase's output in the order the values
//! were made, and each phase takes its inputs in the start's own order, so a
//! refusal or a dropped start drops what it holds in the same order as before
//! the split.

use super::super::request_tasks::RequestTasks;
use super::super::server_seat::{Apdus, Seated};
use super::start_outputs::StartedOutputs;
use super::*;
use bacnet_endpoint_core::seat::{ClientCopies, SeatNumber, ServerDcc};

/// What [`BACnetServer::prepare`] hands the link's start: the link, the
/// checked settings, the Audit routes, the shared config and clocks, and the
/// database, its outputs started, held until the start completes.
pub(super) struct Prepared<L> {
    pub(super) link: L,
    pub(super) device_bindings: DeviceBindingTable,
    pub(super) segment_timeout: Duration,
    pub(super) request_tasks: Arc<RequestTasks>,
    pub(super) audit_routes: AuditRoutes,
    pub(super) config: Arc<ServerConfig>,
    pub(super) clock: Option<Arc<ServerClock>>,
    pub(super) monotonic: MonotonicClocks,
    pub(super) outputs: StartedOutputs,
    pub(super) membership: Arc<tokio::sync::Notify>,
}

/// What the start hands [`BACnetServer::assemble`] once its link is up and
/// target Audit installed: what [`Prepared`] held but the database and the
/// Audit routes, what the link gave, and the installed target Audit.
pub(super) struct Linked<T: TransportPort + 'static, A> {
    pub(super) config: Arc<ServerConfig>,
    pub(super) device_bindings: DeviceBindingTable,
    pub(super) segment_timeout: Duration,
    pub(super) request_tasks: Arc<RequestTasks>,
    pub(super) clock: Option<Arc<ServerClock>>,
    pub(super) monotonic: MonotonicClocks,
    pub(super) membership: Arc<tokio::sync::Notify>,
    pub(super) network: Arc<NetworkLayer<T>>,
    pub(super) apdu_rx: A,
    pub(super) network_controls:
        Option<mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>>,
    pub(super) notification_transactions: Arc<NotificationTransactions>,
    pub(super) seated: Option<Seated>,
    pub(super) execution: crate::device_view::DeviceExecution,
    pub(super) node_dcc: Option<ServerDcc>,
    pub(super) seat_number: Option<SeatNumber>,
    pub(super) client_copies: Option<ClientCopies>,
    pub(super) local_mac: MacAddr,
    pub(super) target_audit: Option<Arc<super::super::audit_recipient::TargetAudit<T>>>,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Phase 1 (see the [module](self) documentation): check the settings
    /// against `link`'s transport, bind and prepare the database, and start
    /// its outputs. Each refusal hands the database back.
    pub(super) fn prepare<L: Link<T>>(
        mut config: ServerConfig,
        mut db: ObjectDatabase,
        link: L,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> Result<Prepared<L>, StartError> {
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
        Ok(Prepared {
            link,
            device_bindings,
            segment_timeout,
            request_tasks,
            audit_routes,
            config,
            clock,
            monotonic,
            outputs,
            membership,
        })
    }

    /// Phase 4 (see the [module](self) documentation): share the database,
    /// make the server's handles, spawn its tasks and make the server. It
    /// neither waits nor fails. It hands back, beside the server, the clone
    /// of the target Audit owner it made from `linked`, which the start holds
    /// until it ends, so that clone still drops after the server and before
    /// the outputs at the Staging wait.
    pub(super) fn assemble<A: Apdus>(
        linked: Linked<T, A>,
        outputs: &mut StartedOutputs,
    ) -> (tasks::AuditOwner, Self) {
        let Linked {
            config,
            device_bindings,
            segment_timeout,
            request_tasks,
            clock,
            monotonic,
            membership,
            network,
            apdu_rx,
            network_controls,
            notification_transactions,
            seated,
            execution,
            node_dcc,
            seat_number,
            client_copies,
            local_mac,
            target_audit,
        } = linked;
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
        (audit_owner, server)
    }
}

use super::*;
#[path = "background_lifecycle.rs"]
mod background;
#[path = "intrinsic_reporting_lifecycle.rs"]
mod intrinsic;
#[path = "fanout_tail.rs"]
pub(super) mod tail;

#[path = "lifecycle_period.rs"]
mod period;
#[path = "start_phases.rs"]
mod phases;
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

    /// Every refusal hands the database back ([`StartError`], #2752). The
    /// phases, what each must follow, and what a refusal or a dropped start
    /// leaves behind: [`phases`].
    async fn start_on_heap<L: Link<T>>(
        config: ServerConfig,
        db: ObjectDatabase,
        link: L,
        clock_config: Option<ClockConfig>,
        configured_device_bindings: Vec<DeviceBinding>,
    ) -> Result<Self, StartError> {
        let phases::Prepared {
            link,
            device_bindings,
            segment_timeout,
            request_tasks,
            audit_routes,
            config,
            clock,
            monotonic,
            mut outputs,
            membership,
        } = Self::prepare(config, db, link, clock_config, configured_device_bindings)?;

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

        // The start holds `assemble`'s clone of the target Audit owner to its
        // end: bound before the server, it drops after it.
        let (_audit_owner, server) = Self::assemble(
            phases::Linked {
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
            },
            &mut outputs,
        );
        // The one wait since the database was shared, with dispatch running:
        // its work runs in a task of the server's (#2781).
        boxed(|| server.execute_initial_staging_plans()).await;
        outputs.complete();
        Ok(server)
    }
}

use super::*;
use bacnet_types::enums::AuditOperation;

#[path = "comm_state.rs"]
mod comm_state;
pub(crate) use comm_state::CommState;
pub use comm_state::DccState;

/// Last-owner destruction aborts the timer even if server Drop could not acquire
/// the async slot while a request was replacing it.
#[derive(Default)]
pub(crate) struct TimerSlot(pub(crate) Option<JoinHandle<()>>);
impl std::ops::Deref for TimerSlot {
    type Target = Option<JoinHandle<()>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TimerSlot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for TimerSlot {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

// Borrow the handle in its owning slot through the join. Cancelling the caller
// leaves the (possibly already aborted) handle available for the next cleanup.
pub(super) async fn cancel(slot: &mut Option<JoinHandle<()>>) {
    if let Some(task) = slot.as_mut() {
        task.abort();
        let _ = task.await;
    }
    *slot = None;
}

/// Commit accepted WARMSTART/COLDSTART while the caller holds the timer slot.
/// Abort first so the old deadline cannot outlive this transition. Joining
/// remains a separate cancellable cleanup step; acceptance must enable now.
/// This is not a DCC request or timer expiry and records neither outcome/Audit.
pub(super) fn enable_for_restart(
    slot: &Option<JoinHandle<()>>,
    comm: &CommState,
    cov_resume: &crate::cov::timed::TimedStore,
) {
    if let Some(task) = slot {
        task.abort();
    }
    comm.set(DccState::Enable);
    cov_resume.rearm();
}

/// One DeviceCommunicationControl request and where it came from.
pub(super) struct DccRequest<'a> {
    pub(super) service_data: &'a [u8],
    pub(super) source_mac: &'a [u8],
    pub(super) source: Option<&'a bacnet_encoding::npdu::NpduAddress>,
    /// This network's own number as read for the request, against which a
    /// routed source restriction entry names a direct requester (#1458).
    pub(super) local_network: Option<u16>,
}

/// Apply a DCC request and own its revert timer. Whenever communication is
/// enabled again, by the request or when the timer expires, `cov_resume` is
/// rearmed so timestamped COV changes held meanwhile go out promptly (#856).
///
/// With target Audit configured, each change carried out is reported
/// (Table 19-5, #1387): DEVICE_DISABLE_COMM for DISABLE_INITIATION and
/// DEVICE_ENABLE_COMM for ENABLE, from `audit`'s requester, and
/// DEVICE_ENABLE_COMM from this device when a timed disable runs out. A
/// refused request changes nothing and reports nothing. The record is
/// admitted under the timer slot with the change, so records follow the
/// order changes take effect in; the disable's goes out under the state it
/// reports, as audit notifications do (#1370).
pub(super) async fn replace<T: TransportPort + 'static>(
    services: &super::request_services::RequestServices<T>,
    request: DccRequest<'_>,
    request_tasks: &super::request_tasks::RequestTaskSpawner,
    cov_resume: &crate::cov::timed::TimedStore,
    audit: &mut super::audit_reporter::WriteAudit<'_, T>,
) -> Result<dcc_outcomes::DccMetadata, handlers::device_mgmt::DccFailure> {
    let super::request_services::RequestServices {
        dcc_timer: timer,
        comm_state,
        config,
        ..
    } = services;
    let DccRequest {
        service_data,
        source_mac,
        source,
        local_network,
    } = request;
    // Decode and validate once, retaining only non-secret proposed state and
    // metadata. Never change live state before a cancellable await.
    let (proposed, duration) =
        handlers::device_mgmt::validate_dcc(service_data, &config.dcc_password, config.dcc_policy)?;
    let mode = bacnet_types::enums::EnableDisable::from(proposed).to_raw();
    // Short-circuit source refusal before touching the shared budget. ENABLE
    // never checks it. A successful charge precedes every cancellable await.
    if config
        .dcc_source_restriction
        .as_ref()
        .is_some_and(|restriction| {
            config.dcc_policy != DccPolicy::RequirePassword
                || !restriction.allows(source_mac, source, local_network)
        })
        || (proposed == DccState::DisableInitiation && !request_tasks.admit_dcc_disable())
    {
        return Err(handlers::device_mgmt::DccFailure {
            error: Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            },
            outcome: dcc_outcomes::DccOutcome::PolicyDenied,
            metadata: dcc_outcomes::DccMetadata {
                mode: Some(mode),
                duration,
            },
        });
    }
    let mut slot = timer.lock().await;
    // The database guard the record needs is taken before the old timer is
    // cancelled, so no suspension separates that cancel from the commit.
    // Expiry takes the slot and then the database too, in the same order.
    let mut audit_db = match &config.audit_reporters {
        Some(_) => Some(services.db.write().await),
        None => None,
    };
    cancel(&mut slot).await;
    // Replacement, expiry, and shutdown share this linearization boundary.
    // No suspension between the live commit and installing the new owner.
    comm_state.set(proposed);
    if proposed == DccState::Enable {
        cov_resume.rearm();
    }
    if let Some(minutes) = duration {
        let owner = Arc::downgrade(timer);
        let comm = Arc::clone(comm_state);
        let cov_resume = cov_resume.clone();
        // Only a disable running out re-enables communication; an ENABLE's
        // own timer changes nothing when it fires, so it reports nothing.
        let expiry_audit = DccAudit::of(services).filter(|_| proposed != DccState::Enable);
        // The duration runs from the commit, not from the task's first poll.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(minutes as u64 * 60);
        **slot = Some(tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if let Some(owner) = owner.upgrade() {
                // An old task waiting here can be aborted and joined while a
                // replacement holds the slot; it never joins or removes itself.
                let _slot = owner.lock().await;
                let mut db = match &expiry_audit {
                    Some(audit) => Some(audit.db.write().await),
                    None => None,
                };
                comm.set(DccState::Enable);
                cov_resume.rearm();
                if let (Some(audit), Some(db)) = (&expiry_audit, db.as_deref_mut()) {
                    audit.record_expiry(db);
                }
                debug!(
                    "DCC timer expired after {} min, state reverted to ENABLE",
                    minutes
                );
            }
            // A timer nothing cancelled may outlive every other handle on
            // the database (#1560).
            if let Some(audit) = expiry_audit {
                audit.release();
            }
        }));
    }
    if let Some(db) = audit_db.as_deref_mut() {
        audit.device_communication(
            db,
            if proposed == DccState::Enable {
                AuditOperation::DEVICE_ENABLE_COMM
            } else {
                AuditOperation::DEVICE_DISABLE_COMM
            },
        );
    }
    Ok(dcc_outcomes::DccMetadata {
        mode: Some(mode),
        duration,
    })
}

/// The handles a DCC change's Audit record is admitted with (#1387), held
/// by a disable's timer for the record its expiry owes. Present only with
/// target Audit configured, so a server without it never takes the
/// database guard for DCC.
pub(super) struct DccAudit<T: TransportPort + 'static> {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<T>>,
    transactions: Arc<NotificationTransactions>,
    config: Arc<ServerConfig>,
}

impl<T: TransportPort + 'static> DccAudit<T> {
    fn of(services: &super::request_services::RequestServices<T>) -> Option<Self> {
        services.config.audit_reporters.as_ref()?;
        Some(Self {
            db: Arc::clone(&services.db),
            network: Arc::clone(&services.network),
            transactions: Arc::clone(&services.notification_transactions),
            config: Arc::clone(&services.config),
        })
    }

    /// Let go of the database through [`drop_database_off_runtime`], for a
    /// timer that ends on its own.
    fn release(self) {
        drop(drop_database_off_runtime(self.db));
    }

    /// Report a timed disable running out: DEVICE_ENABLE_COMM, with this
    /// device as its source and no invoke ID, since no request caused it.
    fn record_expiry(&self, db: &mut ObjectDatabase) {
        if let Some(mut audit) = super::audit_reporter::WriteAudit::local(
            &self.config,
            &self.network,
            &self.transactions,
            db,
        ) {
            audit.device_communication(db, AuditOperation::DEVICE_ENABLE_COMM);
        }
    }
}

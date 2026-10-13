//! Borrowed handles shared by the COV notification entry points.
use super::*;
use crate::cov::{AtomicCovCounters, CovInFlightTracker};

/// The server handles a COV notification pass reads: object database,
/// network, subscription table, in-flight permits, notification transactions,
/// the DCC communication state and the server config.
pub(super) struct CovNotifyContext<'a, T: TransportPort + 'static> {
    pub(super) db: &'a Arc<RwLock<ObjectDatabase>>,
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) cov_table: &'a Arc<RwLock<CovSubscriptionTable>>,
    pub(super) cov_in_flight: &'a Arc<Semaphore>,
    pub(super) notification_transactions: &'a Arc<NotificationTransactions>,
    pub(super) comm_state: &'a Arc<CommState>,
    /// Shared, so a task can own it without copying it (#1521).
    pub(super) config: &'a Arc<ServerConfig>,
}

/// A [`CovNotifyContext`] plus the subscription table's in-flight tracker and
/// counters, both captured under the table lock before a fanout pass starts.
pub(super) struct CovFanoutHandles<'a, 'b, T: TransportPort + 'static> {
    pub(super) ctx: &'a CovNotifyContext<'b, T>,
    pub(super) in_flight_tracker: &'a Arc<CovInFlightTracker>,
    pub(super) counters: &'a Arc<AtomicCovCounters>,
}

//! The access-event capability: an Access Point's completed-authorization
//! event batches.

use alloc::vec::Vec;

use bacnet_types::error::Error;

use crate::access_control::{AccessEventReport, CommittedAccessEvent};

/// An Access Point that installs a completed authorization's events as one
/// batch.
///
/// The object returns itself from
/// [`BACnetObject::access_events`](crate::traits::BACnetObject::access_events);
/// an object that lends none takes no batch, which the bundled server checks
/// before it evaluates an authorization. Not public API yet (#2179).
#[doc(hidden)]
pub trait AccessEvents {
    /// Install at most four ordered authorization event reports atomically.
    ///
    /// Validate every report and timestamp before changing any served
    /// state. On error, leave all event fields untouched. On success, retain
    /// each installed intermediate state and its clock frame for post-commit
    /// COV preparation. The native Access Point refuses out-of-service or
    /// faulty operation and timestamps omitted times. Callers must never
    /// replace this operation with a fallible per-report loop.
    fn record_access_event_batch(
        &mut self,
        reports: &[AccessEventReport],
    ) -> Result<Vec<CommittedAccessEvent>, Error>;
}

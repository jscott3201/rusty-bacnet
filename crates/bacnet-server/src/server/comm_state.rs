//! The server's DeviceCommunicationControl state (Clause 16.1).
//!
//! This module sits under `dcc_timer`, which owns changes from accepted DCC
//! requests, timer expiry and accepted WARMSTART/COLDSTART. Everything else
//! holding a [`CommState`] reads it.

use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_types::enums::EnableDisable;

#[cfg(test)]
#[path = "comm_state_tests.rs"]
mod tests;

/// The communication state DeviceCommunicationControl has put the server in
/// (Clause 16.1).
///
/// A request for the deprecated DISABLE value is always refused with
/// `SERVICES` / `SERVICE_REQUEST_DENIED` (Clause 16.1.2), whatever the
/// [`DccPolicy`](crate::server::DccPolicy), so the server is only ever in one of
/// these two states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DccState {
    /// Communication is enabled: ENABLE. The state at every start.
    #[default]
    Enable,
    /// DISABLE_INITIATION: the server still executes and answers requests, but
    /// starts nothing of its own except the I-Am that answers a Who-Is and
    /// audit notifications.
    DisableInitiation,
}

impl DccState {
    /// Whether the server holds back the messages it would start on its own:
    /// COV and event notifications, I-Have, unsolicited I-Am, remote writes
    /// and their Who-Is.
    pub const fn initiation_restricted(self) -> bool {
        matches!(self, Self::DisableInitiation)
    }
}

impl From<DccState> for EnableDisable {
    fn from(state: DccState) -> Self {
        match state {
            DccState::Enable => Self::ENABLE,
            DccState::DisableInitiation => Self::DISABLE_INITIATION,
        }
    }
}

/// The server's live [`DccState`], read lock-free by every path that starts
/// a message.
///
/// The setter is private to `dcc_timer`; tests reach it through
/// `set_for_test`.
#[derive(Debug, Default)]
pub(crate) struct CommState {
    initiation_restricted: AtomicBool,
}

impl CommState {
    /// The current state.
    pub(crate) fn get(&self) -> DccState {
        if self.initiation_restricted() {
            DccState::DisableInitiation
        } else {
            DccState::Enable
        }
    }

    /// Whether DCC restricts initiation now; see
    /// [`DccState::initiation_restricted`].
    pub(crate) fn initiation_restricted(&self) -> bool {
        self.initiation_restricted.load(Ordering::Acquire)
    }

    /// Commit a new state. Only `dcc_timer` calls this: when a DCC request is
    /// accepted, when its timer runs out, or when a restart is accepted.
    pub(super) fn set(&self, state: DccState) {
        self.initiation_restricted
            .store(state.initiation_restricted(), Ordering::Release);
    }

    /// Put the state in place without a DCC request, for tests that need DCC
    /// in force at a given step.
    #[cfg(test)]
    pub(crate) fn set_for_test(&self, state: DccState) {
        self.set(state);
    }
}

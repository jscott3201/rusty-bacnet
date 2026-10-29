//! The TUI's write-safety model (#964; `docs/design/tui.md` section 5):
//! the shared core in `bacnet_ops::safety` (levels, tiers, previews, the
//! ledger, the audit log and the workspace lock, #2068), and what only the
//! TUI has:
//!
//! - [`audit`]: the [`Secret`](audit::Secret) a device management password
//!   lives in, and the MS/TP join record, beside the shared log;
//! - [`manage`]: DeviceCommunicationControl and ReinitializeDevice, the
//!   device management requests, always at T3;
//! - the messages between the app and its write worker.
//!
//! The app decides what the user may do; the worker checks the same rules
//! again before it sends anything (through `bacnet_ops::safety::desk`),
//! owns the ledger and the audit log, and records every outcome even when
//! the session ends under a write.

pub(crate) mod audit;
pub(crate) mod manage;

pub(crate) use bacnet_ops::safety::{
    DEFAULT_MINUTES, Effect, FrontEnd, Gate, Level, MAX_MINUTES, Occupancy, Outcome, Planned,
    Preview, Tier, WriteSpec, ledger,
};
#[cfg(test)]
pub(crate) use bacnet_ops::safety::{Stored, plan};

use self::ledger::Entry;
use super::profile::Safety;

/// A change waiting for its confirmation: a write preview, or a device
/// management request. Both go through the one confirmation dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    /// One WriteProperty or a WritePropertyMultiple batch.
    Write(Box<Preview>),
    /// DeviceCommunicationControl or ReinitializeDevice.
    Device(Box<manage::Request>),
}

impl Change {
    /// The device it goes to, typed out to confirm at T3.
    pub(crate) fn device(&self) -> u32 {
        match self {
            Self::Write(preview) => preview.device,
            Self::Device(request) => request.device,
        }
    }

    /// Its tier, or why it is blocked.
    pub(crate) fn gate(&self) -> Gate {
        match self {
            Self::Write(preview) => preview.gate.clone(),
            Self::Device(request) => request.gate(),
        }
    }
}

/// Write session requests, UI to worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SafetyCommand {
    /// Arm a write session.
    Arm {
        /// The optional reason or ticket.
        reason: String,
        /// How long, in minutes.
        minutes: u32,
    },
    /// Disarm: by the user, or at expiry.
    Disarm {
        /// The session ran out.
        expired: bool,
    },
    /// The user declined a confirmation; recorded, nothing sent. A device
    /// management request comes without its password.
    Declined {
        /// What was declined.
        change: Change,
    },
    /// Read what the writes need and preview them.
    Prepare {
        /// Request id, echoed.
        req: u64,
        /// The writes, all to one device.
        specs: Vec<WriteSpec>,
        /// Send them as one WritePropertyMultiple.
        batch: bool,
    },
    /// Send a confirmed preview.
    Execute {
        /// Request id, echoed.
        req: u64,
        /// What was confirmed.
        preview: Box<Preview>,
    },
    /// Send a confirmed device management request.
    Manage {
        /// Request id, echoed.
        req: u64,
        /// What was confirmed, password included.
        request: Box<manage::Request>,
    },
    /// Undo every ledger entry, newest first.
    Revert {
        /// Request id, echoed.
        req: u64,
    },
    /// Leave the ledger's entries in place on exit, and say so in the log.
    Keep,
}

/// Write session reports, worker to UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SafetyEvent {
    /// The session's policy and ledger, once the worker has opened it.
    Ready {
        /// The profile's policy.
        policy: Safety,
        /// A workspace holds a ledger and an audit log; without one
        /// nothing can be armed.
        workspace: bool,
        /// This process's run, which marks the entries it wrote.
        run: String,
        /// The ledger, entries from earlier runs included.
        ledger: Vec<Entry>,
    },
    /// Armed for this many minutes.
    Armed {
        /// Minutes.
        minutes: u32,
    },
    /// Disarmed.
    Disarmed,
    /// A request was refused, and why.
    Refused {
        /// The request, if it had one.
        req: Option<u64>,
        /// Why.
        reason: String,
    },
    /// A preview to confirm.
    Preview {
        /// The request it answers.
        req: u64,
        /// The preview.
        preview: Box<Preview>,
    },
    /// Writes, or a device management request, ended: each target's label
    /// and outcome.
    Done {
        /// The request it answers.
        req: u64,
        /// Per write.
        outcomes: Vec<(String, Outcome)>,
    },
    /// The ledger changed.
    Ledger(Vec<Entry>),
    /// A revert ended with this many entries still outstanding.
    Reverted {
        /// The request it answers.
        req: u64,
        /// Entries that could not be reverted.
        failed: usize,
    },
}

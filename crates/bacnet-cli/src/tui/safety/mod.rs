//! The write-safety model (#964; `docs/design/tui.md` section 5): session
//! levels, confirmation tiers, the session ledger and the audit log.
//!
//! - [`plan`]: a requested write turned into a preview: the value to send,
//!   the pre-image, what Present_Value will do, and the tier.
//! - [`ledger`]: every change still in effect, with its pre-image, saved to
//!   the profile workspace after each change so it outlives a crash.
//! - [`audit`]: the append-only `audit.jsonl`.
//!
//! The app decides what the user may do; the worker checks the same rules
//! again before it sends anything, owns the ledger and the audit log, and
//! records every outcome even when the session ends under a write.

pub(crate) mod audit;
pub(crate) mod ledger;
pub(crate) mod lock;
pub(crate) mod plan;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use self::ledger::Entry;
use super::profile::Safety;
use crate::core::class::Class;

/// Default write session length, in minutes.
pub(crate) const DEFAULT_MINUTES: u32 = 15;

/// Longest write session, in minutes.
pub(crate) const MAX_MINUTES: u32 = 480;

/// What the session may do, always shown in the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    /// `--passive`: nothing is transmitted.
    Passive,
    /// The default: discovery, reads, finite COV and FD registrations.
    ReadOnly,
    /// A write session is armed; `dangerous` when the profile allows
    /// dangerous operations in it.
    Armed {
        /// The profile allows dangerous operations.
        dangerous: bool,
    },
    /// The worker's view of an armed session whose time has run out but
    /// which nobody has disarmed yet: read-only, named apart in the audit
    /// log. The app never shows it.
    Expired,
}

impl Level {
    /// Whether a command of `class` may run at this level. The classes are
    /// the ones the one-shot guard uses (#972): reads from read-only up,
    /// writes in an armed session, device management only in a dangerous
    /// one, and nothing passive.
    pub(crate) fn permits(self, class: Class) -> bool {
        match (self, class) {
            (Self::Passive, _) => false,
            (_, Class::Read) => true,
            (Self::Armed { .. }, Class::Write) => true,
            (Self::Armed { dangerous }, Class::DeviceManagement) => dangerous,
            (Self::ReadOnly | Self::Expired, _) => false,
        }
    }

    /// The name in audit records.
    pub(crate) fn audit_name(self) -> &'static str {
        match self {
            Self::Passive => "passive",
            Self::ReadOnly => "read-only",
            Self::Armed { dangerous: false } => "write-session",
            Self::Armed { dangerous: true } => "write-session+dangerous",
            Self::Expired => "write-session (expired)",
        }
    }
}

/// How much confirmation a change needs. Ordered: a batch takes its
/// highest member's tier. T0 (reads and discovery) needs none and never
/// reaches this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    /// A preview, then Enter.
    T1,
    /// A preview, then `y`.
    T2,
    /// A preview, then the device instance typed out.
    T3,
}

/// Whether a change may go ahead, and with what confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Allowed after this tier's confirmation.
    Allowed(Tier),
    /// Refused, and why.
    Blocked(String),
}

/// Application-tagged bytes as sent or read, with their display text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stored {
    /// The encoded value, as hex in the files.
    #[serde(serialize_with = "to_hex", deserialize_with = "from_hex")]
    pub(crate) bytes: Vec<u8>,
    /// How it reads.
    pub(crate) text: String,
}

/// The encoding of NULL, which relinquishes a slot (Clause 19.2.1).
pub(crate) const NULL: [u8; 1] = [0x00];

impl Stored {
    /// A relinquish.
    pub(crate) fn null() -> Self {
        Self {
            bytes: NULL.to_vec(),
            text: "NULL".into(),
        }
    }
}

fn to_hex<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn from_hex<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(serde::de::Error::custom("odd or non-ASCII hex"));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(serde::de::Error::custom))
        .collect()
}

/// One write the user asked for, before anything is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WriteSpec {
    /// Device instance, from the discovery table.
    pub(crate) device: u32,
    /// The object.
    pub(crate) object: ObjectIdentifier,
    /// The property.
    pub(crate) property: PropertyIdentifier,
    /// Array index, if any.
    pub(crate) index: Option<u32>,
    /// The value as typed, for `parse::parse_value`; `None` relinquishes.
    pub(crate) value: Option<String>,
    /// The priority asked for.
    pub(crate) priority: Option<u8>,
}

/// Who holds the slot a commandable write goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Occupancy {
    /// The property is not commandable.
    NotCommandable,
    /// The slot is NULL.
    Empty,
    /// It holds a value this ledger wrote.
    Ours,
    /// It holds another commander's value, which the write replaces
    /// without telling them (Clause 19.2.1.5).
    Other,
}

/// What a write does to Present_Value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    /// It changes.
    Changes {
        /// Now.
        from: String,
        /// After the write.
        to: String,
    },
    /// It stays, held by a numerically lower (stronger) priority when
    /// there is one (Clause 19.2).
    Unchanged {
        /// The slot that keeps control.
        held_by: Option<u8>,
    },
    /// The write is to some other property.
    NotPresentValue,
}

/// One write, read and ready to confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Planned {
    /// What was asked.
    pub(crate) spec: WriteSpec,
    /// The priority sent: the one asked, or 16 by default, for a
    /// commandable property; none for another.
    pub(crate) priority: Option<u8>,
    /// The value sent.
    pub(crate) value: Stored,
    /// It goes to a priority slot.
    pub(crate) commandable: bool,
    /// The pre-image: the slot's value (`None` when NULL) or the
    /// property's value (`None` when it couldn't be read).
    pub(crate) before: Option<Stored>,
    /// Who holds the slot.
    pub(crate) occupancy: Occupancy,
    /// What Present_Value does.
    pub(crate) effect: Effect,
    /// Out_Of_Service as read.
    pub(crate) out_of_service: Option<bool>,
    /// Warnings for the preview.
    pub(crate) notes: Vec<String>,
    /// Its own tier.
    pub(crate) gate: Gate,
}

impl Planned {
    /// `device 100 analog-output 1 present-value @8`.
    pub(crate) fn label(&self) -> String {
        label(
            self.spec.device,
            self.spec.object,
            self.spec.property,
            self.spec.index,
            self.priority,
        )
    }
}

/// A target as one line.
pub(crate) fn label(
    device: u32,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    priority: Option<u8>,
) -> String {
    let object = super::app::objects::object_label(object);
    let property = super::app::objects::kebab(property);
    let index = index.map(|i| format!("[{i}]")).unwrap_or_default();
    let priority = priority.map(|p| format!(" @{p}")).unwrap_or_default();
    format!("device {device} {object} {property}{index}{priority}")
}

/// A previewed change: one WriteProperty, or a WritePropertyMultiple batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preview {
    /// The device every write goes to.
    pub(crate) device: u32,
    /// The writes, in order.
    pub(crate) items: Vec<Planned>,
    /// Sent as one WritePropertyMultiple.
    pub(crate) batch: bool,
    /// The whole change's tier.
    pub(crate) gate: Gate,
}

/// How a confirmed request ended, for the audit log and the result view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Acknowledged.
    Ack,
    /// The device's mutation policy refused it: SERVICES /
    /// SERVICE_REQUEST_DENIED, shown apart from other errors.
    Denied,
    /// Another Error PDU.
    Error {
        /// Error class.
        class: u32,
        /// Error code.
        code: u32,
    },
    /// A Reject PDU.
    Reject(u8),
    /// An Abort PDU.
    Abort(u8),
    /// No answer: it may or may not have been applied.
    Timeout,
    /// Never sent, and why.
    NotSent(String),
    /// Failed some other way (the transport, say).
    Failed(String),
}

impl Outcome {
    /// From a client result.
    pub(crate) fn of(result: &Result<(), Error>) -> Self {
        let denied = |class: ErrorClass, code: ErrorCode| {
            class == ErrorClass::SERVICES && code == ErrorCode::SERVICE_REQUEST_DENIED
        };
        match result {
            Ok(()) => Self::Ack,
            Err(Error::Protocol { class, code } | Error::Structured { class, code, .. }) => {
                if denied(*class, *code) {
                    Self::Denied
                } else {
                    Self::Error {
                        class: u32::from(class.to_raw()),
                        code: u32::from(code.to_raw()),
                    }
                }
            }
            Err(Error::Reject { reason }) => Self::Reject(reason.to_raw()),
            Err(Error::Abort { reason })
                if *reason == bacnet_types::enums::AbortReason::TSM_TIMEOUT =>
            {
                Self::Timeout
            }
            Err(Error::Abort { reason }) => Self::Abort(reason.to_raw()),
            Err(Error::Timeout(_)) => Self::Timeout,
            Err(other) => Self::Failed(other.to_string()),
        }
    }

    /// Whether the change took effect: `None` when that can't be known.
    pub(crate) fn applied(&self) -> Option<bool> {
        match self {
            Self::Ack => Some(true),
            Self::Denied | Self::Error { .. } | Self::Reject(_) | Self::Abort(_) => Some(false),
            Self::NotSent(_) => Some(false),
            Self::Timeout | Self::Failed(_) => None,
        }
    }

    /// One line for the result view.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Ack => "acknowledged".into(),
            Self::Denied => "DENIED by the device's write policy (SERVICE_REQUEST_DENIED)".into(),
            Self::Error { class, code } => format!(
                "error {} / {}",
                ErrorClass::from_raw(*class as u16),
                ErrorCode::from_raw(*code as u16)
            ),
            Self::Reject(reason) => format!("rejected, reason {reason}"),
            Self::Abort(reason) => format!("aborted, reason {reason}"),
            Self::Timeout => "no answer: it may or may not have been applied".into(),
            Self::NotSent(why) => format!("not sent: {why}"),
            Self::Failed(why) => format!("failed: {why}"),
        }
    }

    /// The audit record's `outcome`.
    pub(crate) fn json(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::Ack => json!({ "result": "ack" }),
            Self::Denied => json!({ "result": "error", "class": "SERVICES",
                                    "code": "SERVICE_REQUEST_DENIED", "denied": true }),
            Self::Error { class, code } => json!({ "result": "error",
                "class": ErrorClass::from_raw(*class as u16).to_string(),
                "code": ErrorCode::from_raw(*code as u16).to_string() }),
            Self::Reject(reason) => json!({ "result": "reject", "reason": reason }),
            Self::Abort(reason) => json!({ "result": "abort", "reason": reason }),
            Self::Timeout => json!({ "result": "timeout" }),
            Self::NotSent(why) => json!({ "result": "not-sent", "why": why }),
            Self::Failed(why) => json!({ "result": "failed", "why": why }),
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
    /// The user declined a confirmation; recorded, nothing sent.
    Declined {
        /// What was declined.
        preview: Box<Preview>,
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
    /// Writes ended: each target's label and outcome.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The TUI's levels and the one-shot guard share one classification.
    #[test]
    fn levels_permit_the_shared_classes() {
        use Class::{DeviceManagement, Read, Write};
        let table = [
            (Level::Passive, [false, false, false]),
            (Level::ReadOnly, [true, false, false]),
            (Level::Expired, [true, false, false]),
            (Level::Armed { dangerous: false }, [true, true, false]),
            (Level::Armed { dangerous: true }, [true, true, true]),
        ];
        for (level, permits) in table {
            let got = [Read, Write, DeviceManagement].map(|class| level.permits(class));
            assert_eq!(got, permits, "{level:?}");
        }
    }
}

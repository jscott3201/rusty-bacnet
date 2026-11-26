//! The write-safety core every front end shares (#964, #2068;
//! `docs/design/tui.md` section 5, `docs/design/mcp.md` section 7):
//! session levels, confirmation tiers, previews, the ledger, the audit log
//! and the workspace lock, with the client half of a write.
//!
//! - [`plan`]: a requested write turned into a preview: the value to send,
//!   the pre-image, what Present_Value will do, and the tier.
//! - [`ledger`]: every change still in effect, with its pre-image, saved to
//!   the profile workspace after each change so it outlives a crash.
//! - [`audit`]: the append-only `audit.jsonl`.
//! - [`lock`]: the workspace's lock, so one process at a time changes the
//!   ledger.
//! - [`desk`]: the client half: read the pre-images and plan, send a
//!   confirmed preview through the ledger and the audit log, and revert.
//!
//! The TUI and the MCP server decide who may confirm what; [`desk`] checks
//! the same rules again before it sends anything, and records every outcome
//! even when the caller goes away under a write.

pub mod audit;
pub mod desk;
pub mod file;
pub mod ledger;
pub mod lock;
mod place;
pub mod plan;
mod send;

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError, SegmentationError};
use bacnet_types::primitives::ObjectIdentifier;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::class::Class;
use bacnet_ops::DeviceAddress;
use bacnet_ops::octets::hex;

/// What a write does to Present_Value: `bacnet-ops`' analysis, which the
/// planner builds on.
pub use bacnet_ops::write::Effect;
pub use place::{Unsent, reach, revert_address, turn};
pub use send::item_json;

/// Default write session length, in minutes.
pub const DEFAULT_MINUTES: u32 = 15;

/// Longest write session, in minutes.
pub const MAX_MINUTES: u32 = 480;

/// How long a preview waits for its confirmation before it is declined:
/// the TUI's dialog, an agent's request to it, and an MCP confirmation.
pub const PREVIEW_LIFETIME: std::time::Duration = std::time::Duration::from_secs(120);

/// Which front end made a change or a record: written into the ledger and
/// the audit log, so one front end's view lists another's changes as
/// theirs.
///
/// It is written as its name, a string; a name this release doesn't know
/// (a later release's front end) loads as [`FrontEnd::Other`] and is
/// written back as it was, so the entry survives a save unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FrontEnd {
    /// The terminal UI; what a ledger written before #2068 holds.
    #[default]
    Tui,
    /// The MCP server.
    Mcp,
    /// A front end this release doesn't know, by the name it was written
    /// with: its entries still load, and revert like any other.
    Other(String),
}

impl FrontEnd {
    /// Its name: `tui`, `mcp`, or the name another release wrote.
    pub fn name(&self) -> &str {
        match self {
            Self::Tui => "tui",
            Self::Mcp => "mcp",
            Self::Other(name) => name,
        }
    }

    /// True for the TUI, the default a ledger entry leaves out.
    pub fn is_tui(&self) -> bool {
        *self == Self::Tui
    }
}

impl Serialize for FrontEnd {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for FrontEnd {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(match name.as_str() {
            "tui" => Self::Tui,
            "mcp" => Self::Mcp,
            _ => Self::Other(name),
        })
    }
}

/// This process's run, which marks the ledger entries it writes, so the
/// next launch can tell which ones an earlier run left.
pub fn run_id() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(|| {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("{}-{millis}", std::process::id())
    })
}

/// What the session may do, always shown in the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
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
    pub fn permits(self, class: Class) -> bool {
        match (self, class) {
            (Self::Passive, _) => false,
            (_, Class::Read) => true,
            (Self::Armed { .. }, Class::Write) => true,
            (Self::Armed { dangerous }, Class::DeviceManagement) => dangerous,
            (Self::ReadOnly | Self::Expired, _) => false,
        }
    }

    /// The name in audit records.
    pub fn audit_name(self) -> &'static str {
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
pub enum Tier {
    /// A preview, then Enter.
    T1,
    /// A preview, then `y`.
    T2,
    /// A preview, then the device instance typed out.
    T3,
}

/// Whether a change may go ahead, and with what confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// Allowed after this tier's confirmation.
    Allowed(Tier),
    /// Refused, and why.
    Blocked(String),
}

impl Gate {
    /// `T1`, `T2`, `T3` or `blocked`, as the audit log names it.
    pub fn name(&self) -> String {
        match self {
            Self::Allowed(tier) => format!("{tier:?}"),
            Self::Blocked(_) => "blocked".into(),
        }
    }
}

/// Application-tagged bytes as sent or read, with their display text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// The encoded value, as hex in the files.
    #[serde(serialize_with = "to_hex", deserialize_with = "from_hex")]
    pub bytes: Vec<u8>,
    /// How it reads.
    pub text: String,
}

/// The encoding of NULL, which relinquishes a slot (Clause 19.2.1).
pub const NULL: [u8; 1] = [0x00];

impl Stored {
    /// A relinquish.
    pub fn null() -> Self {
        Self {
            bytes: NULL.to_vec(),
            text: "NULL".into(),
        }
    }
}

fn to_hex<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&hex(bytes))
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
pub struct WriteSpec {
    /// Where the device answers: the write is read, planned, gated and sent
    /// at this address, and its ledger entry keeps it (#2560).
    pub address: DeviceAddress,
    /// Its device instance, for the labels, the typed confirmation and the
    /// policies that name devices by instance.
    pub device: u32,
    /// The object.
    pub object: ObjectIdentifier,
    /// The property.
    pub property: PropertyIdentifier,
    /// Array index, if any.
    pub index: Option<u32>,
    /// The value as typed, for `parse::parse_value_for`; `None` relinquishes.
    pub value: Option<String>,
    /// The priority asked for.
    pub priority: Option<u8>,
}

/// Who holds the slot a commandable write goes to: `bacnet-ops`' answer
/// ([`bacnet_ops::write::Occupancy`]) with the ledger's, which says whose
/// command a held slot is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupancy {
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

/// One write, read and ready to confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// What was asked.
    pub spec: WriteSpec,
    /// The priority sent: the one asked, or 16 by default, for a
    /// commandable property; none for another.
    pub priority: Option<u8>,
    /// The value sent.
    pub value: Stored,
    /// It goes to a priority slot.
    pub commandable: bool,
    /// The pre-image: the slot's value (`None` when NULL) or the
    /// property's value (`None` when it couldn't be read).
    pub before: Option<Stored>,
    /// Who holds the slot.
    pub occupancy: Occupancy,
    /// What Present_Value does.
    pub effect: Effect,
    /// Out_Of_Service as read.
    pub out_of_service: Option<bool>,
    /// Warnings for the preview.
    pub notes: Vec<String>,
    /// Its own tier.
    pub gate: Gate,
}

impl Planned {
    /// `device 100 analog-output 1 present-value @8`.
    pub fn label(&self) -> String {
        label(
            self.spec.device,
            self.spec.object,
            self.spec.property,
            self.spec.index,
            self.priority,
        )
    }

    /// A relinquish: NULL to a slot, typed or from `n`.
    pub fn relinquishes(&self) -> bool {
        self.value.bytes == NULL
    }
}

/// A target as one line.
pub fn label(
    device: u32,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    priority: Option<u8>,
) -> String {
    let object = bacnet_ops::label::object_label(object);
    let property = bacnet_ops::label::kebab(property);
    let index = index.map(|i| format!("[{i}]")).unwrap_or_default();
    let priority = priority.map(|p| format!(" @{p}")).unwrap_or_default();
    format!("device {device} {object} {property}{index}{priority}")
}

/// A previewed change: one WriteProperty, or a WritePropertyMultiple batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The address every write goes to, the one its pre-images were read
    /// at.
    pub address: DeviceAddress,
    /// That device's instance.
    pub device: u32,
    /// The writes, in order.
    pub items: Vec<Planned>,
    /// Sent as one WritePropertyMultiple.
    pub batch: bool,
    /// The whole change's tier.
    pub gate: Gate,
}

impl Preview {
    /// True when `fresh`, read again just before sending, plans the same
    /// writes as this confirmed preview: the same pre-images, slot holders
    /// and priorities.
    pub fn unchanged(&self, fresh: &Preview) -> bool {
        self.items.len() == fresh.items.len()
            && self.items.iter().zip(&fresh.items).all(|(a, b)| {
                (a.before == b.before)
                    && (a.occupancy == b.occupancy)
                    && (a.priority == b.priority)
                    && (a.commandable == b.commandable)
            })
    }

    /// The service it goes out as.
    pub fn service(&self) -> &'static str {
        if self.batch {
            "WritePropertyMultiple"
        } else {
            "WriteProperty"
        }
    }
}

/// How a confirmed request ended, for the audit log and the result view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Acknowledged.
    Ack,
    /// The device's mutation policy refused it: SERVICES /
    /// SERVICE_REQUEST_DENIED, shown apart from other errors.
    Denied,
    /// Another Error PDU, or a Reject or Abort PDU, with its typed class and
    /// code or reason (#1814).
    Refused(RemoteError),
    /// No answer: it may or may not have been applied.
    Timeout,
    /// Never sent, and why.
    NotSent(String),
    /// Failed some other way (the transport, say).
    Failed(String),
}

impl Outcome {
    /// From a client result.
    pub fn of(result: &Result<(), Error>) -> Self {
        let error = match result {
            Ok(()) => return Self::Ack,
            // A timeout after a router said it was busy is a timeout too.
            Err(Error::Timeout(_) | Error::RouterBusyTimeout(_)) => return Self::Timeout,
            // The node's DeviceCommunicationControl stopped it (#2617):
            // before it went out it was never sent; once it had, only its
            // retry was held back, and whether it was applied is unknown.
            Err(Error::InitiationDisabled { sent: false }) => {
                return Self::NotSent(
                    "DeviceCommunicationControl has disabled initiation on this node".into(),
                );
            }
            Err(error @ Error::InitiationDisabled { sent: true }) => {
                return Self::Failed(error.to_string());
            }
            // The client's stop ended it (#1833): while it waited for its
            // turn it was never sent; once it may have gone out, whether it
            // was applied is unknown, as for a withheld retry.
            Err(error @ Error::RequestCutOff { sent: false }) => {
                return Self::NotSent(error.to_string());
            }
            Err(error @ Error::RequestCutOff { sent: true }) => {
                return Self::Failed(error.to_string());
            }
            // The client's or its link's own state refused it before
            // anything of it went out: a link that stopped under a request
            // already sent is a cut-off above, never this (#1833).
            Err(error @ Error::State(..)) => return Self::NotSent(error.to_string()),
            // A request too long for one APDU that the client may not send in
            // segments, or that the device can't take in segments or in so
            // many: the client refuses each before anything goes out (#2617).
            Err(
                error @ Error::Segmentation(
                    SegmentationError::LocalCannotTransmit
                    | SegmentationError::PeerCannotReceive { .. }
                    | SegmentationError::PeerSegmentLimit { .. }
                    | SegmentationError::TooManySegments { .. },
                ),
            ) => return Self::NotSent(error.to_string()),
            Err(error) => error,
        };
        match error.remote() {
            Some(RemoteError::Error {
                class: ErrorClass::SERVICES,
                code: ErrorCode::SERVICE_REQUEST_DENIED,
            }) => Self::Denied,
            Some(RemoteError::Abort {
                reason: AbortReason::TSM_TIMEOUT,
            }) => Self::Timeout,
            Some(remote) => Self::Refused(remote),
            None => Self::Failed(error.to_string()),
        }
    }

    /// Whether the change took effect: `None` when that can't be known.
    pub fn applied(&self) -> Option<bool> {
        match self {
            Self::Ack => Some(true),
            Self::Denied | Self::Refused(_) | Self::NotSent(_) => Some(false),
            Self::Timeout | Self::Failed(_) => None,
        }
    }

    /// One line for the result view: names, or numbers for values without
    /// one.
    pub fn describe(&self) -> String {
        match self {
            Self::Ack => "acknowledged".into(),
            Self::Denied => "DENIED by the device's policy (SERVICE_REQUEST_DENIED)".into(),
            Self::Refused(RemoteError::Error { class, code }) => format!("error {class} / {code}"),
            Self::Refused(RemoteError::Reject { reason }) => format!("rejected, reason {reason}"),
            Self::Refused(RemoteError::Abort { reason }) => format!("aborted, reason {reason}"),
            Self::Timeout => "no answer: it may or may not have been applied".into(),
            Self::NotSent(why) => format!("not sent: {why}"),
            Self::Failed(why) => format!("failed: {why}"),
        }
    }

    /// The audit record's `outcome`. A class, code or reason is always a
    /// string: its name, or its decimal number when it has none.
    pub fn json(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::Ack => json!({ "result": "ack" }),
            Self::Denied => json!({ "result": "error", "class": "SERVICES",
                                    "code": "SERVICE_REQUEST_DENIED", "denied": true }),
            Self::Refused(RemoteError::Error { class, code }) => json!({ "result": "error",
                "class": class.to_string(), "code": code.to_string() }),
            Self::Refused(RemoteError::Reject { reason }) => json!({ "result": "reject",
                "reason": reason.to_string() }),
            Self::Refused(RemoteError::Abort { reason }) => json!({ "result": "abort",
                "reason": reason.to_string() }),
            Self::Timeout => json!({ "result": "timeout" }),
            Self::NotSent(why) => json!({ "result": "not-sent", "why": why }),
            Self::Failed(why) => json!({ "result": "failed", "why": why }),
        }
    }
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

    /// A write the node's DeviceCommunicationControl stopped before it went
    /// out was not applied; one whose retry it held back may have been
    /// (#2617).
    #[test]
    fn a_write_a_disabled_node_stopped() {
        let refused = Outcome::of(&Err(Error::InitiationDisabled { sent: false }));
        assert!(matches!(refused, Outcome::NotSent(_)), "{refused:?}");
        assert_eq!(refused.applied(), Some(false));
        let shown = refused.describe();
        assert_eq!(shown.matches("not sent").count(), 1, "{shown}");
        let withheld = Outcome::of(&Err(Error::InitiationDisabled { sent: true }));
        assert!(matches!(withheld, Outcome::Failed(_)), "{withheld:?}");
        assert_eq!(withheld.applied(), None);
    }

    /// A write the client's stop cut off while it waited for its turn was
    /// not applied; one it cut off after it may have gone out may have been
    /// (#1833).
    #[test]
    fn a_write_the_clients_stop_cut_off() {
        let unsent = Outcome::of(&Err(Error::RequestCutOff { sent: false }));
        assert!(matches!(unsent, Outcome::NotSent(_)), "{unsent:?}");
        assert_eq!(unsent.applied(), Some(false));
        let sent = Outcome::of(&Err(Error::RequestCutOff { sent: true }));
        assert!(matches!(sent, Outcome::Failed(_)), "{sent:?}");
        assert_eq!(sent.applied(), None);
    }

    /// A write its link refused for its state (stopped, say) never went
    /// out, so it was not applied: one already sent when the link stopped
    /// is a cut-off instead (#1833).
    #[test]
    fn a_write_its_link_refused_for_its_state_was_not_sent() {
        for refused in [
            bacnet_transport::port::stopped(),
            bacnet_transport::port::stopped_for_good(),
        ] {
            let outcome = Outcome::of(&Err(refused));
            assert!(matches!(outcome, Outcome::NotSent(_)), "{outcome:?}");
            assert_eq!(outcome.applied(), Some(false));
        }
    }

    /// A write too long for one APDU that the client refused to segment,
    /// or that the device can't take in segments, never went out, so it
    /// was not applied (#2617). One that failed partway through its
    /// segments may have been.
    #[test]
    fn a_write_refused_for_segmentation_was_not_sent() {
        use bacnet_types::enums::Segmentation;
        for refusal in [
            SegmentationError::LocalCannotTransmit,
            SegmentationError::PeerCannotReceive {
                capability: Segmentation::NO_SEGMENTATION,
            },
            SegmentationError::PeerSegmentLimit {
                segments: 9,
                max: 4,
            },
            SegmentationError::TooManySegments { segments: 300 },
        ] {
            let outcome = Outcome::of(&Err(Error::Segmentation(refusal)));
            assert!(matches!(outcome, Outcome::NotSent(_)), "{outcome:?}");
            assert_eq!(outcome.applied(), Some(false));
        }
        let partway = Outcome::of(&Err(Error::Segmentation(
            SegmentationError::NegativeAckRetriesExhausted,
        )));
        assert!(matches!(partway, Outcome::Failed(_)), "{partway:?}");
        assert_eq!(partway.applied(), None);
    }
}

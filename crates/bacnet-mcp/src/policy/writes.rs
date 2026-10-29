//! What the server may change (#2068; design §7.2, §7.3): whether the
//! write tools are offered at all, who may approve a write, and the
//! workspace whose ledger and audit log every change goes through.
//!
//! The default offers no write tool. A profile opts in with
//! `mcp_writes = "elicit"`: the client's own confirmation (an elicitation
//! form) may then approve T1 writes, never T2 or T3, because the server
//! can't check that a person answered it. `mcp_arm = "elicit"` lets that
//! form arm a time-boxed write session too. A [`PlanGrant`], approved by a
//! person in a terminal, approves the writes it names without a prompt.

use std::path::PathBuf;

use bacnet_types::enums::PropertyIdentifier;

/// Whether the write tools are offered (`mcp_writes`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WriteMode {
    /// No write tool is listed: the default.
    #[default]
    Never,
    /// `bacnet_write` and `bacnet_revert` are listed; a client's
    /// confirmation may approve T1 writes.
    Elicit,
}

/// How a write session may be armed (`mcp_arm`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ArmMode {
    /// Not by the client: only a plan grant's window counts as armed.
    #[default]
    Never,
    /// The confirmation form of a write while unarmed arms a session too.
    Elicit,
}

/// Where changes are recorded: the profile's workspace, which holds the
/// ledger and the audit log the TUI shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The workspace directory.
    pub dir: PathBuf,
    /// The profile's name, for audit records.
    pub profile: Option<String>,
    /// `BIP`, `BIP6`, `SC` or `MSTP`, for audit records.
    pub transport: &'static str,
    /// The local address, for audit records.
    pub local: String,
}

/// A commissioning plan a person approved in a terminal (`bacnet mcp
/// authorize`): the writes it names run without a prompt, up to its
/// budget and until it expires (8 hours at most).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanGrant {
    /// The device instances it covers, as inclusive ranges.
    pub devices: Vec<(u32, u32)>,
    /// The properties it may write.
    pub properties: Vec<PropertyIdentifier>,
    /// The lowest and highest priority it may command, inclusive; 6 is
    /// refused whatever the range (Clause 19.2.3).
    pub priorities: (u8, u8),
    /// How many writes it allows in this process.
    pub max_writes: u32,
    /// When it ends, in seconds since the Unix epoch.
    pub expires_unix: u64,
    /// Why it was granted, for the audit log.
    pub reason: Option<String>,
}

/// The longest a plan grant may last, in seconds.
pub const MAX_PLAN_SECS: u64 = 8 * 3_600;

/// The most writes a plan grant may allow.
pub const MAX_PLAN_WRITES: u32 = 10_000;

/// The most writes one `bacnet_write` call may make.
pub const MAX_WRITES: usize = 20;

impl PlanGrant {
    /// Whether it covers device `device`.
    pub fn covers_device(&self, device: u32) -> bool {
        self.devices
            .iter()
            .any(|&(low, high)| (low..=high).contains(&device))
    }

    /// Whether it covers a write of `property` at `priority` (none for a
    /// property that isn't commandable) on `device`.
    pub fn covers(&self, device: u32, property: PropertyIdentifier, priority: Option<u8>) -> bool {
        let (low, high) = self.priorities;
        self.covers_device(device)
            && self.properties.contains(&property)
            && priority.is_none_or(|p| p != 6 && (low..=high).contains(&p))
    }

    fn validate(&self) -> Result<(), String> {
        if self.devices.is_empty() || self.properties.is_empty() {
            return Err("plan: name its devices and properties".into());
        }
        for &(low, high) in &self.devices {
            if low > high || high > bacnet_types::primitives::ObjectIdentifier::MAX_INSTANCE {
                return Err(format!("plan: devices {low}-{high} is not a range"));
            }
        }
        let (low, high) = self.priorities;
        if low == 0 || low > high || high > 16 {
            return Err(format!("plan: priorities {low}-{high} is not within 1-16"));
        }
        if !(1..=MAX_PLAN_WRITES).contains(&self.max_writes) {
            return Err(format!("plan: max_writes must be 1 to {MAX_PLAN_WRITES}"));
        }
        Ok(())
    }
}

/// What the server may change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WritePolicy {
    /// Whether the write tools are offered.
    pub mode: WriteMode,
    /// How a write session may be armed.
    pub arm: ArmMode,
    /// A plan grant, if one was approved.
    pub plan: Option<PlanGrant>,
    /// The profile's workspace; writes need one.
    pub workspace: Option<Workspace>,
}

impl WritePolicy {
    /// Whether the write tools are offered.
    pub fn offered(&self) -> bool {
        self.mode == WriteMode::Elicit
    }

    /// Check it before anything starts.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.plan.is_some() && !self.offered() {
            return Err("a plan grant needs mcp_writes = \"elicit\"".into());
        }
        if self.offered() && self.workspace.is_none() {
            return Err("writes need a profile workspace for the ledger and the audit log".into());
        }
        if self.arm == ArmMode::Elicit && !self.offered() {
            return Err("mcp_arm = \"elicit\" needs mcp_writes = \"elicit\"".into());
        }
        self.plan.as_ref().map_or(Ok(()), PlanGrant::validate)
    }
}

//! Writes over MCP (#2068; design §7): the server's write session, who may
//! approve a write ([`Approver`]), the sealed confirmation state
//! ([`seal`]), and the MCP front end as a caller of the shared write desk
//! (`bacnet_ops::safety::desk`), which owns the ledger and the audit log.
//!
//! The tools themselves are `tools/write.rs` and `tools/revert.rs`.

pub(crate) mod budget;
pub(crate) mod notify;
pub(crate) mod seal;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bacnet_ops::safety::desk::Caller;
use bacnet_ops::safety::{DEFAULT_MINUTES, Level, Preview, Tier, hex};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::time::Instant;

use self::notify::Signals;
use self::seal::Sealer;
use crate::policy::{ArmMode, PlanGrant, WritePolicy};

/// The server's write state: its armed session, the plan grant's budget
/// spent, the confirmation sealer, and the signals that tell subscribers
/// of `bacnet://session` and `bacnet://ledger` they changed.
pub(crate) struct Writes {
    pub(crate) sealer: Sealer,
    armed: Mutex<Option<Armed>>,
    /// The plan grant's writes as last read from or saved to the
    /// workspace's count ([`budget`]), which is what a write is checked
    /// against; never lowered while the server runs.
    plan_used: AtomicU32,
    /// Why the plan grant's count couldn't be read, when it last
    /// couldn't: the plan's writes are refused meanwhile, and the session
    /// says so.
    count_error: Mutex<Option<String>>,
    pub(crate) signals: Signals,
    /// Set while the server waits for an older client to confirm a write
    /// (#2158): one such request at a time.
    asking: AtomicBool,
    /// Fail the next count saves, for tests.
    #[cfg(test)]
    pub(crate) budget_fault: AtomicBool,
    /// Seconds added to the wall clock, for tests.
    #[cfg(test)]
    pub(crate) wall_offset: std::sync::atomic::AtomicU64,
}

/// The one confirmation an older client is being asked for, held until it
/// is answered or given up.
pub(crate) struct Asking<'a>(&'a AtomicBool);

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// An armed write session: until when, and the reason given.
#[derive(Debug, Clone)]
struct Armed {
    until: Instant,
    reason: Option<String>,
}

impl Writes {
    /// The write state under `policy`, with a plan grant's count as the
    /// workspace keeps it ([`budget::load`]).
    pub(crate) fn new(policy: &WritePolicy) -> Self {
        let (used, count_error) = budget::load(policy);
        Self {
            sealer: Sealer::new(),
            armed: Mutex::new(None),
            plan_used: AtomicU32::new(used),
            count_error: Mutex::new(count_error),
            signals: Signals::default(),
            asking: AtomicBool::new(false),
            #[cfg(test)]
            budget_fault: AtomicBool::new(false),
            #[cfg(test)]
            wall_offset: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Seconds since the Unix epoch, on the clock a plan grant's window is
    /// checked against.
    pub(crate) fn wall_now(&self) -> u64 {
        #[cfg(test)]
        return unix_now().saturating_add(self.wall_offset.load(Ordering::Relaxed));
        #[cfg(not(test))]
        unix_now()
    }

    /// Claim the one confirmation request an older client may have
    /// waiting, unless another holds it.
    pub(crate) fn ask(&self) -> Option<Asking<'_>> {
        self.asking
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then(|| Asking(&self.asking))
    }

    fn armed(&self) -> Option<Armed> {
        self.armed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether a write session is armed now.
    pub(crate) fn is_armed(&self) -> bool {
        self.armed().is_some_and(|a| Instant::now() < a.until)
    }

    /// Seconds left in the armed session, if one is armed.
    pub(crate) fn armed_for(&self) -> Option<u64> {
        self.armed()
            .and_then(|a| a.until.checked_duration_since(Instant::now()))
            .map(|left| left.as_secs())
    }

    /// When the armed session ends, if one is armed now.
    pub(crate) fn armed_until(&self) -> Option<Instant> {
        self.armed()
            .map(|a| a.until)
            .filter(|until| Instant::now() < *until)
    }

    /// Arm a write session for `minutes`.
    pub(crate) fn arm(&self, minutes: u32, reason: Option<String>) {
        let until = Instant::now() + Duration::from_secs(u64::from(minutes) * 60);
        *self.armed.lock().unwrap_or_else(PoisonError::into_inner) = Some(Armed { until, reason });
        self.signals.session_changed();
    }

    /// The armed session's reason, for audit records.
    pub(crate) fn reason(&self) -> Option<String> {
        self.armed().and_then(|a| a.reason)
    }

    /// The plan grant, while it is in force: not expired, budget left (as
    /// last counted; [`spend`](Self::spend) reads the count again).
    /// Checked on every write: a clock set back past the plan's start
    /// can't stretch it beyond its 8 hours either.
    pub(crate) fn plan<'a>(&self, policy: &'a WritePolicy) -> Option<&'a PlanGrant> {
        let now = self.wall_now();
        policy.plan.as_ref().filter(|plan| {
            in_window(plan, now) && self.plan_used.load(Ordering::Relaxed) < plan.max_writes
        })
    }

    /// The level a write runs at: armed while a session or a plan grant
    /// is in force (never `dangerous` over MCP), expired once a session
    /// has run out, read-only otherwise.
    pub(crate) fn level(&self, policy: &WritePolicy) -> Level {
        if self.is_armed() || self.plan(policy).is_some() {
            Level::Armed { dangerous: false }
        } else if self.armed().is_some() {
            Level::Expired
        } else {
            Level::ReadOnly
        }
    }
}

/// Whether `plan`'s window is open at `now` (seconds since the Unix
/// epoch): before its end, and no more than 8 hours before it, so a clock
/// set back can't stretch it either.
pub(crate) fn in_window(plan: &PlanGrant, now: u64) -> bool {
    now < plan.expires_unix && plan.expires_unix <= now.saturating_add(crate::policy::MAX_PLAN_SECS)
}

/// Seconds since the Unix epoch.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// How a write was approved, as the audit log and the result name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Via {
    /// The client's elicitation form: the client's claim that a person
    /// confirmed, so T1 only.
    Elicitation,
    /// The same claim, answered to an elicitation request the server sent
    /// a client older than 2026-07-28 (#2158): T1 only as well.
    ServerElicitation,
    /// A plan grant a person approved in a terminal.
    Plan,
}

impl Via {
    /// `confirmed_via`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Elicitation => "elicitation",
            Self::ServerElicitation => "server-elicitation",
            Self::Plan => "plan",
        }
    }

    /// `approved_by`: who stands behind the approval.
    pub(crate) fn by(self) -> &'static str {
        match self {
            Self::Elicitation | Self::ServerElicitation => "client",
            Self::Plan => "plan",
        }
    }
}

/// An approver's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Approval {
    /// Send it.
    Approved(Via),
    /// Ask the client to confirm (an elicitation form), arming a session
    /// for these minutes as well if it isn't armed.
    Ask { arm: Option<u32> },
    /// Refused: nothing is sent.
    Refused {
        reason: &'static str,
        message: String,
    },
}

fn refused(reason: &'static str, message: impl Into<String>) -> Approval {
    Approval::Refused {
        reason,
        message: message.into(),
    }
}

/// Who may approve a change (design §7.3). An approver only decides; the
/// desk checks the level and the preview again before anything is sent.
pub(crate) trait Approver: Send + Sync {
    /// Whether `principal` may make `preview`, whose tier is `tier`.
    fn approve(&self, principal: &str, preview: &Preview, tier: Tier) -> Approval;
}

/// Approves nothing: the default, when no other channel is configured.
pub(crate) struct Deny;

impl Approver for Deny {
    fn approve(&self, _: &str, _: &Preview, _: Tier) -> Approval {
        refused("write-not-granted", "this server approves no writes")
    }
}

/// The client's own confirmation, through a form elicitation. The server
/// can't check that a person answered it, so it approves T1 only (the
/// maintainer's decision of 2026-10-25); T2 and T3 wait for a verifiable
/// approver.
pub(crate) struct ClientElicitation {
    /// How a session may be armed.
    pub(crate) arm: ArmMode,
    /// Whether a session is armed now.
    pub(crate) armed: bool,
    /// Whether the client can be asked: it takes form elicitations over
    /// multi round-trip requests (a 2026-07-28 client that declares the
    /// capability), or is an older client that takes the server's
    /// elicitation requests.
    pub(crate) supported: bool,
}

impl Approver for ClientElicitation {
    fn approve(&self, _: &str, _: &Preview, tier: Tier) -> Approval {
        if tier != Tier::T1 {
            return refused(
                "needs-verified-approver",
                format!(
                    "a {tier:?} change needs an approver the server can verify (the TUI, in a \
                     later release); a client's confirmation approves T1 only"
                ),
            );
        }
        let arm = match (self.armed, self.arm) {
            (true, _) => None,
            (false, ArmMode::Elicit) => Some(DEFAULT_MINUTES),
            (false, ArmMode::Never) => {
                return refused(
                    "not-armed",
                    "no write session is armed, and this profile sets mcp_arm = \"never\": only \
                     a plan grant arms MCP writes on it",
                );
            }
        };
        if !self.supported {
            return refused(
                "elicitation-unsupported",
                "confirming a write needs a client that takes form elicitations (over multi \
                 round-trip requests on 2026-07-28, or as the server's requests before it)",
            );
        }
        Approval::Ask { arm }
    }
}

/// A plan grant's approver: the writes it names, within its budget,
/// counted in `workspace` (whose lock the caller holds) before they go.
pub(crate) struct PlanApprover<'a> {
    pub(crate) plan: &'a PlanGrant,
    pub(crate) writes: &'a Writes,
    pub(crate) workspace: &'a Path,
}

impl Approver for PlanApprover<'_> {
    fn approve(&self, _: &str, preview: &Preview, _: Tier) -> Approval {
        let outside = preview.items.iter().find(|item| {
            !self
                .plan
                .covers(item.spec.device, item.spec.property, item.priority)
        });
        if let Some(item) = outside {
            return refused(
                "outside-plan",
                format!("{} is outside the plan grant", item.label()),
            );
        }
        let n = u32::try_from(preview.items.len()).unwrap_or(u32::MAX);
        match self.writes.spend(self.plan, n, self.workspace) {
            Ok(()) => Approval::Approved(Via::Plan),
            Err(refusal) => refusal,
        }
    }
}

/// A digest of what a person confirmed: each write's target, value,
/// pre-image, slot holder, commandability and tier.
pub(crate) fn preview_digest(preview: &Preview) -> String {
    let items: Vec<Value> = preview
        .items
        .iter()
        .map(|item| {
            json!([
                item.label(),
                hex(&item.value.bytes),
                item.before.as_ref().map(|b| hex(&b.bytes)),
                format!("{:?}", item.occupancy),
                item.commandable,
                item.gate.name(),
            ])
        })
        .collect();
    let canonical = json!([preview.device, preview.batch, preview.gate.name(), items]);
    hex(&Sha256::digest(canonical.to_string().as_bytes()))
}

/// The MCP front end as the desk's caller: its level, the fields its
/// records carry, and the approval that must still hold when the request
/// goes.
pub(crate) struct McpCaller<'a> {
    pub(crate) writes: &'a Writes,
    pub(crate) policy: &'a WritePolicy,
    /// How the write was approved; none for a refusal or a revert.
    pub(crate) via: Option<Via>,
    pub(crate) fields: Map<String, Value>,
    /// Where the ids of the entries this write records as `sending` go,
    /// so a call that stops waiting can still name them.
    pub(crate) sending: Option<Sending>,
    /// Set once this caller's `sending` rows were announced.
    announced: AtomicBool,
}

/// The ledger ids a handed-off write has recorded, shared with its call.
pub(crate) type Sending = std::sync::Arc<Mutex<Vec<u64>>>;

impl McpCaller<'_> {
    /// A caller for `writes` under `policy`, approved `via`.
    pub(crate) fn new<'a>(
        writes: &'a Writes,
        policy: &'a WritePolicy,
        via: Option<Via>,
        fields: Map<String, Value>,
    ) -> McpCaller<'a> {
        McpCaller {
            writes,
            policy,
            via,
            fields,
            sending: None,
            announced: AtomicBool::new(false),
        }
    }

    /// Whether this caller's write reached the ledger as `sending` (and
    /// was announced), so its settle is announced whatever it leaves.
    pub(crate) fn announced(&self) -> bool {
        self.announced.load(Ordering::Acquire)
    }
}

impl Caller for McpCaller<'_> {
    fn level(&self) -> Level {
        self.writes.level(self.policy)
    }

    fn refusal(&self) -> Option<String> {
        match self.via {
            Some(Via::Elicitation | Via::ServerElicitation) if !self.writes.is_armed() => {
                Some("the write session expired before the write went".into())
            }
            Some(Via::Plan)
                if self
                    .policy
                    .plan
                    .as_ref()
                    .is_none_or(|p| self.writes.wall_now() >= p.expires_unix) =>
            {
                Some("the plan grant expired before the write went".into())
            }
            _ => None,
        }
    }

    fn ledger_changed<'a>(
        &'a self,
        entries: &'a [bacnet_ops::safety::ledger::Entry],
    ) -> bacnet_ops::safety::desk::Waiting<'a> {
        // The `sending` rows are in the file now: tell the ledger's
        // subscribers, and again once they settle, however they settle.
        self.announced.store(true, Ordering::Release);
        self.writes.signals.ledger_changed();
        if let Some(sending) = &self.sending {
            // Under the workspace's lock, this run's `sending` entries are
            // this write's.
            let ids = entries
                .iter()
                .filter(|e| {
                    e.state == bacnet_ops::safety::ledger::State::Sending
                        && e.run == bacnet_ops::safety::run_id()
                })
                .map(|e| e.id);
            *sending.lock().unwrap_or_else(PoisonError::into_inner) = ids.collect();
        }
        Box::pin(async {})
    }

    fn fields(&self) -> Map<String, Value> {
        let mut fields = self.fields.clone();
        if let Some(via) = self.via {
            fields.insert("confirmed_via".into(), json!(via.name()));
            fields.insert("approved_by".into(), json!(via.by()));
            if via == Via::Plan
                && let Some(plan) = &self.policy.plan
            {
                fields.insert(
                    "plan".into(),
                    json!({ "reason": plan.reason, "used": self.writes.plan_used(),
                            "max_writes": plan.max_writes }),
                );
            }
        }
        if let Some(reason) = self.writes.reason() {
            fields
                .entry("session_reason")
                .or_insert_with(|| json!(reason));
        }
        fields
    }
}

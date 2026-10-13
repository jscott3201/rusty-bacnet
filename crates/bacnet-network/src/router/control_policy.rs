//! RB-09 routing-control admission policy (wire ingress only).
//!
//! Single admission boundary for state-changing routing controls. Direct
//! [`crate::router_table::RouterTable`] API calls bypass this policy by
//! construction (LOCAL); all wire ingress including loopback stays untrusted.
//! APDUs never reach this boundary; directed-forward paths mutate nothing.
//!
//! Protected (hardened denies unknown authority): Init updates/purges, I-Am
//! learning, Init-ACK learning, Busy/Available marks, Reject transitions,
//! I-Could-Be inserts. Read-only (always allowed): Who-Is relay/answer,
//! Init queries, What-Is/Number-Is, Establish/Disconnect no-ops, security
//! ack-only, proprietary explicit-reject, directed opaque forwarding.
//! Deny is a silent local drop: no mutation, relay, ACK, or Reject emission.
//! Validation precedes policy. The router's engine asks for each decision
//! under its lock, with the message's claims, before anything changes.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_transport::port::TransportProvenance;
use tracing::debug;

/// Wire-control authorization mode. Permissive preserves today's behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlPolicy {
    /// Wire controls are admitted unless an installed authorizer says otherwise.
    #[default]
    Permissive,
    /// Protected controls are denied unless an installed authorizer approves them.
    Hardened,
}

/// Channel/relay scope only. Never leaf identity: a verified SC ingress
/// asserts the channel/relay validation, not that a claimed SNET/SADR leaf
/// is the authenticated peer. Provenance alone never authorizes; only the
/// callback does (callback-only, no static allowlist).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlTrust {
    /// Ingress arrived on a path with no channel or relay verification.
    Unverified,
    /// Ingress arrived over a verified direct-peer or hub channel.
    VerifiedChannel,
    /// Ingress was relayed from a verified origin behind another node.
    VerifiedRelay,
}

impl ControlTrust {
    /// Map transport provenance to a trust scope: direct-peer or hub channels are
    /// `VerifiedChannel`, relayed origins are `VerifiedRelay`, everything else is `Unverified`.
    pub fn from_provenance(p: TransportProvenance) -> Self {
        if p.is_direct_peer() || p.is_hub_channel() {
            Self::VerifiedChannel
        } else if p.is_relayed_origin() {
            Self::VerifiedRelay
        } else {
            Self::Unverified
        }
    }
}

/// Immutable admission facts for one protected control. Claims only.
#[derive(Clone, PartialEq, Eq)]
pub struct ControlAuthContext {
    /// Index of the router port the control arrived on.
    pub port_idx: usize,
    /// Network number of that port.
    pub port_network: u16,
    /// Data-link source address of the sending node.
    pub source_mac: bacnet_types::MacAddr,
    /// How the transport learned of the frame's origin (direct peer, hub, relayed).
    pub provenance: TransportProvenance,
    /// Channel-level trust derived from `provenance`; says nothing about the leaf sender's
    /// identity.
    pub trust: ControlTrust,
    /// Network-layer message type code of the control, as received.
    pub message_type: u8,
    /// Vendor ID from a proprietary network message, if present.
    pub vendor_id: Option<u16>,
    /// Source network number claimed in the NPDU, if any.
    pub source_net: Option<u16>,
    /// Destination network number claimed in the NPDU, if any.
    pub dest_net: Option<u16>,
    /// Network numbers the control refers to, after validation.
    pub target_nets: Vec<u16>,
    /// True when the control omitted its network list, meaning every network served via the sender.
    pub targets_omitted: bool,
}

impl std::fmt::Debug for ControlAuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlAuthContext")
            .field("port_idx", &self.port_idx)
            .field("port_network", &self.port_network)
            .field("source_mac_len", &self.source_mac.len())
            .field("provenance", &self.provenance)
            .field("trust", &self.trust)
            .field("message_type", &self.message_type)
            .field("vendor_id", &self.vendor_id)
            .field("source_net", &self.source_net)
            .field("dest_net", &self.dest_net)
            .field("target_nets", &self.target_nets)
            .field("targets_omitted", &self.targets_omitted)
            .finish_non_exhaustive()
    }
}

/// Fast, nonblocking, side-effect-free control authorizer.
///
/// Consulted for protected controls only; read-only discovery never calls it.
/// Do not block, reenter the router/table, or perform side effects. It runs
/// under the router's engine lock (#1679), one call at a time per router:
/// every port's routing waits while it runs, and calling into the router
/// from it ([`BACnetRouter::table`](super::BACnetRouter::table) included)
/// deadlocks. Returning `false` or panicking denies fail-closed (silent
/// drop, no mutation/relay/ACK/Reject).
pub type ControlAuthorizer = Arc<dyn Fn(&ControlAuthContext) -> bool + Send + Sync>;

/// Protected control class, `bacnet-router-core`'s. One counter row each.
pub use bacnet_router_core::router::ControlClass;

/// The counter row of `class`; `None` for a class a later engine adds,
/// which has no row yet and isn't counted.
fn index(class: ControlClass) -> Option<usize> {
    match class {
        ControlClass::IAm => Some(0),
        ControlClass::InitAck => Some(1),
        ControlClass::InitMgmt => Some(2),
        ControlClass::Busy => Some(3),
        ControlClass::Available => Some(4),
        ControlClass::Reject => Some(5),
        ControlClass::ICouldBe => Some(6),
        _ => None,
    }
}

/// Saturating per-class decision totals. Count-only; never affects policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlServiceCounters {
    /// Controls of this class that were admitted.
    pub allow_total: u64,
    /// Controls of this class that were refused for any reason (includes `policy_deny_total`).
    pub deny_total: u64,
    /// Refusals caused by hardened policy with no authorizer installed.
    pub policy_deny_total: u64,
}

/// Fixed-shape telemetry for the seven protected classes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlDecisionCounters {
    /// Totals for I-Am-Router-To-Network.
    pub i_am: ControlServiceCounters,
    /// Totals for Initialize-Routing-Table-Ack.
    pub init_ack: ControlServiceCounters,
    /// Totals for Initialize-Routing-Table updates and purges.
    pub init_mgmt: ControlServiceCounters,
    /// Totals for Router-Busy-To-Network.
    pub busy: ControlServiceCounters,
    /// Totals for Router-Available-To-Network.
    pub available: ControlServiceCounters,
    /// Totals for Reject-Message-To-Network.
    pub reject: ControlServiceCounters,
    /// Totals for I-Could-Be-Router-To-Network.
    pub i_could_be: ControlServiceCounters,
}

#[derive(Clone, Copy)]
enum Decision {
    Allow,
    Deny,
    PolicyDeny,
}

/// Atomic per-class totals shared via [`ControlGate`].
#[derive(Default)]
pub struct ControlDecisions([[AtomicU64; 3]; 7]);

impl std::fmt::Debug for ControlDecisions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlDecisions")
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl ControlDecisions {
    /// Copy the current totals; each counter is read independently, so the set is not atomic.
    pub fn snapshot(&self) -> ControlDecisionCounters {
        let rows = self.0.each_ref().map(|r| {
            let [a, d, p] = r.each_ref().map(|n| n.load(Ordering::Relaxed));
            ControlServiceCounters {
                allow_total: a,
                deny_total: d,
                policy_deny_total: p,
            }
        });
        let [i_am, init_ack, init_mgmt, busy, available, reject, i_could_be] = rows;
        ControlDecisionCounters {
            i_am,
            init_ack,
            init_mgmt,
            busy,
            available,
            reject,
            i_could_be,
        }
    }

    fn record(&self, class: ControlClass, decision: Decision) {
        let Some(row) = index(class) else {
            debug!(
                ?class,
                "No decision counters for this control class; not counted"
            );
            return;
        };
        let inc = |col: usize| {
            #[allow(deprecated, reason = "try_update needs Rust 1.95; the MSRV is 1.93")]
            let _ = self.0[row][col].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
        };
        match decision {
            Decision::Allow => inc(0),
            Decision::Deny => inc(1),
            Decision::PolicyDeny => {
                inc(1);
                inc(2);
            }
        }
    }
}

/// Policy + optional authorizer + shared counters for wire controls.
pub struct ControlGate {
    policy: ControlPolicy,
    authorizer: Option<ControlAuthorizer>,
    decisions: ControlDecisions,
}

impl std::fmt::Debug for ControlGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlGate")
            .field("policy", &self.policy)
            .field(
                "authorizer",
                &self.authorizer.as_ref().map(|_| "<callback>"),
            )
            .field("decisions", &self.decisions)
            .finish_non_exhaustive()
    }
}

impl Default for ControlGate {
    fn default() -> Self {
        Self::permissive()
    }
}

impl ControlGate {
    /// Gate that admits every protected control (no authorizer).
    pub fn permissive() -> Self {
        Self {
            policy: ControlPolicy::Permissive,
            authorizer: None,
            decisions: ControlDecisions::default(),
        }
    }

    /// Gate that denies every protected control (no authorizer).
    pub fn hardened() -> Self {
        Self {
            policy: ControlPolicy::Hardened,
            authorizer: None,
            decisions: ControlDecisions::default(),
        }
    }

    /// Gate with an explicit policy and optional authorizer; a present authorizer decides every
    /// protected control regardless of `policy`.
    pub fn new(policy: ControlPolicy, authorizer: Option<ControlAuthorizer>) -> Self {
        Self {
            policy,
            authorizer,
            decisions: ControlDecisions::default(),
        }
    }

    /// Configured policy mode.
    pub fn policy(&self) -> ControlPolicy {
        self.policy
    }

    /// Snapshot of the allow/deny totals recorded by this gate.
    pub fn snapshot(&self) -> ControlDecisionCounters {
        self.decisions.snapshot()
    }

    /// Decide `class` for the redacted context `ctx`, counting the
    /// decision. The engine's host calls it after validation, before any
    /// change.
    pub(super) fn decide(&self, ctx: &ControlAuthContext, class: ControlClass) -> bool {
        match (&self.policy, &self.authorizer) {
            (ControlPolicy::Permissive, None) => {
                self.decisions.record(class, Decision::Allow);
                true
            }
            (ControlPolicy::Hardened, None) => {
                self.decisions.record(class, Decision::PolicyDeny);
                false
            }
            (_, Some(auth)) => {
                let allowed =
                    std::panic::catch_unwind(AssertUnwindSafe(|| auth(ctx))).unwrap_or(false);
                self.decisions.record(
                    class,
                    if allowed {
                        Decision::Allow
                    } else {
                        Decision::Deny
                    },
                );
                allowed
            }
        }
    }
}

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
//! Validation precedes policy; policy runs lock-free on immutable data.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_transport::port::TransportProvenance;

/// Wire-control authorization mode. Permissive preserves today's behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlPolicy {
    #[default]
    Permissive,
    Hardened,
}

/// Channel/relay scope only. Never leaf identity: a verified SC ingress
/// asserts the channel/relay validation, not that a claimed SNET/SADR leaf
/// is the authenticated peer. Provenance alone never authorizes; only the
/// callback does (callback-only, no static allowlist).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlTrust {
    Unverified,
    VerifiedChannel,
    VerifiedRelay,
}

impl ControlTrust {
    pub fn from_provenance(p: TransportProvenance) -> Self {
        if p.is_direct_peer() {
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
    pub port_idx: usize,
    pub port_network: u16,
    pub source_mac: bacnet_types::MacAddr,
    pub provenance: TransportProvenance,
    pub trust: ControlTrust,
    pub message_type: u8,
    pub vendor_id: Option<u16>,
    pub source_net: Option<u16>,
    pub dest_net: Option<u16>,
    pub target_nets: Vec<u16>,
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
/// Do not block, reenter the router/table, or perform side effects; callbacks
/// may run concurrently while no table lock is held. Returning `false` or
/// panicking denies fail-closed (silent drop, no mutation/relay/ACK/Reject).
pub type ControlAuthorizer = Arc<dyn Fn(&ControlAuthContext) -> bool + Send + Sync>;

/// Protected control class. One counter row each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlClass {
    IAm,
    InitAck,
    InitMgmt,
    Busy,
    Available,
    Reject,
    ICouldBe,
}

impl ControlClass {
    fn index(self) -> usize {
        match self {
            Self::IAm => 0,
            Self::InitAck => 1,
            Self::InitMgmt => 2,
            Self::Busy => 3,
            Self::Available => 4,
            Self::Reject => 5,
            Self::ICouldBe => 6,
        }
    }
}

/// Saturating per-class decision totals. Count-only; never affects policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlServiceCounters {
    pub allow_total: u64,
    pub deny_total: u64,
    pub policy_deny_total: u64,
}

/// Fixed-shape telemetry for the seven protected classes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlDecisionCounters {
    pub i_am: ControlServiceCounters,
    pub init_ack: ControlServiceCounters,
    pub init_mgmt: ControlServiceCounters,
    pub busy: ControlServiceCounters,
    pub available: ControlServiceCounters,
    pub reject: ControlServiceCounters,
    pub i_could_be: ControlServiceCounters,
}

#[derive(Clone, Copy)]
enum Decision {
    Allow,
    Deny,
    PolicyDeny,
}

/// Atomic per-class totals shared via [`ControlGate`].
pub struct ControlDecisions([[AtomicU64; 3]; 7]);

impl Default for ControlDecisions {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl std::fmt::Debug for ControlDecisions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlDecisions")
            .field("snapshot", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl ControlDecisions {
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
        let inc = |col: usize| {
            let _ = self.0[class.index()][col].fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |n| Some(n.saturating_add(1)),
            );
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
    pub fn permissive() -> Self {
        Self {
            policy: ControlPolicy::Permissive,
            authorizer: None,
            decisions: ControlDecisions::default(),
        }
    }

    pub fn hardened() -> Self {
        Self {
            policy: ControlPolicy::Hardened,
            authorizer: None,
            decisions: ControlDecisions::default(),
        }
    }

    pub fn new(policy: ControlPolicy, authorizer: Option<ControlAuthorizer>) -> Self {
        Self {
            policy,
            authorizer,
            decisions: ControlDecisions::default(),
        }
    }

    pub fn policy(&self) -> ControlPolicy {
        self.policy
    }

    pub fn snapshot(&self) -> ControlDecisionCounters {
        self.decisions.snapshot()
    }

    fn decide(&self, ctx: &ControlAuthContext, class: ControlClass) -> bool {
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

    /// Build redacted context from immutable ingress + validated targets and
    /// decide. Call after validation, before any table lock. `targets_omitted`
    /// marks Busy/Available omitted-list scope (all served via peer).
    pub fn authorize(
        &self,
        ingress: &super::IngressContext,
        class: ControlClass,
        targets: &[u16],
        targets_omitted: bool,
    ) -> bool {
        let ctx = ControlAuthContext {
            port_idx: ingress.port_idx,
            port_network: ingress.port_network,
            source_mac: ingress.source_mac.clone(),
            provenance: ingress.provenance,
            trust: ControlTrust::from_provenance(ingress.provenance),
            message_type: ingress.npdu.message_type.unwrap_or(0xFF),
            vendor_id: ingress.npdu.vendor_id,
            source_net: ingress.npdu.source.as_ref().map(|s| s.network),
            dest_net: ingress.npdu.destination.as_ref().map(|d| d.network),
            target_nets: targets.to_vec(),
            targets_omitted,
        };
        self.decide(&ctx, class)
    }
}

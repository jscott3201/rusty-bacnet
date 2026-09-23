//! Constrained hub admission and bounded status (RB-12).
//!
//! Three pieces, all bounded and redacted by construction:
//!
//! - [`ScHubAdmissionLimits`] — validated `{max_clients, max_handshakes}`
//!   bounds carried through [`super::ScHubTlsConfig`] like the broadcast
//!   policy. Every public startup validates them before binding; zero or
//!   overflowing bounds are configuration errors, never a way to disable
//!   limiting. Each hub started from a clone gets independent counters.
//! - Admin admission policy — a synchronous operator callback over
//!   [`ScHubAdmissionInput`] returning [`ScHubAdmissionDecision`]. It is
//!   evaluated under the registry lock, before `deadline.commit()`, so a
//!   deny cannot race replacement or insertion. A deny answers with the
//!   existing `RESOURCES`/`OTHER` NAK family (owner-approved; same wire
//!   signal as `NakMaxClients`), closes without map mutation, leaves any
//!   incumbent untouched (no `closed=true`, no wake), and bumps its own
//!   saturating deny counter, distinct from replacement.
//! - [`ScHubStatus`] — a bounded snapshot reusing existing lifecycle
//!   predicates. Counts and kind labels only: no certificates, keys, VMAC
//!   maps, or unbounded payloads. There is no event stream.
//!
//! Duplicate behavior follows ASHRAE Standard 135-2020, Annex AB, AB.6.2.3
//! and Fig. AB-12 (printed pp. 1403-1404; PDF pp. 1405-1406 of the licensed
//! file): a new Device UUID with no VMAC collision is accepted; a known
//! Device UUID is accepted while the existing connection to that UUID is
//! disconnected and closed; a new Device UUID colliding with the hub's own
//! VMAC or any existing peer VMAC gets a `COMMUNICATION` /
//! `NODE_DUPLICATE_VMAC` NAK and the WebSocket is closed. Same-UUID
//! replacement still wins over capacity, and different-UUID VMAC collisions
//! still NAK as duplicates. Admin Deny is a local extension and a new
//! outcome, distinct from Replace and NAK-duplicate.

use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_types::error::Error;

use crate::port::TransportProvenance;
use crate::sc_frame::Vmac;

/// Default cap on simultaneously established (registered) hub clients.
///
/// Replaces the previous hardcoded 256-client constant; the default value
/// is unchanged.
pub const DEFAULT_MAX_CLIENTS: usize = 256;

/// Default cap on in-progress (accepted but unregistered) handshakes.
///
/// Sized so the default total (`max_clients + max_handshakes = 512`)
/// preserves the previous hardcoded 512-connection total. Established
/// clients keep their slots: only unregistered connections compete for
/// this bound, so replacement and steady-state peers are unaffected by
/// handshake pressure.
pub const DEFAULT_MAX_HANDSHAKES: usize = 256;

/// Validated hub admission bounds.
///
/// Both bounds are nonzero counts, not rates. `max_clients` caps
/// simultaneously registered clients (enforced with a NAK at
/// Connect-Request registration, after same-UUID replacement). The total
/// accepted-connection cap is the sum `max_clients + max_handshakes`,
/// enforced by dropping the TCP connection at accept; the handshake bound
/// is enforced the same way against unregistered connections only.
/// Rejections at accept are silent TCP drops (existing behavior); only
/// registration-time outcomes produce wire NAKs.
///
/// Construct with [`Self::new`] for an early error, or set pub fields and
/// rely on startup validation: every public [`super::ScHub`] startup API
/// validates these bounds before binding. Clones share the bound values
/// but never the runtime counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubAdmissionLimits {
    /// Maximum simultaneously established (registered) clients.
    pub max_clients: usize,
    /// Maximum simultaneously unregistered (handshake) connections.
    pub max_handshakes: usize,
}

impl Default for ScHubAdmissionLimits {
    fn default() -> Self {
        Self {
            max_clients: DEFAULT_MAX_CLIENTS,
            max_handshakes: DEFAULT_MAX_HANDSHAKES,
        }
    }
}

impl ScHubAdmissionLimits {
    /// Construct checked admission bounds. Zero or overflowing (summing
    /// past the addressable count) bounds return [`Error::Encoding`]
    /// before any socket is opened.
    pub fn new(max_clients: usize, max_handshakes: usize) -> Result<Self, Error> {
        let limits = Self {
            max_clients,
            max_handshakes,
        };
        limits.validate()?;
        Ok(limits)
    }

    /// Total accepted-connection cap enforced at TCP accept.
    ///
    /// Saturating; validated instances never saturate because startup
    /// rejects overflowing bounds before binding.
    pub fn total_active(self) -> usize {
        self.max_clients.saturating_add(self.max_handshakes)
    }

    pub(super) fn validate(self) -> Result<(), Error> {
        if self.max_clients == 0 {
            return Err(Error::Encoding(
                "hub admission max_clients must be nonzero".into(),
            ));
        }
        if self.max_handshakes == 0 {
            return Err(Error::Encoding(
                "hub admission max_handshakes must be nonzero".into(),
            ));
        }
        if self.max_clients.checked_add(self.max_handshakes).is_none() {
            return Err(Error::Encoding(
                "hub admission limits overflow: max_clients + max_handshakes exceeds the addressable count"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Current registration relationship, captured under the registry lock.
///
/// Fixed labels only: no incumbent address, VMAC, UUID or certificate material.
/// This describes payload-claim equality, not an authenticated device principal.
/// Different-UUID VMAC conflict takes precedence even if the requested UUID is
/// registered at another VMAC. Capacity is independent: a new UUID remains
/// `Initial` when the client limit is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScHubRegistrationKind {
    /// No known UUID and no conflicting peer VMAC.
    Initial,
    /// The requested UUID already owns the requested VMAC.
    SameUuidSameVmac,
    /// The requested UUID owns another VMAC; the requested VMAC is free.
    SameUuidMovedVmac,
    /// The requested VMAC belongs to a different UUID.
    ConflictingVmac,
}

impl super::HubClientRegistrationDecision {
    pub(super) fn admission_kind(&self, requested_vmac: Vmac) -> ScHubRegistrationKind {
        match self {
            Self::Accept | Self::NakMaxClients => ScHubRegistrationKind::Initial,
            Self::Replace { old_vmac } if *old_vmac == requested_vmac => {
                ScHubRegistrationKind::SameUuidSameVmac
            }
            Self::Replace { .. } => ScHubRegistrationKind::SameUuidMovedVmac,
            Self::NakDuplicateVmac => ScHubRegistrationKind::ConflictingVmac,
        }
    }
}

/// Bounded input to the admin admission policy.
///
/// Request claims and channel facts are separate from the current locked
/// registration classification; none binds a claimed UUID to a certificate:
///
/// - `claimed_vmac` / `claimed_uuid` / `claimed_max_bvlc` /
///   `claimed_max_npdu` are the Connect-Request payload bytes. Device UUID
///   shape and equality are registry keys for duplicate handling, not
///   identity proof.
/// - `tls_client_verified` reports only whether the TLS handshake presented
///   a client certificate chain the acceptor verified against the
///   configured CA. No subject, fingerprint, or other certificate field is
///   extracted or exposed (that surface stays excluded).
/// - `provenance` is the RB-07 peer context for this channel: the verified
///   direct-peer variant when `tls_client_verified` holds, else unverified.
///   Its scope is the TLS channel before Connect-Accept; the claimed VMAC
///   and UUID are payload claims inside that channel, not certificate-bound
///   (see [`TransportProvenance`]).
///
/// Only requests that already passed shape validation (nonzero UUID,
/// nonzero limits) and reserved-VMAC screening reach the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubAdmissionInput {
    /// Current registry relationship, classified before this policy executes
    /// under the same lock as the eventual registration decision. An Allow
    /// still applies standard collision/capacity rules; it does not force accept.
    pub registration: ScHubRegistrationKind,
    /// Source socket of the TCP connection carrying the request.
    pub peer: SocketAddr,
    /// VMAC claimed in the Connect-Request payload.
    pub claimed_vmac: Vmac,
    /// Device UUID claimed in the Connect-Request payload.
    pub claimed_uuid: [u8; 16],
    /// Max-BVLC-Length claimed in the Connect-Request payload.
    pub claimed_max_bvlc: u16,
    /// Max-NPDU-Length claimed in the Connect-Request payload.
    pub claimed_max_npdu: u16,
    /// Whether the TLS handshake verified a client certificate against the
    /// configured CA (boolean channel only; no certificate fields).
    pub tls_client_verified: bool,
    /// RB-07 peer context for this channel (see scope note above).
    pub provenance: TransportProvenance,
}

/// Admin admission verdict for one Connect-Request.
///
/// `Deny` answers with the existing `RESOURCES`/`OTHER` NAK family (same
/// wire signal as `NakMaxClients`, per owner approval), closes without map
/// mutation, leaves any incumbent untouched, and bumps the saturating deny
/// counter. It is structurally distinct from Replace and NAK-duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScHubAdmissionDecision {
    /// Admit into the standard AB.6.2.3 registration decision.
    Allow,
    /// Refuse with `RESOURCES`/`OTHER`, no state change, counted denial.
    Deny,
}

/// Operator admission policy: a synchronous function of the bounded input.
///
/// Stored in [`super::ScHubTlsConfig`] via `with_admission_policy` and
/// shared by clones; every hub built from the config keeps independent
/// deny counters. Absent policy means allow-all (limits still apply).
pub type ScHubAdmissionPolicy =
    Arc<dyn Fn(&ScHubAdmissionInput) -> ScHubAdmissionDecision + Send + Sync>;

/// Bounded hub snapshot: counts and kind labels only.
///
/// - `listening` is the accept-loop state (false after [`super::ScHub::stop`]).
/// - `client_count` is the established (registered) client count.
/// - `handshake_count` derives from existing state (accepted slots minus
///   registered clients); it is approximate under replacement churn, not a
///   transactional read, and never a second state machine.
/// - `admin_denied` is the saturating lifetime count of admin-policy
///   denials. Capacity NAKs and silent accept drops are not included.
/// - `broadcast_drops` reuses the existing saturating relay counters.
///
/// Redacted by construction: no certificates, keys, VMAC maps, or payloads.
/// [`super::ScHub::status`] remains usable after `stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubStatus {
    /// Whether the accept loop still runs.
    pub listening: bool,
    /// Configured admission bounds for this hub.
    pub limits: ScHubAdmissionLimits,
    /// Established (registered) clients.
    pub client_count: usize,
    /// Accepted but unregistered connections (approximate; see above).
    pub handshake_count: usize,
    /// Lifetime admin-policy denials (saturating).
    pub admin_denied: u64,
    /// Lifetime broadcast-relay drop counters (saturating).
    pub broadcast_drops: super::ScHubBroadcastDropCounts,
}

/// Per-hub admission runtime: validated limits, optional shared policy,
/// and hub-owned saturating deny counter.
///
/// The config carries values; this carries counters. Every hub startup
/// builds exactly one, so clones of one [`super::ScHubTlsConfig`] never
/// share denial counts.
pub(super) struct AdmissionRuntime {
    pub(super) limits: ScHubAdmissionLimits,
    pub(super) policy: Option<ScHubAdmissionPolicy>,
    pub(super) denied: AtomicU64,
}

impl AdmissionRuntime {
    pub(super) fn new(limits: ScHubAdmissionLimits, policy: Option<ScHubAdmissionPolicy>) -> Self {
        Self {
            limits,
            policy,
            denied: AtomicU64::new(0),
        }
    }

    /// Evaluate the admin policy for one request.
    ///
    /// Called under the registry lock, before `deadline.commit()`. The
    /// policy is synchronous and must not block, perform I/O, or await:
    /// it runs while the registry is locked. A missing policy allows; a
    /// panicking policy fails closed to [`ScHubAdmissionDecision::Deny`]
    /// without touching the registry.
    pub(super) fn evaluate(&self, input: &ScHubAdmissionInput) -> ScHubAdmissionDecision {
        let Some(policy) = self.policy.as_ref() else {
            return ScHubAdmissionDecision::Allow;
        };
        match std::panic::catch_unwind(AssertUnwindSafe(|| policy(input))) {
            Ok(decision) => decision,
            Err(_) => {
                tracing::warn!("Hub: admission policy panicked, denying");
                ScHubAdmissionDecision::Deny
            }
        }
    }

    /// Record one admin denial (saturating; statistics only, never an
    /// input to limiting decisions).
    pub(super) fn note_denied(&self) {
        let _ = self
            .denied
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            });
    }

    pub(super) fn denied(&self) -> u64 {
        self.denied.load(Ordering::Relaxed)
    }
}

impl Default for AdmissionRuntime {
    /// Allow-all policy with default limits, for direct-handler test
    /// harnesses that bypass public startup. Public startup always builds
    /// from validated config.
    fn default() -> Self {
        Self::new(ScHubAdmissionLimits::default(), None)
    }
}

/// RB-07 peer context for the hub admission channel.
///
/// Verified direct peer exactly when the TLS handshake verified a client
/// certificate against the configured CA; unverified otherwise. Scope is
/// the pre-Connect-Accept TLS channel: claims stay claims.
pub(super) fn channel_provenance(tls_client_verified: bool) -> TransportProvenance {
    if tls_client_verified {
        TransportProvenance::verified_direct_peer()
    } else {
        TransportProvenance::unverified()
    }
}

/// Admin-deny NAK for a Connect-Request: the existing `RESOURCES`/`OTHER`
/// family shared with `NakMaxClients` (owner-approved wire signal).
pub(super) fn connect_denied_nak(message_id: u16) -> crate::sc_frame::ScMessage {
    use bacnet_types::enums::{ErrorClass, ErrorCode};

    use super::helpers::build_bvlc_result_nak;
    use crate::sc_frame::ScFunction;

    build_bvlc_result_nak(
        message_id,
        ScFunction::ConnectRequest,
        ErrorClass::RESOURCES,
        ErrorCode::OTHER,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_preserve_previous_total() {
        let limits = ScHubAdmissionLimits::default();
        assert_eq!(limits.max_clients, DEFAULT_MAX_CLIENTS);
        assert_eq!(limits.max_handshakes, DEFAULT_MAX_HANDSHAKES);
        assert_eq!(limits.max_clients, 256);
        assert_eq!(limits.max_handshakes, 256);
        assert_eq!(limits.total_active(), 512);
    }

    #[test]
    fn checked_constructor_rejects_zero_and_overflow() {
        for (clients, handshakes) in [(0, 1), (1, 0), (0, 0)] {
            let error = ScHubAdmissionLimits::new(clients, handshakes).unwrap_err();
            assert!(
                matches!(error, Error::Encoding(ref message) if message.contains("must be nonzero")),
                "{error}"
            );
        }
        let error = ScHubAdmissionLimits::new(usize::MAX, 1).unwrap_err();
        assert!(
            matches!(error, Error::Encoding(ref message) if message.contains("overflow")),
            "{error}"
        );
        assert_eq!(
            ScHubAdmissionLimits::new(1, 1).unwrap(),
            ScHubAdmissionLimits {
                max_clients: 1,
                max_handshakes: 1
            }
        );
    }

    #[test]
    fn missing_policy_allows_and_panicking_policy_denies() {
        let open = AdmissionRuntime::default();
        assert_eq!(
            open.evaluate(&allow_input([0x01; 6])),
            ScHubAdmissionDecision::Allow
        );
        let deny_all = AdmissionRuntime::new(
            ScHubAdmissionLimits::default(),
            Some(Arc::new(|_| ScHubAdmissionDecision::Deny)),
        );
        assert_eq!(
            deny_all.evaluate(&allow_input([0x01; 6])),
            ScHubAdmissionDecision::Deny
        );
        let by_vmac = AdmissionRuntime::new(
            ScHubAdmissionLimits::default(),
            Some(Arc::new(|input: &ScHubAdmissionInput| {
                if input.claimed_vmac == [0x09; 6] {
                    ScHubAdmissionDecision::Deny
                } else {
                    ScHubAdmissionDecision::Allow
                }
            })),
        );
        assert_eq!(
            by_vmac.evaluate(&allow_input([0x09; 6])),
            ScHubAdmissionDecision::Deny
        );
        assert_eq!(
            by_vmac.evaluate(&allow_input([0x08; 6])),
            ScHubAdmissionDecision::Allow
        );
        let panicking = AdmissionRuntime::new(
            ScHubAdmissionLimits::default(),
            Some(Arc::new(|_| -> ScHubAdmissionDecision {
                panic!("operator policy bug")
            })),
        );
        assert_eq!(
            panicking.evaluate(&allow_input([0x01; 6])),
            ScHubAdmissionDecision::Deny
        );
        assert_eq!(
            panicking.denied(),
            0,
            "evaluate must not count; only actual denies do"
        );
    }

    #[test]
    fn deny_counter_saturates() {
        let runtime = AdmissionRuntime::default();
        runtime.note_denied();
        assert_eq!(runtime.denied(), 1);
        runtime.denied.store(u64::MAX, Ordering::Relaxed);
        runtime.note_denied();
        assert_eq!(runtime.denied(), u64::MAX);
    }

    #[test]
    fn channel_provenance_maps_the_boolean_channel() {
        assert!(channel_provenance(true).is_direct_peer());
        assert!(channel_provenance(false).is_unverified());
    }

    #[test]
    fn status_debug_is_counts_and_labels_only() {
        let status = ScHubStatus {
            listening: true,
            limits: ScHubAdmissionLimits::default(),
            client_count: 1,
            handshake_count: 2,
            admin_denied: 3,
            broadcast_drops: super::super::ScHubBroadcastDropCounts {
                sender_exhausted: 4,
                global_exhausted: 5,
            },
        };
        // Exact-match: any future identity field breaks this on purpose.
        assert_eq!(
            format!("{status:?}"),
            "ScHubStatus { listening: true, limits: ScHubAdmissionLimits { max_clients: 256, max_handshakes: 256 }, \
             client_count: 1, handshake_count: 2, admin_denied: 3, \
             broadcast_drops: ScHubBroadcastDropCounts { sender_exhausted: 4, global_exhausted: 5 } }"
        );
    }

    fn allow_input(vmac: Vmac) -> ScHubAdmissionInput {
        ScHubAdmissionInput {
            registration: ScHubRegistrationKind::Initial,
            peer: "127.0.0.1:47808".parse().unwrap(),
            claimed_vmac: vmac,
            claimed_uuid: [0x11; 16],
            claimed_max_bvlc: 8192,
            claimed_max_npdu: 4096,
            tls_client_verified: true,
            provenance: TransportProvenance::verified_direct_peer(),
        }
    }
}

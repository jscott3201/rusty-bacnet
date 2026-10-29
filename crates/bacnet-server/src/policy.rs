//! One place for the server's authorization decisions (#524).
//!
//! [`ServerPolicy`] answers every "may this peer do this?" question the
//! server asks of an application: the covered mutations, LifeSafetyOperation,
//! DeviceCommunicationControl and both AuditNotification services. Each
//! method has a documented default, so a policy overrides only what it
//! decides.
//!
//! What stays outside it, and why:
//! - [`MutationPolicy::DenyAll`](crate::mutation::MutationPolicy::DenyAll)
//!   is a static hardening switch on [`ServerConfig`](crate::server::ServerConfig).
//!   It wins over any policy, which isn't asked, and its denials are counted
//!   apart.
//! - DeviceCommunicationControl's password, static
//!   [`DccPolicy`](crate::server::DccPolicy), source restriction and
//!   disable rate limit are settings, not callbacks. The policy is asked
//!   after the password, `DccPolicy` and source checks, and before the rate
//!   limit is charged.
//! - `on_reinitialize` carries out a restart rather than deciding on one,
//!   and runs after the server's own checks.
//! - `on_time_sync` observes a change it can't refuse.
//! - Routing control belongs to the router (`bacnet-network`'s
//!   `ControlAuthorizer`), not to the server.
//!
//! ## Who sent it
//!
//! Every context carries the claimed source addresses, the ingress
//! [`TransportProvenance`], and
//! from those a `principal()`: the [`ScPrincipal`] of the leaf certificate
//! when the request came over a verified direct BACnet/SC connection from
//! the peer that originated it, and `None` otherwise. B/IP, B/IPv6, MS/TP
//! and Ethernet have no principal. Neither does a request a hub relays: the
//! hub authenticated its own TLS client, and the origin's certificate never
//! reaches this node, so the policy can't tell one hub member from another.
//!
//! A routed request (one with an SNET/SADR source) has no principal either,
//! even over a direct connection: the connection authenticates the router
//! that forwarded it, not the device behind it. `forwarding_peer_principal()`
//! returns that router's principal, for a policy that trusts a router with
//! everything it forwards. A source VMAC, UUID or routed SNET/SADR is a
//! claim, never a principal.
//!
//! With `sc-tls`, every context's `certificate()` returns the verified leaf
//! certificate that `principal()` fingerprints, under the same rule, and
//! `forwarding_peer_certificate()` the forwarding peer's. Its metadata gives
//! the subject, issuer and Subject Alternative Names, so a rule can follow a
//! name across certificate renewal instead of listing old and new
//! fingerprints. Allow on a name found rather than deny on one missing, and
//! never log a certificate's contents.
//!
//! So `principal()` means the direct SC peer that sent the NPDU without an
//! SNET. A router that doesn't follow the standard, or an application
//! gateway that re-originates requests, shows its own principal for what it
//! relays: trust a principal as far as everything its holder may relay.
//!
//! SC mutual TLS authenticates a connection; it doesn't authorize a
//! service. Without a policy, any peer a CA admits may use every service
//! the server's defaults allow.
//!
//! ## Calling rules
//!
//! Every method should be quick and free of side effects. For a received
//! request the server calls them on Tokio's blocking pool with no object
//! database guard held (#2032; each method says when it asks), so a slow
//! policy delays only its own request, but a call that never returns keeps
//! the process from exiting. Calls may run concurrently. Returning `false`, or
//! panicking, denies, and the request changes nothing.
//!
//! ```
//! use bacnet_server::mutation::MutationAuthorizationContext;
//! use bacnet_server::ServerPolicy;
//! use bacnet_transport::port::ScPrincipal;
//!
//! /// Writes only from the operator workstation's certificate.
//! struct Site {
//!     operator: ScPrincipal,
//! }
//!
//! impl ServerPolicy for Site {
//!     fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
//!         context.principal() == Some(self.operator)
//!     }
//! }
//! ```
//!
//! [`FnPolicy`] builds one from closures instead, keeping the default of
//! every decision it isn't given:
//!
//! ```
//! use bacnet_server::policy::FnPolicy;
//! use bacnet_server::server::BACnetServer;
//!
//! // Remote writes from verified direct SC peers only; LifeSafetyOperation
//! // from anyone. Audit notifications keep denying.
//! let builder = BACnetServer::bip_builder().policy(
//!     FnPolicy::new()
//!         .mutation(|context| context.principal().is_some())
//!         .life_safety_operation(|_| true),
//! );
//! ```

use crate::audit_notification::{
    AuditNotificationAuthorizationContext, UnconfirmedAuditNotificationAuthorizationContext,
};
use crate::device_communication_control::DeviceCommunicationControlAuthorizationContext;
use crate::life_safety::LifeSafetyOperationAuthorizationContext;
use crate::mutation::MutationAuthorizationContext;
use bacnet_encoding::npdu::NpduAddress;
pub use bacnet_transport::port::ScPrincipal;
use bacnet_transport::port::TransportProvenance;
#[cfg(feature = "sc-tls")]
pub use bacnet_transport::sc_tls::{ScCertificateMetadata, ScPeerCertificate, ScSubjectAltName};
use std::sync::Arc;

/// The originating peer's principal: the direct-SC leaf, unless the request
/// was routed, when the connection only authenticates the forwarding router.
pub(crate) fn originating_principal(
    provenance: &TransportProvenance,
    source_network: Option<&NpduAddress>,
) -> Option<ScPrincipal> {
    source_network
        .is_none()
        .then(|| provenance.principal())
        .flatten()
}

/// The originating peer's verified leaf certificate, under the rule
/// [`originating_principal`] applies (#1965).
#[cfg(feature = "sc-tls")]
pub(crate) fn originating_certificate<'a>(
    provenance: &'a TransportProvenance,
    source_network: Option<&NpduAddress>,
) -> Option<&'a ScPeerCertificate> {
    source_network
        .is_none()
        .then(|| provenance.leaf_certificate())
        .flatten()
}

/// The `certificate()` and `forwarding_peer_certificate()` accessors every
/// policy context has, for a context with `provenance` and
/// `source_network` fields. With `sc-tls` only: no other build has direct
/// SC peers.
macro_rules! certificate_accessors {
    () => {
        /// The verified leaf certificate of the peer that originated the
        /// request: the one [`principal`](Self::principal) fingerprints, for
        /// rules by subject, Subject Alternative Name or issuer, which
        /// survive certificate renewal. `None` exactly when `principal()`
        /// is: other ingress, hub-relayed and routed requests (see
        /// [`crate::policy`]). Its contents are the peer's: don't log them.
        #[cfg(feature = "sc-tls")]
        pub fn certificate(&self) -> Option<&$crate::policy::ScPeerCertificate> {
            $crate::policy::originating_certificate(&self.provenance, self.source_network.as_ref())
        }

        /// The verified leaf certificate of the direct BACnet/SC peer that
        /// handed this request over, a router included when the request
        /// was routed: the one
        /// [`forwarding_peer_principal`](Self::forwarding_peer_principal)
        /// fingerprints. `None` off a verified direct SC connection.
        #[cfg(feature = "sc-tls")]
        pub fn forwarding_peer_certificate(&self) -> Option<&$crate::policy::ScPeerCertificate> {
            self.provenance.leaf_certificate()
        }
    };
}
pub(crate) use certificate_accessors;

/// The server's authorization decisions, each with a documented default.
///
/// See the [module documentation](self) for what it covers, the principal
/// each context carries and the calling rules. Install one with
/// `policy(..)` on a server builder, or in
/// [`ServerConfig::policy`](crate::server::ServerConfig::policy).
///
/// Without any policy installed, the server behaves as these defaults do,
/// except that it allows a covered mutation without decoding it for
/// authorization first; so does a policy whose
/// [`decides_mutations`](Self::decides_mutations) is `false`.
///
/// `Arc<P>` and `Box<P>` are policies when `P` is, so one policy can be
/// shared or boxed.
pub trait ServerPolicy: Send + Sync + 'static {
    /// Whether [`authorize_mutation`](Self::authorize_mutation) decides
    /// anything (default: `true`).
    ///
    /// When `false`, the server never asks it: each covered mutation is
    /// allowed without being decoded for authorization first, and counted as
    /// allowed before the handler runs, exactly as with no policy. A policy
    /// that only decides LifeSafetyOperation or Audit receipt returns `false`
    /// so that mutations keep that path. The server may read it more than
    /// once for one request, so it must give the same answer for the
    /// policy's lifetime.
    fn decides_mutations(&self) -> bool {
        true
    }

    /// Whether to carry out one covered mutation (default: allow).
    ///
    /// Asked after DeviceCommunicationControl's checks, request admission,
    /// service decoding and element validation, and before any database
    /// change, COV or event work, or Audit record. A denial writes no Audit
    /// record; it is counted in `BACnetServer::mutation_decision_counters`
    /// and answered SERVICES / SERVICE_REQUEST_DENIED.
    /// [`MutationPolicy::DenyAll`](crate::mutation::MutationPolicy::DenyAll)
    /// denies without asking.
    ///
    /// WritePropertyMultiple asks once per decoded, validated element, in
    /// wire order, up to the first it denies, all before the database is
    /// write-locked and with no database guard held (#2032); the writes
    /// follow under the lock. A denial, or a malformed later element, leaves
    /// the elements before it written. Since every element is asked about
    /// first, the policy can be asked about an element the request never
    /// reaches, when an earlier one fails as it is written. An element whose
    /// object appears between the decisions and the writes, so it was never
    /// asked about, is denied. The other confirmed services ask once, for
    /// the whole decoded request, also with no database guard held.
    ///
    /// Every call for a received request runs on Tokio's blocking pool while
    /// the request waits (#2032), so a policy that blocks delays only that
    /// request: it holds no runtime worker, and `BACnetServer::stop` ends
    /// without waiting for it (the abandoned call finishes on its own and is
    /// ignored). That costs one trip to the pool per request (a
    /// WritePropertyMultiple's elements share one; WriteGroup makes one per
    /// Channel), so keep the policy quick all the same. A call that never
    /// returns keeps its blocking thread after `stop()`, and a Tokio runtime
    /// waits for blocking tasks when it shuts down, so such a policy keeps
    /// the process from exiting. The other `authorize_*` methods for a
    /// received request run the same way.
    ///
    /// An inbound WriteGroup asks once per Channel it would write, with no
    /// database guard held, no invoke ID and the unconfirmed service in the
    /// context. A denial skips that Channel silently (nothing answers an
    /// unconfirmed request) and writes no Audit record; the other Channels
    /// are decided on their own (#1319).
    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        let _ = context;
        true
    }

    /// Whether to carry out a LifeSafetyOperation (default: deny).
    ///
    /// Asked after the request is validated and its object found, with no
    /// database guard held, on Tokio's blocking pool as
    /// [`authorize_mutation`](Self::authorize_mutation) is (#2032). A denial
    /// is answered SERVICES / SERVICE_REQUEST_DENIED before the object
    /// changes.
    fn authorize_life_safety_operation(
        &self,
        context: &LifeSafetyOperationAuthorizationContext,
    ) -> bool {
        let _ = context;
        false
    }

    /// Whether to carry out a DeviceCommunicationControl request (default:
    /// allow).
    ///
    /// Asked after decoding, the password, the deprecated DISABLE refusal,
    /// the static [`DccPolicy`](crate::server::DccPolicy) and the source
    /// restriction; a request any of them refuses never reaches the policy.
    /// For DISABLE_INITIATION the disable rate limit is charged only after
    /// the policy allows, so a denied requester spends none of that shared
    /// budget. Asked with no lock held, on Tokio's blocking pool as
    /// [`authorize_mutation`](Self::authorize_mutation) is (#2032), before
    /// the timer is replaced or the state changes. A denial is answered SERVICES /
    /// SERVICE_REQUEST_DENIED, changes neither the state nor a running
    /// timer, writes no Audit record and is counted in
    /// `BACnetServer::dcc_outcome_counters` as `policy_denied_total`, which
    /// also counts the static source and rate-limit refusals.
    fn authorize_device_communication_control(
        &self,
        context: &DeviceCommunicationControlAuthorizationContext,
    ) -> bool {
        let _ = context;
        true
    }

    /// Whether to store a ConfirmedAuditNotification in the configured
    /// Audit Log sink (default: deny).
    ///
    /// Asked after the request is decoded and checked against completed
    /// receipts, with no database guard held, on Tokio's blocking pool as
    /// [`authorize_mutation`](Self::authorize_mutation) is (#2032). A denial
    /// is answered SERVICES / SERVICE_REQUEST_DENIED and stores nothing.
    fn authorize_audit_notification(
        &self,
        context: &AuditNotificationAuthorizationContext,
    ) -> bool {
        let _ = context;
        false
    }

    /// Whether to store an UnconfirmedAuditNotification in the configured
    /// Audit Log sink (default: deny).
    ///
    /// Asked after the request is decoded, with no database guard held, on
    /// Tokio's blocking pool as
    /// [`authorize_mutation`](Self::authorize_mutation) is (#2032). A denial
    /// drops the request silently and stores nothing.
    fn authorize_unconfirmed_audit_notification(
        &self,
        context: &UnconfirmedAuditNotificationAuthorizationContext,
    ) -> bool {
        let _ = context;
        false
    }
}

type Decide<C> = Arc<dyn Fn(&C) -> bool + Send + Sync>;

/// A [`ServerPolicy`] made of closures, one per decision. A decision with no
/// closure keeps the trait's default; with no mutation closure it doesn't
/// decide mutations at all ([`ServerPolicy::decides_mutations`]).
///
/// Each closure follows the calling rules of the method it stands for.
#[derive(Clone, Default)]
pub struct FnPolicy {
    mutation: Option<Decide<MutationAuthorizationContext>>,
    life_safety_operation: Option<Decide<LifeSafetyOperationAuthorizationContext>>,
    device_communication_control: Option<Decide<DeviceCommunicationControlAuthorizationContext>>,
    audit_notification: Option<Decide<AuditNotificationAuthorizationContext>>,
    unconfirmed_audit_notification:
        Option<Decide<UnconfirmedAuditNotificationAuthorizationContext>>,
}

impl FnPolicy {
    /// A policy that decides as the defaults do, until given closures.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide mutations with `decide`; see [`ServerPolicy::authorize_mutation`].
    pub fn mutation(
        mut self,
        decide: impl Fn(&MutationAuthorizationContext) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.mutation = Some(Arc::new(decide));
        self
    }

    /// Decide LifeSafetyOperation with `decide`; see
    /// [`ServerPolicy::authorize_life_safety_operation`].
    pub fn life_safety_operation(
        mut self,
        decide: impl Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.life_safety_operation = Some(Arc::new(decide));
        self
    }

    /// Decide DeviceCommunicationControl with `decide`; see
    /// [`ServerPolicy::authorize_device_communication_control`].
    pub fn device_communication_control(
        mut self,
        decide: impl Fn(&DeviceCommunicationControlAuthorizationContext) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.device_communication_control = Some(Arc::new(decide));
        self
    }

    /// Decide ConfirmedAuditNotification with `decide`; see
    /// [`ServerPolicy::authorize_audit_notification`].
    pub fn audit_notification(
        mut self,
        decide: impl Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.audit_notification = Some(Arc::new(decide));
        self
    }

    /// Decide UnconfirmedAuditNotification with `decide`; see
    /// [`ServerPolicy::authorize_unconfirmed_audit_notification`].
    pub fn unconfirmed_audit_notification(
        mut self,
        decide: impl Fn(&UnconfirmedAuditNotificationAuthorizationContext) -> bool
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.unconfirmed_audit_notification = Some(Arc::new(decide));
        self
    }
}

impl<P: ServerPolicy + ?Sized> ServerPolicy for Arc<P> {
    fn decides_mutations(&self) -> bool {
        (**self).decides_mutations()
    }
    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        (**self).authorize_mutation(context)
    }
    fn authorize_life_safety_operation(
        &self,
        context: &LifeSafetyOperationAuthorizationContext,
    ) -> bool {
        (**self).authorize_life_safety_operation(context)
    }
    fn authorize_device_communication_control(
        &self,
        context: &DeviceCommunicationControlAuthorizationContext,
    ) -> bool {
        (**self).authorize_device_communication_control(context)
    }
    fn authorize_audit_notification(
        &self,
        context: &AuditNotificationAuthorizationContext,
    ) -> bool {
        (**self).authorize_audit_notification(context)
    }
    fn authorize_unconfirmed_audit_notification(
        &self,
        context: &UnconfirmedAuditNotificationAuthorizationContext,
    ) -> bool {
        (**self).authorize_unconfirmed_audit_notification(context)
    }
}

impl<P: ServerPolicy + ?Sized> ServerPolicy for Box<P> {
    fn decides_mutations(&self) -> bool {
        (**self).decides_mutations()
    }
    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        (**self).authorize_mutation(context)
    }
    fn authorize_life_safety_operation(
        &self,
        context: &LifeSafetyOperationAuthorizationContext,
    ) -> bool {
        (**self).authorize_life_safety_operation(context)
    }
    fn authorize_device_communication_control(
        &self,
        context: &DeviceCommunicationControlAuthorizationContext,
    ) -> bool {
        (**self).authorize_device_communication_control(context)
    }
    fn authorize_audit_notification(
        &self,
        context: &AuditNotificationAuthorizationContext,
    ) -> bool {
        (**self).authorize_audit_notification(context)
    }
    fn authorize_unconfirmed_audit_notification(
        &self,
        context: &UnconfirmedAuditNotificationAuthorizationContext,
    ) -> bool {
        (**self).authorize_unconfirmed_audit_notification(context)
    }
}

impl std::fmt::Debug for FnPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnPolicy")
            .field("mutation", &self.mutation.is_some())
            .field(
                "life_safety_operation",
                &self.life_safety_operation.is_some(),
            )
            .field(
                "device_communication_control",
                &self.device_communication_control.is_some(),
            )
            .field("audit_notification", &self.audit_notification.is_some())
            .field(
                "unconfirmed_audit_notification",
                &self.unconfirmed_audit_notification.is_some(),
            )
            .finish()
    }
}

impl ServerPolicy for FnPolicy {
    /// Only when it was given a mutation closure.
    fn decides_mutations(&self) -> bool {
        self.mutation.is_some()
    }

    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        self.mutation.as_ref().is_none_or(|decide| decide(context))
    }

    fn authorize_life_safety_operation(
        &self,
        context: &LifeSafetyOperationAuthorizationContext,
    ) -> bool {
        self.life_safety_operation
            .as_ref()
            .is_some_and(|decide| decide(context))
    }

    fn authorize_device_communication_control(
        &self,
        context: &DeviceCommunicationControlAuthorizationContext,
    ) -> bool {
        self.device_communication_control
            .as_ref()
            .is_none_or(|decide| decide(context))
    }

    fn authorize_audit_notification(
        &self,
        context: &AuditNotificationAuthorizationContext,
    ) -> bool {
        self.audit_notification
            .as_ref()
            .is_some_and(|decide| decide(context))
    }

    fn authorize_unconfirmed_audit_notification(
        &self,
        context: &UnconfirmedAuditNotificationAuthorizationContext,
    ) -> bool {
        self.unconfirmed_audit_notification
            .as_ref()
            .is_some_and(|decide| decide(context))
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

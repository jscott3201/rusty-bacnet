//! One place for the server's authorization decisions (#524).
//!
//! [`ServerPolicy`] answers every "may this peer do this?" question the
//! server asks of an application: the covered mutations, LifeSafetyOperation
//! and both AuditNotification services. Each method has a documented
//! default, so a policy overrides only what it decides.
//!
//! What stays outside it, and why:
//! - [`MutationPolicy::DenyAll`](crate::mutation::MutationPolicy::DenyAll)
//!   is a static hardening switch on [`ServerConfig`](crate::server::ServerConfig).
//!   It wins over any policy, which isn't asked, and its denials are counted
//!   apart.
//! - DeviceCommunicationControl's password, source restriction and rate
//!   limit are static settings, not callbacks.
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
//! Every method must be fast, nonblocking and free of side effects. The
//! server calls them on its dispatch tasks, sometimes with the object
//! database locked (each method says when); calling back into the server
//! from one can deadlock. Calls may run concurrently. Returning `false`, or
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
use crate::life_safety::LifeSafetyOperationAuthorizationContext;
use crate::mutation::MutationAuthorizationContext;
use bacnet_encoding::npdu::NpduAddress;
pub use bacnet_transport::port::ScPrincipal;
use bacnet_transport::port::TransportProvenance;
use std::sync::Arc;

/// The originating peer's principal: the direct-SC leaf, unless the request
/// was routed, when the connection only authenticates the forwarding router.
pub(crate) fn originating_principal(
    provenance: TransportProvenance,
    source_network: Option<&NpduAddress>,
) -> Option<ScPrincipal> {
    source_network
        .is_none()
        .then(|| provenance.principal())
        .flatten()
}

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
    /// wire order, just before that element's write, with the database
    /// write-locked. A denial, or a malformed later element, leaves the
    /// elements before it written. An element that an object saves before
    /// serving it (a Notification Forwarder's or Notification Class's list,
    /// an Access Rights object's rules, Enable or Accompaniment, an Audit
    /// Log's Log_Enable or Buffer_Size) is asked about earlier, under a
    /// database read guard, before the save is staged off the write lock
    /// (#1321). It is still asked once and in wire order among such
    /// elements, so the policy can be asked about an element the request
    /// never reaches. The other confirmed services ask once, for the whole
    /// decoded request.
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
    /// database guard held. A denial is answered SERVICES /
    /// SERVICE_REQUEST_DENIED before the object changes.
    fn authorize_life_safety_operation(
        &self,
        context: &LifeSafetyOperationAuthorizationContext,
    ) -> bool {
        let _ = context;
        false
    }

    /// Whether to store a ConfirmedAuditNotification in the configured
    /// Audit Log sink (default: deny).
    ///
    /// Asked after the request is decoded and checked against completed
    /// receipts, with no database guard held. A denial is answered
    /// SERVICES / SERVICE_REQUEST_DENIED and stores nothing.
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
    /// Asked after the request is decoded, with no database guard held. A
    /// denial drops the request silently and stores nothing.
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

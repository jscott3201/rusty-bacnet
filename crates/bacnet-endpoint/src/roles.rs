//! Private role adapters above the sibling client/server roles.
//!
//! Composition-visible only: everything is `#[doc(hidden)]`, no public
//! facade promise. Inbound server transactions reuse the wire invoke ID
//! directly and NEVER allocate from the shared outbound client ID pool, so
//! equal inbound/outbound numeric IDs stay unambiguous via the ingress
//! classifier + coordinator admission.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CanonicalPeer, OutboundTransactionCoordinator,
};
use bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination;
use bacnet_network::layer::ReceivedApdu;
use bacnet_transport::port::DataAttribute;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_client::EndpointRequester;
use bacnet_server::server::{
    __endpoint_EndpointResponder as EndpointResponder,
    __endpoint_NotificationTransactions as NotificationTransactions,
};

fn shutdown_error() -> Error {
    Error::Encoding("endpoint shutdown".into())
}

/// Session-owned open state shared with role handles.
///
/// Roles hold only a [`Weak`] to this token: dropping the session ends role
/// work even when a role [`Arc`] was cloned out beforehand.
#[doc(hidden)]
pub struct SessionToken {
    open: AtomicBool,
    /// Shared device-wide outbound lease pool (client + server notifications).
    pub coordinator: Arc<OutboundTransactionCoordinator>,
    /// RB-17 one-shot deferred-reply arm (MS/TP `ReplyPostponed` wiring).
    ///
    /// When set, the session dispatch strips the one-use prompt reply sender
    /// from the next `reply_tx`-bearing inbound request before the responder
    /// sees it, forcing the token-owned egress path with identical
    /// addressing/invoke ID. Consumed exactly once via [`take_suspend`](Self::take_suspend);
    /// broadcasts (no `reply_tx`) never consume the arm.
    suspend_next: AtomicBool,
}

impl SessionToken {
    #[doc(hidden)]
    pub fn new(coordinator: Arc<OutboundTransactionCoordinator>) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicBool::new(true),
            coordinator,
            suspend_next: AtomicBool::new(false),
        })
    }

    #[doc(hidden)]
    pub fn shutdown(&self) {
        self.open.store(false, Ordering::Release);
    }

    #[doc(hidden)]
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Arms one-shot deferred-reply suspension (RB-17 MS/TP wiring).
    ///
    /// Test + slow-application seam: the next `reply_tx`-bearing inbound
    /// request is answered token-owned via egress (after the MAC releases
    /// `ReplyPostponed`) instead of promptly via `reply_tx`.
    #[doc(hidden)]
    pub fn suspend_next_reply(&self) {
        self.suspend_next.store(true, Ordering::Release);
    }

    /// Consumes the suspension arm exactly once (single dispatch consumer).
    #[doc(hidden)]
    pub fn take_suspend(&self) -> bool {
        self.suspend_next.swap(false, Ordering::AcqRel)
    }
}

/// Private client role: routed + attribute-preserving sends over the shared
/// egress/coordinator. No lifecycle methods; the session owner drives
/// `close()` on the inner requester.
#[doc(hidden)]
#[derive(Clone)]
pub struct ClientRoleHandle {
    token: Weak<SessionToken>,
    requester: EndpointRequester,
}

impl ClientRoleHandle {
    #[doc(hidden)]
    pub fn new(token: &Arc<SessionToken>, requester: EndpointRequester) -> Self {
        Self {
            token: Arc::downgrade(token),
            requester,
        }
    }

    fn session(&self) -> Result<Arc<SessionToken>, Error> {
        self.token.upgrade().ok_or_else(shutdown_error)
    }

    fn check_open(&self) -> Result<Arc<SessionToken>, Error> {
        let session = self.session()?;
        if !session.is_open() {
            return Err(shutdown_error());
        }
        Ok(session)
    }

    /// Direct ReadProperty (compat: no attributes).
    #[doc(hidden)]
    pub async fn read_property(
        &self,
        destination_mac: &[u8],
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        self.check_open()?;
        // RB-07 compat: provenance passes through `ReceivedApdu` on the
        // reply path; no new decision is made here.
        self.requester
            .read_property(
                destination_mac,
                object_identifier,
                property_identifier,
                property_array_index,
            )
            .await
    }

    /// Routed ReadProperty with pass-through data attributes.
    #[doc(hidden)]
    pub async fn read_property_routed(
        &self,
        router_mac: &[u8],
        destination_network: u16,
        destination_mac: &[u8],
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        self.check_open()?;
        self.requester
            .read_property_routed(
                router_mac,
                destination_network,
                destination_mac,
                data_attributes,
                object_identifier,
                property_identifier,
                property_array_index,
            )
            .await
    }

    /// Explicit-destination ReadProperty with pass-through attributes.
    #[doc(hidden)]
    pub async fn read_property_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        self.check_open()?;
        self.requester
            .read_property_with_destination(
                destination,
                data_attributes,
                object_identifier,
                property_identifier,
                property_array_index,
            )
            .await
    }

    /// Completes one pre-admitted response (session dispatch only).
    ///
    /// Preserves routing/attributes/provenance via the inner requester;
    /// never allocates an Invoke ID (the lease was reserved before the send).
    #[doc(hidden)]
    pub async fn complete_pre_admitted(
        &self,
        admission: bacnet_endpoint_core::coordinator::Admission,
        apdu: Apdu,
        received: ReceivedApdu,
    ) -> bool {
        if self.check_open().is_err() {
            return false;
        }
        self.requester
            .complete_pre_admitted(admission, apdu, received)
            .await
    }

    /// Internal close for the session owner only.
    #[doc(hidden)]
    pub fn close_for_owner(&self) {
        self.requester.close();
    }
}

/// Private server role: narrow responder + shared notification pool.
///
/// Service scope stays narrow (`ReadProperty` + `Reject`/`Abort` +
/// segmentation-`Abort`); full parity is a later packet. No lifecycle
/// methods on this handle; the session owner drives dispatch + `close()`.
#[doc(hidden)]
#[derive(Clone)]
pub struct ServerRoleHandle {
    token: Weak<SessionToken>,
    responder: Arc<EndpointResponder>,
    notifications: Arc<NotificationTransactions>,
}

impl ServerRoleHandle {
    #[doc(hidden)]
    pub fn new(
        token: &Arc<SessionToken>,
        responder: Arc<EndpointResponder>,
        notifications: Arc<NotificationTransactions>,
    ) -> Self {
        Self {
            token: Arc::downgrade(token),
            responder,
            notifications,
        }
    }

    fn check_open(&self) -> Result<(), Error> {
        let session = self.token.upgrade().ok_or_else(shutdown_error)?;
        if !session.is_open() {
            return Err(shutdown_error());
        }
        Ok(())
    }

    /// Returns `true` while the owning session is alive and open.
    ///
    /// Test seam proving no detached role outlives the endpoint: after the
    /// session drops, [`Weak::upgrade`] fails and this returns `false`.
    #[doc(hidden)]
    pub fn is_session_alive(&self) -> bool {
        self.token.upgrade().is_some_and(|s| s.is_open())
    }

    /// Arms one-shot deferred-reply suspension (RB-17 MS/TP wiring).
    ///
    /// The next `reply_tx`-bearing inbound request handled while this arm is
    /// set answers token-owned via egress (after the MAC releases
    /// `ReplyPostponed`) instead of promptly via `reply_tx`. Fails closed
    /// once the owning session shuts down. Broadcasts (no `reply_tx`) never
    /// consume the arm.
    #[doc(hidden)]
    pub fn suspend_next_reply(&self) -> Result<(), Error> {
        let session = self.token.upgrade().ok_or_else(shutdown_error)?;
        if !session.is_open() {
            return Err(shutdown_error());
        }
        session.suspend_next_reply();
        Ok(())
    }

    /// Handles one inbound request via the wire invoke ID directly.
    ///
    /// Session dispatch only. Never touches the outbound coordinator pool.
    /// Preserves link-group/attributes/provenance structurally through the
    /// inner responder (pass-through, no new decisions). Honors a pending
    /// RB-17 suspension arm identically to session dispatch (strips
    /// `reply_tx` so the responder answers token-owned via egress).
    #[doc(hidden)]
    pub async fn handle_inbound(&self, mut received: ReceivedApdu) -> Result<bool, Error> {
        self.check_open()?;
        if received.reply_tx.is_some() && self.token.upgrade().is_some_and(|s| s.take_suspend()) {
            let _ = received.reply_tx.take();
        }
        self.responder.handle(received).await
    }

    /// Admits one terminal response for a server notification lease.
    ///
    /// Standalone admit path (tries the shared coordinator once). Session
    /// dispatch prefers [`Self::complete_notification_pre_admitted`] after
    /// its single [`admit_once`] to avoid double-admit.
    #[doc(hidden)]
    pub fn admit_notification_terminal(
        &self,
        immediate_source: &[u8],
        routed_source: Option<&bacnet_encoding::npdu::NpduAddress>,
        apdu: &Apdu,
    ) -> bool {
        if self.check_open().is_err() {
            return false;
        }
        self.notifications
            .admit_terminal(immediate_source, routed_source, apdu)
    }

    /// Completes one already-admitted notification lease (dispatch only).
    ///
    /// The session's single [`admit_once`] owns exact-once claim; this
    /// releases the exact lease without re-admitting.
    #[doc(hidden)]
    pub fn complete_notification_pre_admitted(
        &self,
        admission: bacnet_endpoint_core::coordinator::Admission,
        apdu: &Apdu,
    ) -> bool {
        if self.check_open().is_err() {
            return false;
        }
        self.notifications.complete_pre_admitted(admission, apdu)
    }

    /// Internal close for the session owner only.
    #[doc(hidden)]
    pub fn close_for_owner(&self) {
        self.responder.close();
        self.notifications.close();
    }
}

/// Derives the canonical peer for one received envelope (direct vs routed).
///
/// RB-07 compat: provenance is preserved by the caller and never gates here.
#[doc(hidden)]
pub fn inbound_canonical_peer(received: &ReceivedApdu) -> CanonicalPeer {
    CanonicalPeer::from_source(
        received.source_mac.as_slice(),
        received.source_network.as_ref(),
    )
}

/// Decodes one terminal/segment APDU, preserving the full envelope for the
/// caller (source/dest, raw + effective group, attributes, provenance).
#[doc(hidden)]
pub fn decode_terminal(received: &ReceivedApdu) -> Option<Apdu> {
    decode_apdu(received.apdu.clone()).ok()
}

/// Classifies whether an admitted lease belongs to the client requester.
///
/// Equal inbound/outbound numeric IDs are legal: ownership (`Requester` vs
/// `ServerNotification`), not the numeric value, selects the consumer.
#[doc(hidden)]
pub fn is_requester_lease(admission: &bacnet_endpoint_core::coordinator::Admission) -> bool {
    admission.metadata().owner() == bacnet_endpoint_core::coordinator::LeaseOwner::Requester
}

/// Single-admit helper for session dispatch: exactly one
/// [`OutboundTransactionCoordinator::admit`] per received terminal APDU.
///
/// Returns the coordinator outcome without releasing the lease; the selected
/// role completes it exactly once via its pre-admitted path.
#[doc(hidden)]
pub fn admit_once(
    coordinator: &OutboundTransactionCoordinator,
    received: &ReceivedApdu,
    apdu: &Apdu,
) -> Result<AdmissionOutcome, bacnet_endpoint_core::coordinator::CoordinatorError> {
    let peer = inbound_canonical_peer(received);
    coordinator.admit(&peer, apdu)
}

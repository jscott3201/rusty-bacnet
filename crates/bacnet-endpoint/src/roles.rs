//! Session-bound role handles above the sibling client/server roles.
//!
//! [`ClientRoleHandle`] exposes unsegmented ReadProperty and ReadRange initiation;
//! [`ServerRoleHandle`] exposes inbound handling, session liveness, the
//! one-shot deferred-reply arm, and notification admit-complete. Neither
//! handle exposes lifecycle: `start`/`stop` exist only on
//! [`EndpointSession`](crate::session::EndpointSession), and the owner-only
//! shutdown helpers are `pub(crate)`.
//!
//! Both handles are `Send + Sync` (statically asserted below), so a handle
//! cloned out of the session may move across tasks and survive the session
//! drop as a value — every call then fails closed via the [`Weak`] token.
//!
//! ```compile_fail,E0599
//! // No lifecycle on roles: `start` exists only on `EndpointSession`.
//! # use bacnet_endpoint::roles::ServerRoleHandle;
//! # async fn forbidden(handle: ServerRoleHandle) {
//! handle.start().await;
//! # }
//! ```

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

/// Client role: ReadProperty and ReadRange over shared egress/coordinator.
///
/// Borrowed from a running [`EndpointSession`](crate::session::EndpointSession)
/// via `client()` / `cloned_client_handle()`. No lifecycle methods: after the
/// owning session stops or drops, every call returns
/// [`Error::Encoding`](bacnet_types::error::Error::Encoding) (`"endpoint
/// shutdown"`). `Send + Sync`, so clones may outlive the session borrow and
/// move across tasks.
///
/// Service scope is deliberately narrow: unsegmented ReadProperty and ReadRange.
/// Explicit destinations preserve data attributes and ingress provenance. When source READ
/// reporting is selected, only direct B/IP IPv4 unicast targets are admitted.
/// Audited calls are session-owned before egress: dropping their caller does not
/// cancel an admitted request or its terminal observation. Other calls retain
/// caller-owned cancellation. Stop/drop may discard undelivered source records.
#[derive(Clone)]
pub struct ClientRoleHandle {
    token: Weak<SessionToken>,
    requester: EndpointRequester,
    source_read: Option<Weak<crate::source_read::SourceRead>>,
}

impl ClientRoleHandle {
    #[doc(hidden)]
    pub fn new(token: &Arc<SessionToken>, requester: EndpointRequester) -> Self {
        Self {
            token: Arc::downgrade(token),
            requester,
            source_read: None,
        }
    }

    pub(crate) fn with_source_read(mut self, source: &Arc<crate::source_read::SourceRead>) -> Self {
        self.source_read = Some(Arc::downgrade(source));
        self
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

    /// Direct ReadProperty (no routing attributes).
    ///
    /// ACK object/property/index correlation matches the standalone client.
    /// Device/Network Port wildcard requests accept a same-type concrete ACK;
    /// other mismatches return a decoding error.
    ///
    /// Fails closed with `"endpoint shutdown"` once the owning session stops
    /// or drops. Timeouts/retries come from the session config (or the
    /// composed identity's max-APDU override for the length clamp).
    pub async fn read_property(
        &self,
        destination_mac: &[u8],
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        self.read_property_with_destination(
            EndpointApduDestination::Direct {
                destination_mac: bacnet_types::MacAddr::from_slice(destination_mac),
            },
            Vec::new(),
            object_identifier,
            property_identifier,
            property_array_index,
        )
        .await
    }

    /// Routed ReadProperty with pass-through data attributes.
    ///
    /// `router_mac` is the immediate neighbor; `destination_network` +
    /// `destination_mac` address the routed target. Attributes ride along
    /// unchanged. Same fail-closed shutdown semantics as
    /// [`read_property`](Self::read_property).
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
        self.read_property_with_destination(
            EndpointApduDestination::Routed {
                router_mac: bacnet_types::MacAddr::from_slice(router_mac),
                destination_network,
                destination_mac: bacnet_types::MacAddr::from_slice(destination_mac),
            },
            data_attributes,
            object_identifier,
            property_identifier,
            property_array_index,
        )
        .await
    }

    /// Explicit-destination ReadProperty with pass-through attributes.
    ///
    /// Covers local-broadcast and addressed destinations the direct/routed
    /// shorthands cannot spell. Same fail-closed shutdown semantics as
    /// [`read_property`](Self::read_property).
    pub async fn read_property_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        self.check_open()?;
        if let Some(source) = &self.source_read {
            let source = source.upgrade().ok_or_else(shutdown_error)?;
            return source
                .read(
                    &self.requester,
                    destination,
                    data_attributes,
                    bacnet_client::EndpointReadRequest::Property(
                        bacnet_services::read_property::ReadPropertyRequest {
                            object_identifier,
                            property_identifier,
                            property_array_index,
                        },
                    ),
                )
                .await?
                .into_property();
        }
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

    /// Read 1–64 explicit property references across nonempty concrete objects.
    ///
    /// ALL/REQUIRED/OPTIONAL and wildcard objects are excluded from this endpoint
    /// profile. Index zero is valid. Responses must match complete ordered
    /// occurrences; inline errors may omit a requested index. An active source
    /// Reporter emits one value-free record per eligible occurrence, sharing one
    /// operation, timestamp, invoke ID and recipient snapshot.
    pub async fn read_property_multiple(
        &self,
        destination_mac: &[u8],
        specs: Vec<bacnet_services::rpm::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        self.read_property_multiple_with_destination(
            EndpointApduDestination::Direct {
                destination_mac: bacnet_types::MacAddr::from_slice(destination_mac),
            },
            Vec::new(),
            specs,
        )
        .await
    }

    /// Explicit RPM destination and pass-through attributes. Audited operations
    /// require direct B/IP IPv4 unicast; all responses remain unsegmented.
    pub async fn read_property_multiple_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        specs: Vec<bacnet_services::rpm::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        self.check_open()?;
        let request = bacnet_client::EndpointReadRequest::Multiple(
            bacnet_services::rpm::ReadPropertyMultipleRequest {
                list_of_read_access_specs: specs,
            },
        );
        if let Some(source) = &self.source_read {
            source
                .upgrade()
                .ok_or_else(shutdown_error)?
                .read(&self.requester, destination, data_attributes, request)
                .await?
                .into_multiple()
        } else {
            self.requester
                .prepare_read(destination, data_attributes, request)?
                .execute()
                .await
                .result?
                .into_multiple()
        }
    }

    /// Read a list/log range through the shared unsegmented requester.
    ///
    /// An active source Reporter emits one value-free READ record per attempted
    /// operation. Audited destinations must be direct B/IP unicast. Invalid
    /// selectors, array index zero, counts and ByTime components fail prewire.
    pub async fn read_range(
        &self,
        destination_mac: &[u8],
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
        range: Option<bacnet_services::read_range::RangeSpec>,
    ) -> Result<bacnet_services::read_range::ReadRangeAck, Error> {
        self.read_range_with_destination(
            EndpointApduDestination::Direct {
                destination_mac: bacnet_types::MacAddr::from_slice(destination_mac),
            },
            Vec::new(),
            object_identifier,
            property_identifier,
            property_array_index,
            range,
        )
        .await
    }

    /// Explicit destination and pass-through attributes for ReadRange.
    /// Routed destinations are available when source reporting is not selected
    /// for the operation. Responses remain unsegmented.
    pub async fn read_range_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
        range: Option<bacnet_services::read_range::RangeSpec>,
    ) -> Result<bacnet_services::read_range::ReadRangeAck, Error> {
        self.check_open()?;
        if let Some(source) = &self.source_read {
            let source = source.upgrade().ok_or_else(shutdown_error)?;
            return source
                .read(
                    &self.requester,
                    destination,
                    data_attributes,
                    bacnet_client::EndpointReadRequest::Range(
                        bacnet_services::read_range::ReadRangeRequest {
                            object_identifier,
                            property_identifier,
                            property_array_index,
                            range,
                        },
                    ),
                )
                .await?
                .into_range();
        }
        self.requester
            .read_range_with_destination(
                destination,
                data_attributes,
                object_identifier,
                property_identifier,
                property_array_index,
                range,
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
    ///
    /// Owner-only seam (`pub(crate)`): the session [`Drop`](std::ops::Drop)
    /// path currently closes the inner requester directly; this stays for
    /// owner-driven shutdown without exposing lifecycle on the handle.
    #[allow(dead_code)]
    pub(crate) fn close_for_owner(&self) {
        self.requester.close();
    }
}

/// Server role: narrow responder + shared notification pool.
///
/// Service scope stays narrow: `ReadProperty`, optionally authorized local
/// Device.Description `WriteProperty`, and `Reject`/`Abort`. Full
/// `bacnet-server` parity is a later packet. No
/// lifecycle methods on this handle; the session owner drives dispatch +
/// `close()`. `Send + Sync`; every method fails closed after shutdown.
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
    /// Cloned handles survive as values but report `false` and fail closed.
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
    /// its single [`admit_once`] to avoid double-admit. Returns `false` after
    /// shutdown or when no lease is available.
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
    /// releases the exact lease without re-admitting. Returns `false` after
    /// shutdown.
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
    ///
    /// Owner-only seam (`pub(crate)`): retained for owner-driven shutdown
    /// without exposing lifecycle on the handle.
    #[allow(dead_code)]
    pub(crate) fn close_for_owner(&self) {
        self.responder.close();
        self.notifications.close();
    }
}

fn assert_send_sync<T: Send + Sync>() {}

const _: fn() = || {
    assert_send_sync::<ClientRoleHandle>();
    assert_send_sync::<ServerRoleHandle>();
};

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
/// `Notification`), not the numeric value, selects the consumer.
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

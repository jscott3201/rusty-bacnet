//! The session-bound handle of the narrow client role.
//!
//! [`ClientRoleHandle`] exposes unsegmented reads and direct B/IP
//! WriteProperty initiation. It exposes no lifecycle: `start`/`stop` exist
//! only on [`EndpointSession`](crate::session::EndpointSession), and the
//! owner-only shutdown helper is `pub(crate)`. The server role is the full
//! server the session owns and
//! [`EndpointSession::server`](crate::session::EndpointSession::server)
//! borrows (#2617).
//!
//! The handle is `Send + Sync` (statically asserted below), so a handle
//! cloned out of the session may move across tasks and survive the session
//! drop as a value — every call then fails closed via the [`Weak`] token.
//!
//! ```compile_fail,E0599
//! // No lifecycle on roles: `start` exists only on `EndpointSession`.
//! # use bacnet_endpoint::roles::ClientRoleHandle;
//! # async fn forbidden(handle: ClientRoleHandle) {
//! handle.start().await;
//! # }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_endpoint_core::coordinator::{AdmissionOutcome, OutboundTransactionCoordinator};
pub use bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination;
use bacnet_network::layer::ReceivedApdu;
use bacnet_transport::port::DataAttribute;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_client::EndpointRequester;

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
}

impl SessionToken {
    #[doc(hidden)]
    pub fn new(coordinator: Arc<OutboundTransactionCoordinator>) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicBool::new(true),
            coordinator,
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
}

/// Caller assertion about the remote target property's command semantics.
/// No local object lookup, priority argument, or remote discovery supplies this fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commandability {
    /// The property accepts prioritized commands; omitted priority is effective 16.
    Commandable,
    /// The property is not commandable; Audit ignores any supplied wire priority.
    Noncommandable,
}

/// Client role: reads and direct B/IP WriteProperty over shared egress/coordinator.
///
/// Borrowed from a running [`EndpointSession`](crate::session::EndpointSession)
/// via `client()` / `cloned_client_handle()`. No lifecycle methods: after the
/// owning session stops or drops, every call returns
/// [`Error::Encoding`] (`"endpoint
/// shutdown"`). `Send + Sync`, so clones may outlive the session borrow and
/// move across tasks.
///
/// Service scope is unsegmented RP/RR/RPM, the paged log read over them, and
/// direct B/IP WriteProperty. Explicit destinations preserve data attributes
/// and ingress provenance. When source Audit reporting is selected, an
/// audited read goes to a direct unicast on the link or to one station
/// behind a router (#2371).
/// Audited calls are session-owned before egress: dropping their caller does not
/// cancel an admitted request or its terminal observation. Other calls retain
/// caller-owned cancellation. Stop/drop may discard undelivered source records.
#[derive(Clone)]
pub struct ClientRoleHandle {
    token: Weak<SessionToken>,
    requester: EndpointRequester,
    bip_broadcast: Option<std::net::Ipv4Addr>,
    source_audit: Option<Weak<crate::source_audit::SourceAudit>>,
}

impl ClientRoleHandle {
    #[doc(hidden)]
    pub fn new(token: &Arc<SessionToken>, requester: EndpointRequester) -> Self {
        Self {
            token: Arc::downgrade(token),
            requester,
            bip_broadcast: None,
            source_audit: None,
        }
    }

    pub(crate) fn with_bip_broadcast(mut self, broadcast: Option<std::net::Ipv4Addr>) -> Self {
        self.bip_broadcast = broadcast;
        self
    }

    /// Binding preflight for encoded WriteProperty input, without a lease or traffic.
    #[doc(hidden)]
    pub fn validate_write_property(
        &self,
        request: &bacnet_services::write_property::WritePropertyRequest,
    ) -> Result<(), Error> {
        self.requester
            .validate_operation(&bacnet_client::EndpointOperationRequest::Write(
                request.clone(),
            ))
    }

    /// Write one property to a direct B/IP IPv4 unicast peer through the shared requester.
    ///
    /// `commandability` is required even without a source Reporter. It controls Audit
    /// filtering only: the supplied wire priority and complete encoded value are preserved.
    /// Empty lists are allowed; malformed TLVs and oversized unsegmented requests fail
    /// before traffic. An eligible admitted write remains session-owned after caller
    /// cancellation. Retries share one Invoke ID and produce at most one source record.
    pub async fn write_property(
        &self,
        destination_mac: &[u8],
        request: bacnet_services::write_property::WritePropertyRequest,
        commandability: Commandability,
    ) -> Result<(), Error> {
        self.check_open()?;
        let broadcast = self
            .bip_broadcast
            .ok_or_else(|| Error::Encoding("endpoint WriteProperty requires IPv4 B/IP".into()))?;
        let (ip, port) = bacnet_transport::bvll::decode_bip_mac(destination_mac)?;
        let ip = std::net::Ipv4Addr::from(ip);
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
            || ip == broadcast
            || port == 0
        {
            return Err(Error::Encoding(
                "endpoint WriteProperty requires direct B/IP IPv4 unicast".into(),
            ));
        }
        let destination = EndpointApduDestination::Direct {
            destination_mac: bacnet_types::MacAddr::from_slice(destination_mac),
        };
        if let Some(source) = &self.source_audit {
            source
                .upgrade()
                .ok_or_else(shutdown_error)?
                .write(&self.requester, destination, request, commandability)
                .await
        } else {
            self.requester
                .prepare_write(destination, Vec::new(), request)
                .await?
                .execute()
                .await
                .result?
                .into_write()
        }
    }

    pub(crate) fn with_source_audit(
        mut self,
        source: &Arc<crate::source_audit::SourceAudit>,
    ) -> Self {
        self.source_audit = Some(Arc::downgrade(source));
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

    /// Explicit-destination ReadProperty with pass-through attributes.
    ///
    /// Covers routed, local-broadcast and addressed destinations the direct
    /// shorthand cannot spell. Same fail-closed shutdown semantics as
    /// [`read_property`](Self::read_property). A routed destination names one
    /// device: a network outside 1..=65534, or a MAC that is empty or longer
    /// than 18 octets, fails with [`Error::Encoding`] before a transaction
    /// is reserved, as does a broadcast destination. Once the session knows
    /// its own network's number, a routed destination naming it goes to its
    /// MAC with no DNET (#1403). The answer from that MAC completes the read,
    /// and so does one a router relays back with that number as its SNET and
    /// that MAC as its SADR (#1465).
    pub async fn read_property_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        self.check_open()?;
        if let Some(source) = &self.source_audit {
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
        specs: Vec<bacnet_types::constructed::ReadAccessSpecification>,
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

    /// Explicit RPM destination and pass-through attributes, under the
    /// one-device rule of
    /// [`read_property_with_destination`](Self::read_property_with_destination).
    /// An audited operation goes to a direct unicast or to one station behind
    /// a router (#2371), a routed destination naming the session's own
    /// network number once it is known going direct (#1403); all responses
    /// remain unsegmented.
    pub async fn read_property_multiple_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        specs: Vec<bacnet_types::constructed::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        self.check_open()?;
        let request = bacnet_client::EndpointReadRequest::Multiple(
            bacnet_services::rpm::ReadPropertyMultipleRequest {
                list_of_read_access_specs: specs,
            },
        );
        if let Some(source) = &self.source_audit {
            source
                .upgrade()
                .ok_or_else(shutdown_error)?
                .read(&self.requester, destination, data_attributes, request)
                .await?
                .into_multiple()
        } else {
            self.requester
                .prepare_read(destination, data_attributes, request)
                .await?
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
        let request = bacnet_services::read_range::ReadRangeRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            range,
        };
        self.read_range_with_destination(
            EndpointApduDestination::Direct {
                destination_mac: bacnet_types::MacAddr::from_slice(destination_mac),
            },
            Vec::new(),
            request,
            bacnet_services::read_range::ReadRangeValidation::Strict,
        )
        .await
        .map(|reply| reply.ack)
    }

    /// Read a range of items, choosing how to treat an acknowledgement that
    /// breaks a rule, as
    /// [`BACnetClient::read_range_with`](bacnet_client::client::BACnetClient::read_range_with)
    /// does. Audited like [`read_range`](Self::read_range).
    pub async fn read_range_with(
        &self,
        destination_mac: &[u8],
        request: &bacnet_services::read_range::ReadRangeRequest,
        validation: bacnet_services::read_range::ReadRangeValidation,
    ) -> Result<bacnet_services::read_range::ReadRangeReply, Error> {
        self.read_range_with_destination(
            EndpointApduDestination::Direct {
                destination_mac: bacnet_types::MacAddr::from_slice(destination_mac),
            },
            Vec::new(),
            request.clone(),
            validation,
        )
        .await
    }

    /// [`read_range_with`](Self::read_range_with) to an explicit destination,
    /// with pass-through attributes, under the one-device rule of
    /// [`read_property_with_destination`](Self::read_property_with_destination).
    /// A routed destination naming the session's own network number, once
    /// it is known, goes direct (#1403). An audited read of a station behind
    /// a router names it by its network and MAC (#2371). Responses remain
    /// unsegmented.
    pub async fn read_range_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        request: bacnet_services::read_range::ReadRangeRequest,
        validation: bacnet_services::read_range::ReadRangeValidation,
    ) -> Result<bacnet_services::read_range::ReadRangeReply, Error> {
        self.check_open()?;
        if let Some(source) = &self.source_audit {
            let source = source.upgrade().ok_or_else(shutdown_error)?;
            return source
                .read(
                    &self.requester,
                    destination,
                    data_attributes,
                    bacnet_client::EndpointReadRequest::Range(request, validation),
                )
                .await?
                .into_range();
        }
        self.requester
            .read_range_with_destination(destination, data_attributes, request, validation)
            .await
    }

    /// Read one page of `log`'s Log_Buffer from `cursor`, as
    /// [`bacnet_client::log_reader::read_log_page`] does; each request is
    /// audited like any other read, and paced by the session's
    /// [`min_request_interval_ms`](crate::session::SessionConfig::min_request_interval_ms)
    /// as a paced `BACnetClient`'s are.
    pub async fn read_log_page(
        &self,
        destination_mac: &[u8],
        log: ObjectIdentifier,
        cursor: bacnet_client::log_reader::LogCursor,
        page_size: u16,
    ) -> Result<bacnet_client::log_reader::LogPage, Error> {
        bacnet_client::log_reader::read_log_page(self, destination_mac, log, cursor, page_size)
            .await
    }

    /// [`read_log_page`](Self::read_log_page) to an explicit destination,
    /// with pass-through attributes: every request of the page, the count
    /// reads included, goes to `destination` as
    /// [`read_property_with_destination`](Self::read_property_with_destination)
    /// and [`read_range_with_destination`](Self::read_range_with_destination)
    /// send it (#2612).
    pub async fn read_log_page_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        log: ObjectIdentifier,
        cursor: bacnet_client::log_reader::LogCursor,
        page_size: u16,
    ) -> Result<bacnet_client::log_reader::LogPage, Error> {
        let to = Destined {
            handle: self,
            destination,
            data_attributes,
        };
        // The reader hands each request the MAC it was given; `Destined`
        // sends every one to its own destination instead.
        bacnet_client::log_reader::read_log_page(&to, &[], log, cursor, page_size).await
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
    /// Owner-only seam (`pub(crate)`): the session [`Drop`]
    /// path currently closes the inner requester directly; this stays for
    /// owner-driven shutdown without exposing lifecycle on the handle.
    #[allow(dead_code)]
    pub(crate) fn close_for_owner(&self) {
        self.requester.close();
    }
}

impl bacnet_client::log_reader::LogRequester for ClientRoleHandle {
    async fn read_unsigned(
        &self,
        mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        let ack = self.read_property(mac, log, property, None).await?;
        bacnet_client::log_reader::unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        mac: &[u8],
        request: &bacnet_services::read_range::ReadRangeRequest,
    ) -> Result<bacnet_services::read_range::ReadRangeReply, Error> {
        self.read_range_with(
            mac,
            request,
            bacnet_services::read_range::ReadRangeValidation::Lenient,
        )
        .await
    }
}

/// A paged log read's requests to one explicit destination, fixed for the
/// page: the MAC the reader passes is unused.
struct Destined<'a> {
    handle: &'a ClientRoleHandle,
    destination: EndpointApduDestination,
    data_attributes: Vec<DataAttribute>,
}

impl bacnet_client::log_reader::LogRequester for Destined<'_> {
    async fn read_unsigned(
        &self,
        _mac: &[u8],
        log: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        let ack = self
            .handle
            .read_property_with_destination(
                self.destination.clone(),
                self.data_attributes.clone(),
                log,
                property,
                None,
            )
            .await?;
        bacnet_client::log_reader::unsigned_value(&ack.property_value)
    }

    async fn read_range_lenient(
        &self,
        _mac: &[u8],
        request: &bacnet_services::read_range::ReadRangeRequest,
    ) -> Result<bacnet_services::read_range::ReadRangeReply, Error> {
        self.handle
            .read_range_with_destination(
                self.destination.clone(),
                self.data_attributes.clone(),
                request.clone(),
                bacnet_services::read_range::ReadRangeValidation::Lenient,
            )
            .await
    }
}

fn assert_send_sync<T: Send + Sync>() {}

const _: fn() = || {
    assert_send_sync::<ClientRoleHandle>();
};

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
/// [`OutboundTransactionCoordinator::admit_from_source`] per received
/// terminal APDU, with `local_network`, the session's known network number.
///
/// Returns the coordinator outcome without releasing the lease; the selected
/// role completes it exactly once via its pre-admitted path, keyed to the
/// peer the admission matched.
#[doc(hidden)]
pub fn admit_once(
    coordinator: &OutboundTransactionCoordinator,
    received: &ReceivedApdu,
    local_network: Option<u16>,
    apdu: &Apdu,
) -> Result<AdmissionOutcome, bacnet_endpoint_core::coordinator::CoordinatorError> {
    coordinator.admit_from_source(
        received.source_mac.as_slice(),
        received.source_network.as_ref(),
        local_network,
        apdu,
    )
}

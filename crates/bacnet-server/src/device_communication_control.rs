//! Server policy types for DeviceCommunicationControl (Clause 16.1, #1964).

use crate::server::DccState;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;

/// Network identity and the requested change supplied to
/// [`ServerPolicy::authorize_device_communication_control`](crate::ServerPolicy::authorize_device_communication_control).
///
/// The policy is asked only once the password, the static
/// [`DccPolicy`](crate::server::DccPolicy) and the source restriction pass,
/// and before the disable rate limit is charged. The password is never part of
/// the context. `Debug` is redacted: address lengths and whether a principal
/// is present, never addresses or certificate data.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceCommunicationControlAuthorizationContext {
    /// Immediate data-link peer (or router) address.
    pub source_mac: MacAddr,
    /// Originating NPDU source when the request was routed.
    pub source_network: Option<NpduAddress>,
    /// Immutable ingress snapshot; the source of the principal, if any.
    pub provenance: TransportProvenance,
    /// Confirmed-request invoke identifier.
    pub invoke_id: u8,
    /// The state the request asks for. The deprecated DISABLE never gets
    /// this far: the server refuses it first.
    pub requested_state: DccState,
    /// The requested duration in minutes; `None` asks for no time limit.
    pub duration_minutes: Option<u16>,
}

impl DeviceCommunicationControlAuthorizationContext {
    /// The principal of the peer that originated the request: the verified
    /// leaf certificate of a direct BACnet/SC connection, when the request
    /// carries no routed source. `None` for other ingress, hub-relayed and
    /// routed requests included (see [`crate::policy`]).
    pub fn principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        crate::policy::originating_principal(&self.provenance, self.source_network.as_ref())
    }

    /// The principal of the direct BACnet/SC peer that handed this request
    /// over, a router included when the request was routed; `None` off a
    /// verified direct SC connection. Trusting it trusts everything that
    /// peer forwards.
    pub fn forwarding_peer_principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        self.provenance.principal()
    }

    crate::policy::certificate_accessors!();

    /// The immediate direct-SC hop's verified leaf and connection incarnation:
    /// for a routed request, the router that forwarded it. Absent off a
    /// verified direct SC connection. Separate from claimed addresses.
    pub fn direct_sc_identity(&self) -> Option<bacnet_transport::port::DirectScIdentity> {
        self.provenance.direct_sc_identity()
    }
}

impl std::fmt::Debug for DeviceCommunicationControlAuthorizationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCommunicationControlAuthorizationContext")
            .field("source_mac_len", &self.source_mac.len())
            .field(
                "source_network",
                &self
                    .source_network
                    .as_ref()
                    .map(|s| (s.network, s.mac_address.len())),
            )
            .field("provenance", &self.provenance)
            .field("principal", &self.principal().is_some())
            .field("invoke_id", &self.invoke_id)
            .field("requested_state", &self.requested_state)
            .field("duration_minutes", &self.duration_minutes)
            .finish()
    }
}

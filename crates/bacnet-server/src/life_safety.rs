//! Server policy types for LifeSafetyOperation (Clause 13.13).

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::MacAddr;

/// Network and request identity supplied to
/// [`ServerPolicy::authorize_life_safety_operation`](crate::ServerPolicy::authorize_life_safety_operation).
///
/// `requesting_source` inside [`request`](Self::request) is peer-controlled
/// descriptive text. It is not an authenticated operator identity. For routed
/// traffic, policy should normally consider `source_network` rather than
/// treating the immediate router in `source_mac` as the requester.
#[derive(Clone, PartialEq, Eq)]
pub struct LifeSafetyOperationAuthorizationContext {
    /// Immediate data-link peer (or router) address.
    pub source_mac: MacAddr,
    /// Originating NPDU source when the request was routed.
    pub source_network: Option<NpduAddress>,
    /// Immutable ingress snapshot, retained after the admitting connection closes.
    pub provenance: bacnet_transport::port::TransportProvenance,
    /// Confirmed-request invoke identifier.
    pub invoke_id: u8,
    /// Decoded service request.
    pub request: LifeSafetyOperationRequest,
}

impl LifeSafetyOperationAuthorizationContext {
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
impl std::fmt::Debug for LifeSafetyOperationAuthorizationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LifeSafetyOperationAuthorizationContext")
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
            .finish_non_exhaustive()
    }
}

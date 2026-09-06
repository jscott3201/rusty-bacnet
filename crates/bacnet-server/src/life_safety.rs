//! Server policy types for LifeSafetyOperation (Clause 13.13).

use std::sync::Arc;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::MacAddr;

/// Network and request identity supplied to a LifeSafetyOperation authorizer.
///
/// `requesting_source` inside [`request`](Self::request) is peer-controlled
/// descriptive text. It is not an authenticated operator identity. For routed
/// traffic, policy should normally consider `source_network` rather than
/// treating the immediate router in `source_mac` as the requester.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifeSafetyOperationAuthorizationContext {
    /// Immediate data-link peer (or router) address.
    pub source_mac: MacAddr,
    /// Originating NPDU source when the request was routed.
    pub source_network: Option<NpduAddress>,
    /// Confirmed-request invoke identifier.
    pub invoke_id: u8,
    /// Decoded service request.
    pub request: LifeSafetyOperationRequest,
}

/// Thread-safe authorization callback for LifeSafetyOperation.
///
/// The callback must be fast and nonblocking. Returning `false`, panicking, or
/// omitting the callback denies the request before object mutation.
pub type LifeSafetyOperationAuthorizer =
    Arc<dyn Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync>;

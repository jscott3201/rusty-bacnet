//! Opt-in authorization for the server's ten covered confirmed mutation services.
//!
//! This is local application policy, not authentication. DCC/ReinitializeDevice,
//! LifeSafety/Audit, unconfirmed services, reads, and queries retain their own
//! behavior. Direct handler calls and trusted local writes are not gated here.

use std::sync::Arc;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
use bacnet_services::file::AtomicWriteFileRequest;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest};
use bacnet_services::wpm::WritePropertyAttempt;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::MacAddr;

/// Decoded mutation target and parameters presented to local policy.
#[derive(Debug, Clone, PartialEq)]
pub enum MutationTarget {
    /// One object/property write, including index, raw value, and priority.
    WriteProperty(WritePropertyRequest),
    /// The current complete WPM element, not the whole request or a future suffix.
    WritePropertyMultiple(WritePropertyAttempt),
    /// Requested object type/identifier and initial property values.
    CreateObject(CreateObjectRequest),
    /// Object to delete.
    DeleteObject(DeleteObjectRequest),
    /// Object/property list and all elements to add.
    AddListElement(ListElementRequest),
    /// Object/property list and all elements to remove.
    RemoveListElement(ListElementRequest),
    /// File identifier, access method, write position, and payload.
    AtomicWriteFile(AtomicWriteFileRequest),
    /// Object subscription, renewal, or cancellation and claimed process ID.
    SubscribeCov(SubscribeCOVRequest),
    /// Property subscription, renewal, or cancellation and claimed process ID.
    SubscribeCovProperty(SubscribeCOVPropertyRequest),
    /// Entire decoded multi-property subscription request, authorized once.
    SubscribeCovPropertyMultiple(SubscribeCOVPropertyMultipleRequest),
}

/// Claimed source identity and decoded target supplied to mutation policy.
///
/// Neither address nor a subscription's process ID authenticates an operator.
/// `source_mac` identifies the immediate peer (often a router); `source_network`
/// is the peer-claimed routed origin, not a verified identity.
#[derive(Debug, Clone, PartialEq)]
pub struct MutationAuthorizationContext {
    /// Immediate data-link peer address.
    pub source_mac: MacAddr,
    /// Claimed originating NPDU address, when present.
    pub source_network: Option<NpduAddress>,
    /// Confirmed-request invoke identifier.
    pub invoke_id: u8,
    /// Outer confirmed service identity.
    pub service_choice: ConfirmedServiceChoice,
    /// Decoded target and parameters; current element for WPM.
    pub target: MutationTarget,
}

/// Fast, nonblocking, side-effect-free mutation authorization callback.
///
/// **Default-allow:** `None` preserves existing behavior, deliberately unlike
/// AuditNotification/LifeSafetyOperation's fail-closed absence. Returning `false`
/// or panicking denies with SERVICES / SERVICE_REQUEST_DENIED before mutation.
/// DCC prechecks and request admission precede this callback.
///
/// WPM invokes policy per decoded, validated element in wire order, immediately
/// before its write. Denial or a malformed suffix leaves the authorized prefix
/// committed. Other covered services decode their complete service request and
/// invoke policy once, including multi-element list/subscription requests.
/// Callbacks can run concurrently and WPM holds the database write lock: do not
/// block, reenter the server, or perform side effects from a callback.
pub type MutationAuthorizer = Arc<dyn Fn(&MutationAuthorizationContext) -> bool + Send + Sync>;

//! Opt-in authorization for the server's ten covered confirmed mutation services.
//!
//! This is local application policy, not authentication. DCC/ReinitializeDevice,
//! LifeSafety/Audit, unconfirmed services, reads, and queries retain their own
//! behavior. Direct handler calls and trusted local writes are not gated here.

use std::sync::atomic::{AtomicU64, Ordering};
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

/// Local authorization mode for the ten services represented by [`MutationTarget`].
///
/// SC mTLS channel/peer authentication is **not service authorization**. Identities
/// here are claimed link/routed addresses, never certificate principals; distinguishing
/// SC certificate principals is out of scope because none reaches this layer.
/// DCC, admission, decoding and WPM element validation retain their existing precedence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MutationPolicy {
    /// Preserve default-allow behavior: an absent authorizer allows; an installed
    /// authorizer must allow each decision.
    #[default]
    Permissive,
    /// Deny every covered decision, even with an allow-all authorizer installed.
    /// The authorizer is not called. Denials use SERVICES / SERVICE_REQUEST_DENIED.
    DenyAll,
}

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
/// SC mTLS authenticates the channel/peer, not service authorization; neither
/// address is a certificate principal.
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
/// Under [`MutationPolicy::Permissive`], **default-allow:** `None` preserves existing
/// behavior, deliberately unlike
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

/// Saturating lifetime decision totals for one covered service.
/// These count authorization decisions, not successful mutations or response delivery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MutationServiceCounters {
    /// Allowed by absent or approving authorizer in permissive mode.
    pub allow_total: u64,
    /// All denials: authorizer refusal/panic or deny-all policy.
    pub deny_total: u64,
    /// Deny-all policy denials, a subset of `deny_total`; no callback was called.
    pub policy_deny_total: u64,
}

/// Fixed-shape local telemetry, not a durable audit log; no source history is retained.
/// New servers start at zero. Each field is independently sampled and saturates at
/// `u64::MAX`, so snapshots are not atomic aggregates. Counters never affect policy.
/// WPM counts each element reaching its gate, not requests or an unvisited suffix.
/// Pre-gate failures, duplicates and DCC drops do not count. In permissive mode an
/// absent authorizer allows without decoding, so a later handler failure still counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MutationDecisionCounters {
    /// WriteProperty decisions.
    pub write_property: MutationServiceCounters,
    /// WritePropertyMultiple element decisions.
    pub write_property_multiple: MutationServiceCounters,
    /// CreateObject decisions.
    pub create_object: MutationServiceCounters,
    /// DeleteObject decisions.
    pub delete_object: MutationServiceCounters,
    /// AddListElement decisions.
    pub add_list_element: MutationServiceCounters,
    /// RemoveListElement decisions.
    pub remove_list_element: MutationServiceCounters,
    /// AtomicWriteFile decisions.
    pub atomic_write_file: MutationServiceCounters,
    /// SubscribeCOV decisions, including cancellation and renewal.
    pub subscribe_cov: MutationServiceCounters,
    /// SubscribeCOVProperty decisions, including cancellation and renewal.
    pub subscribe_cov_property: MutationServiceCounters,
    /// SubscribeCOVPropertyMultiple whole-request decisions.
    pub subscribe_cov_property_multiple: MutationServiceCounters,
}

#[derive(Clone, Copy)]
pub(crate) enum MutationDecision {
    Allow,
    Deny,
    PolicyDeny,
}

#[derive(Default)]
pub(crate) struct MutationDecisions([[AtomicU64; 3]; 10]);

impl MutationDecisions {
    pub(crate) fn snapshot(&self) -> MutationDecisionCounters {
        let [write_property, write_property_multiple, create_object, delete_object, add_list_element, remove_list_element, atomic_write_file, subscribe_cov, subscribe_cov_property, subscribe_cov_property_multiple] =
            self.0.each_ref().map(|row| {
                let [allow_total, deny_total, policy_deny_total] =
                    row.each_ref().map(|n| n.load(Ordering::Relaxed));
                MutationServiceCounters {
                    allow_total,
                    deny_total,
                    policy_deny_total,
                }
            });
        MutationDecisionCounters {
            write_property,
            write_property_multiple,
            create_object,
            delete_object,
            add_list_element,
            remove_list_element,
            atomic_write_file,
            subscribe_cov,
            subscribe_cov_property,
            subscribe_cov_property_multiple,
        }
    }

    pub(crate) fn record(&self, service: ConfirmedServiceChoice, decision: MutationDecision) {
        let index = match service {
            ConfirmedServiceChoice::WRITE_PROPERTY => 0,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => 1,
            ConfirmedServiceChoice::CREATE_OBJECT => 2,
            ConfirmedServiceChoice::DELETE_OBJECT => 3,
            ConfirmedServiceChoice::ADD_LIST_ELEMENT => 4,
            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT => 5,
            ConfirmedServiceChoice::ATOMIC_WRITE_FILE => 6,
            ConfirmedServiceChoice::SUBSCRIBE_COV => 7,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY => 8,
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE => 9,
            _ => return,
        };
        let increment = |column: usize| {
            let _ = self.0[index][column].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
        };
        match decision {
            MutationDecision::Allow => increment(0),
            MutationDecision::Deny => increment(1),
            MutationDecision::PolicyDeny => {
                increment(1);
                increment(2);
            }
        }
    }
}

#[cfg(test)]
#[path = "mutation_counter_tests.rs"]
mod counter_tests;

//! Opt-in authorization for the server's ten covered confirmed mutation
//! services and the Channel writes an inbound WriteGroup makes (#1319).
//!
//! This is local application policy, not authentication. The decision itself
//! is [`ServerPolicy::authorize_mutation`](crate::ServerPolicy::authorize_mutation);
//! this module holds its context, the [`MutationPolicy`] hardening switch and
//! the decision counters. DCC/ReinitializeDevice, the other unconfirmed
//! services, reads, and queries keep their own behavior. Direct handler calls
//! and trusted local writes are not gated here.

use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
use bacnet_services::file::AtomicWriteFileRequest;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest};
use bacnet_services::wpm::WritePropertyAttempt;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::{ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

/// Channel/relay scope derived from ingress provenance, mirroring RB-09
/// `ControlTrust`. Scope only, never leaf identity: a verified SC ingress
/// asserts the channel/relay validation, not that a claimed SNET/SADR leaf
/// is the authenticated peer. Provenance alone never authorizes; only the
/// policy does (no static allowlist).
///
/// Direct ingress exposes its verified leaf as
/// [`MutationAuthorizationContext::principal`]; this enum remains scope-only. The SC VMAC is payload-claimed inside the TLS
/// channel and is not bound to the operational certificate. An unknown origin (including a
/// hub-mediated unknown leaf, which arrives [`TransportProvenance::unverified`])
/// never satisfies a baseline-only allow rule; receive-permission (e.g. an
/// accepted COV subscription or audit receipt) is never write-permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationTrust {
    /// Unverified legacy origin, including hub-mediated unknown leaves.
    Unverified,
    /// Authenticated immediate SC-TLS channel; leaf identity is separate.
    VerifiedChannel,
    /// Independently validated SC-hub relayed origin (post source admission;
    /// the hub peer is not the leaf).
    VerifiedRelay,
}

impl MutationTrust {
    /// Derive the channel/relay scope from one reassembled ingress snapshot.
    pub fn from_provenance(provenance: TransportProvenance) -> Self {
        if provenance.is_direct_peer() || provenance.is_hub_channel() {
            Self::VerifiedChannel
        } else if provenance.is_relayed_origin() {
            Self::VerifiedRelay
        } else {
            Self::Unverified
        }
    }
}

/// Local authorization mode for the services represented by [`MutationTarget`]:
/// ten confirmed services and inbound WriteGroup.
///
/// SC mTLS channel/peer authentication is **not service authorization**. Identities
/// include claimed link/routed addresses separately from the direct-SC
/// [`principal`](MutationAuthorizationContext::principal) an installed
/// [`ServerPolicy`](crate::ServerPolicy) sees.
/// DCC, admission, decoding and WPM element validation retain their existing precedence.
///
/// An inbound WriteGroup is decided once per Channel write it would make
/// ([`MutationTarget::WriteGroup`]): `DenyAll` denies each one, and a policy
/// may allow some Channels and not others (#1319). A denied write is
/// skipped silently, since nothing answers an unconfirmed request, and counted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MutationPolicy {
    /// Preserve default-allow behavior: without a policy every decision is
    /// allowed; an installed policy must allow each one.
    #[default]
    Permissive,
    /// Deny every covered decision, even with an allow-all policy installed.
    /// The policy is not asked. Denials use SERVICES / SERVICE_REQUEST_DENIED.
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
    /// One Channel Present_Value write an inbound WriteGroup would make,
    /// decided per Channel.
    WriteGroup(WriteGroupTarget),
}

/// One Channel write an inbound WriteGroup would make: the Channel, and the
/// change-list entry and priority that reach it (Clause 15.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteGroupTarget {
    /// The Channel whose Present_Value would be written.
    pub channel: ObjectIdentifier,
    /// The request's control group, one the Channel's Control_Groups holds.
    pub group_number: u32,
    /// The change-list entry's channel number, the Channel's Channel_Number.
    pub channel_number: u16,
    /// The priority of the write: the entry's own, or else the request's.
    pub priority: u8,
    /// The entry's BACnetChannelValue, encoded as the request carried it.
    pub value: Vec<u8>,
    /// Whether the request asks for the Channel's execution delays to be
    /// skipped.
    pub inhibit_delay: bool,
}

/// The service a mutation decision is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationService {
    /// A confirmed service, which the server answers.
    Confirmed(ConfirmedServiceChoice),
    /// An unconfirmed service: nothing goes back, a denial included.
    Unconfirmed(UnconfirmedServiceChoice),
}

impl From<ConfirmedServiceChoice> for MutationService {
    fn from(service: ConfirmedServiceChoice) -> Self {
        Self::Confirmed(service)
    }
}

impl MutationTarget {
    /// Stable service-kind label for redacted diagnostics (no parameters).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::WriteProperty(_) => "write-property",
            Self::WritePropertyMultiple(_) => "write-property-multiple",
            Self::CreateObject(_) => "create-object",
            Self::DeleteObject(_) => "delete-object",
            Self::AddListElement(_) => "add-list-element",
            Self::RemoveListElement(_) => "remove-list-element",
            Self::AtomicWriteFile(_) => "atomic-write-file",
            Self::SubscribeCov(_) => "subscribe-cov",
            Self::SubscribeCovProperty(_) => "subscribe-cov-property",
            Self::SubscribeCovPropertyMultiple(_) => "subscribe-cov-property-multiple",
            Self::WriteGroup(_) => "write-group",
        }
    }
}

/// Claimed source identity, verified ingress scope, and decoded target
/// supplied to mutation policy.
///
/// Neither address nor a subscription's process ID authenticates an operator.
/// `source_mac` identifies the immediate peer (often a router); `source_network`
/// is the peer-claimed routed origin, not a verified identity.
/// SC mTLS authenticates the channel/peer, not service authorization; neither
/// address is a certificate principal. [`principal`](Self::principal) is, and
/// only for a verified direct SC connection.
///
/// `provenance` is the reassembled ingress snapshot threaded from dispatch
/// (segmentation mismatches already fail closed at reassembly, so every
/// element of one request — including each WPM element — observes the same
/// snapshot). `trust` is the channel/relay scope derived from it, never leaf
/// identity. Debug is redacted by construction: address lengths, whether a
/// principal is present and the target kind only, never MAC bytes, the
/// fingerprint, property values, file payloads, or other decoded inputs.
#[derive(Clone, PartialEq)]
pub struct MutationAuthorizationContext {
    /// Immediate data-link peer address.
    pub source_mac: MacAddr,
    /// Claimed originating NPDU address, when present.
    pub source_network: Option<NpduAddress>,
    /// Honest transport + origin provenance for this ingress snapshot.
    pub provenance: TransportProvenance,
    /// Channel/relay scope derived from `provenance`; never leaf identity.
    pub trust: MutationTrust,
    /// A confirmed request's invoke identifier; `None` for an unconfirmed
    /// request, which carries none.
    pub invoke_id: Option<u8>,
    /// The service that asks for the mutation.
    pub service_choice: MutationService,
    /// Decoded target and parameters; current element for WPM, current
    /// Channel for WriteGroup.
    pub target: MutationTarget,
}

impl MutationAuthorizationContext {
    /// The principal of the peer that originated the request: the verified
    /// leaf certificate of a direct BACnet/SC connection, when the request
    /// carries no routed source. `None` for other ingress, hub-relayed and
    /// routed requests included (see [`crate::policy`]).
    pub fn principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        crate::policy::originating_principal(self.provenance, self.source_network.as_ref())
    }

    /// The principal of the direct BACnet/SC peer that handed this request
    /// over, a router included when the request was routed; `None` off a
    /// verified direct SC connection. Trusting it trusts everything that
    /// peer forwards.
    pub fn forwarding_peer_principal(&self) -> Option<bacnet_transport::port::ScPrincipal> {
        self.provenance.principal()
    }

    /// The immediate direct-SC hop's verified leaf and connection incarnation:
    /// for a routed request, the router that forwarded it. Absent off a
    /// verified direct SC connection. Separate from claimed addresses.
    pub fn direct_sc_identity(&self) -> Option<bacnet_transport::port::DirectScIdentity> {
        self.provenance.direct_sc_identity()
    }
}

impl std::fmt::Debug for MutationAuthorizationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationAuthorizationContext")
            .field("source_mac_len", &self.source_mac.len())
            .field(
                "source_network",
                &self
                    .source_network
                    .as_ref()
                    .map(|source| (source.network, source.mac_address.len())),
            )
            .field("provenance", &self.provenance)
            .field("principal", &self.principal().is_some())
            .field("trust", &self.trust)
            .field("invoke_id", &self.invoke_id)
            .field("service_choice", &self.service_choice)
            .field("target_kind", &self.target.kind())
            .finish_non_exhaustive()
    }
}

/// The shared endpoint's gate for writes to its local Device, given the same
/// redacted context [`ServerPolicy::authorize_mutation`](crate::ServerPolicy::authorize_mutation)
/// sees, under the same calling rules. A full server uses its
/// [`ServerPolicy`](crate::ServerPolicy) instead.
pub type DeviceWriteAuthorizer =
    std::sync::Arc<dyn Fn(&MutationAuthorizationContext) -> bool + Send + Sync>;

/// Saturating lifetime decision totals for one covered service.
/// These count authorization decisions, not successful mutations or response delivery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MutationServiceCounters {
    /// Allowed, with no policy or an approving one, in permissive mode.
    pub allow_total: u64,
    /// All denials: policy refusal/panic or deny-all mode.
    pub deny_total: u64,
    /// Deny-all mode denials, a subset of `deny_total`; no policy was asked.
    pub policy_deny_total: u64,
}

/// Fixed-shape local telemetry, not a durable audit log; no source history is retained.
/// New servers start at zero. Each field is independently sampled and saturates at
/// `u64::MAX`, so snapshots are not atomic aggregates. Counters never affect policy.
/// WPM counts each element reaching its gate, not requests or an unvisited suffix,
/// even one the policy was asked about ahead of the handler (#1321).
/// Pre-gate failures and duplicates do not count. In permissive mode without
/// a policy, a decision allows without decoding, so a later handler failure still counts.
/// An inbound WriteGroup counts once per Channel write it would make.
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
    /// Decisions on the Channel writes inbound WriteGroups would make.
    pub write_group: MutationServiceCounters,
}

#[derive(Clone, Copy)]
pub(crate) enum MutationDecision {
    Allow,
    Deny,
    PolicyDeny,
}

#[derive(Default)]
pub(crate) struct MutationDecisions([[AtomicU64; 3]; 11]);

impl MutationDecisions {
    pub(crate) fn snapshot(&self) -> MutationDecisionCounters {
        let [write_property, write_property_multiple, create_object, delete_object, add_list_element, remove_list_element, atomic_write_file, subscribe_cov, subscribe_cov_property, subscribe_cov_property_multiple, write_group] =
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
            write_group,
        }
    }

    pub(crate) fn record(&self, service: impl Into<MutationService>, decision: MutationDecision) {
        let service = match service.into() {
            MutationService::Confirmed(service) => service,
            MutationService::Unconfirmed(UnconfirmedServiceChoice::WRITE_GROUP) => {
                return self.increment(10, decision);
            }
            MutationService::Unconfirmed(_) => return,
        };
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
        self.increment(index, decision);
    }

    fn increment(&self, index: usize, decision: MutationDecision) {
        let increment = |column: usize| {
            #[allow(deprecated, reason = "try_update needs Rust 1.95; the MSRV is 1.93")]
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

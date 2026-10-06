//! TransportPort trait for BACnet data-link transport abstraction.
//!
//! MAC addresses are opaque byte slices whose format depends on the transport:
//! - BACnet/IP (Annex J): 6 bytes (4-byte IPv4 + 2-byte port, big-endian)
//! - BACnet/Ethernet (Clause 7): 6 bytes (IEEE 802 MAC)
//! - MS/TP (Clause 9): 1 byte (station address 0-254)

pub use crate::direct_response::{DirectResponse, DirectResponseScope};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// An owned copy of a link's group-destination rule
/// ([`TransportPort::is_group_destination`]), for a sender that checks a
/// destination away from the task that owns the transport, before it takes
/// any state for the send (#1479). Clones share one rule.
#[derive(Clone, Default)]
pub struct GroupDestinations(Option<Arc<GroupRule>>);

/// The predicate inside a [`GroupDestinations`].
type GroupRule = dyn Fn(&[u8]) -> bool + Send + Sync;

impl GroupDestinations {
    /// The rule `is_group` gives.
    pub fn new(is_group: impl Fn(&[u8]) -> bool + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(is_group)))
    }

    /// No rule: nothing matches. A sender holding this leaves the check to the
    /// send path, which asks the transport itself.
    pub fn unknown() -> Self {
        Self(None)
    }

    /// Whether `mac` is a group destination under this rule.
    pub fn contains(&self, mac: &[u8]) -> bool {
        self.0.as_ref().is_some_and(|is_group| is_group(mac))
    }
}

impl std::fmt::Debug for GroupDestinations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("GroupDestinations")
            .field(&if self.0.is_some() { "rule" } else { "unknown" })
            .finish()
    }
}

/// Data-link attributes that accompany an NPDU.
///
/// BACnet/SC maps these to Annex AB Data Options. Transports that cannot
/// carry data attributes ignore them on send and report an empty list on
/// receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataAttribute {
    /// Attribute/header option type. BACnet/SC uses values 1..31.
    pub option_type: u8,
    /// Whether the final consumer must understand this attribute.
    pub must_understand: bool,
    /// Attribute payload bytes, if any.
    pub data: Vec<u8>,
}

/// Verified immediate direct-SC peer and connection incarnation.
///
/// The fingerprint is SHA-256 of the exact verified leaf DER, not of the
/// claimed VMAC, UUID or routed address. Certificate renewal changes the
/// fingerprint; reconnecting with the same leaf changes the incarnation.
/// This sealed snapshot survives queueing and connection retirement. It is
/// evidence for application policy, not authorization or a response route.
///
/// Verified identities cannot be constructed by downstream callers:
/// ```compile_fail
/// use bacnet_transport::port::DirectScIdentity;
/// let forged = DirectScIdentity { leaf_sha256: [0; 32], incarnation: 1 };
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DirectScIdentity {
    leaf_sha256: [u8; 32],
    incarnation: u64,
}
impl DirectScIdentity {
    /// Exact verified leaf-DER SHA-256, suitable for an installation's pins.
    pub fn leaf_sha256(self) -> [u8; 32] {
        self.leaf_sha256
    }

    /// Opaque process-lifetime connection identifier. Never persist or reuse it
    /// as an identity across processes; no ordering contract is exposed.
    pub fn incarnation(self) -> u64 {
        self.incarnation
    }

    // Only the sc-tls direct accept and dial paths mint verified identities.
    #[cfg(any(test, feature = "sc-tls"))]
    pub(crate) fn verified(leaf_sha256: [u8; 32], incarnation: u64) -> Self {
        Self {
            leaf_sha256,
            incarnation,
        }
    }
}
impl std::fmt::Debug for DirectScIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectScIdentity").finish_non_exhaustive()
    }
}

/// Sealed transport provenance, copied unchanged through queued ingress.
///
/// Direct SC provenance records the verified immediate TLS leaf and the
/// committed connection incarnation after Connect acceptance. Complete work
/// already admitted may finish under this original snapshot after close or
/// replacement. Claimed VMAC/UUID/SNET/SADR remain separate claims; there is
/// no certificate-to-claim binding or response-channel confinement here.
///
/// Relayed provenance asserts the authenticated Hub channel and admitted
/// origin-field shape, never an end-to-end leaf principal. Hub admission's
/// own TLS-client channel has a separate scope-only assertion, before Connect
/// acceptance, and cannot mint a downstream direct identity.
///
/// Plain transports and test doubles use [`Self::unverified`]. Verified
/// constructors are crate-private to trusted TLS/admission owners. A custom
/// transport that forwards a verified envelope is part of that trust boundary.
/// Data attributes remain wire values and cannot upgrade provenance.
///
/// ```compile_fail
/// use bacnet_transport::port::TransportProvenance;
/// let forged = TransportProvenance::verified_hub_channel();
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportProvenance {
    kind: ProvenanceKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ProvenanceKind {
    Unverified,
    // Constructed only by sc-tls direct ingress.
    #[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
    DirectPeer(DirectScIdentity),
    HubChannel,
    RelayedOrigin,
}

impl TransportProvenance {
    /// Unverified legacy origin; no authentication assertion.
    pub fn unverified() -> Self {
        Self {
            kind: ProvenanceKind::Unverified,
        }
    }

    /// Only built-in verified direct TLS ingress may mint this principal-bearing value.
    #[cfg(any(test, feature = "sc-tls"))]
    pub(crate) fn verified_direct_peer(identity: DirectScIdentity) -> Self {
        Self {
            kind: ProvenanceKind::DirectPeer(identity),
        }
    }

    /// Hub registration callback's own verified TLS-client channel, not a leaf
    /// identity for application ingress relayed through that Hub.
    #[cfg(any(test, feature = "sc-tls"))]
    pub(crate) fn verified_hub_channel() -> Self {
        Self {
            kind: ProvenanceKind::HubChannel,
        }
    }

    pub(crate) fn verified_relayed_origin() -> Self {
        Self {
            kind: ProvenanceKind::RelayedOrigin,
        }
    }

    /// Verified direct identity, absent on unverified and both Hub scopes.
    pub fn direct_sc_identity(self) -> Option<DirectScIdentity> {
        match self.kind {
            ProvenanceKind::DirectPeer(identity) => Some(identity),
            _ => None,
        }
    }

    /// True only for the unverified legacy variant.
    pub fn is_unverified(self) -> bool {
        self.kind == ProvenanceKind::Unverified
    }

    /// True only for direct-SC ingress with a verified identity.
    pub fn is_direct_peer(self) -> bool {
        matches!(self.kind, ProvenanceKind::DirectPeer(_))
    }

    /// True only for the Hub admission callback's own TLS-client channel.
    pub fn is_hub_channel(self) -> bool {
        self.kind == ProvenanceKind::HubChannel
    }

    /// True only for the verified relayed-origin variant.
    pub fn is_relayed_origin(self) -> bool {
        self.kind == ProvenanceKind::RelayedOrigin
    }

    /// True for any verified scope; this does not imply a direct principal.
    pub fn is_verified(self) -> bool {
        self.kind != ProvenanceKind::Unverified
    }

    /// Human-readable scope, with no identity material.
    pub fn scope(self) -> &'static str {
        match self.kind {
            ProvenanceKind::Unverified => "unverified legacy origin (no assertion)",
            ProvenanceKind::DirectPeer(_) => "authenticated immediate direct SC-TLS peer and connection incarnation",
            ProvenanceKind::HubChannel => "authenticated Hub admission TLS-client channel (scope only)",
            ProvenanceKind::RelayedOrigin => "independently validated SC-hub relayed origin (post source_admission; hub peer is not the leaf)",
        }
    }
}
impl Default for TransportProvenance {
    fn default() -> Self {
        Self::unverified()
    }
}
impl std::fmt::Debug for TransportProvenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self.kind {
            ProvenanceKind::Unverified => "unverified",
            ProvenanceKind::DirectPeer(_) => "verified-direct-peer",
            ProvenanceKind::HubChannel => "verified-hub-channel",
            ProvenanceKind::RelayedOrigin => "verified-relayed-origin",
        };
        f.debug_struct("TransportProvenance")
            .field("kind", &label)
            .finish_non_exhaustive()
    }
}

/// A received NPDU from the transport layer.
pub struct ReceivedNpdu {
    /// Raw NPDU bytes (NPDU header + APDU/network-message payload).
    pub npdu: Bytes,
    /// Source MAC address in transport-native format (claimed identity; see
    /// [`TransportProvenance`]. On SC this is the source VMAC, not a
    /// certificate principal).
    pub source_mac: MacAddr,
    /// Whether the NPDU arrived using a data-link multicast or broadcast destination.
    ///
    /// This is ingress provenance, not a conclusion about the NPDU's logical
    /// destination: routers may legitimately send a routed unicast over a
    /// group data-link destination.
    pub link_layer_group: bool,
    /// Optional data attributes carried by the data link.
    pub data_attributes: Vec<DataAttribute>,
    /// Honest transport + origin provenance, immutable by value. Cloning
    /// preserves the meaning; it never upgrades or downgrades trust.
    pub provenance: TransportProvenance,
    /// Optional sealed original-connection response capability. A verified
    /// direct server reply must fail closed if this is missing or mismatched.
    pub direct_response: Option<DirectResponse>,
    /// Optional reply channel for MS/TP DataExpectingReply frames.
    /// When present, the application layer should send the reply NPDU bytes
    /// through this channel instead of via normal send_unicast.
    pub reply_tx: Option<oneshot::Sender<Bytes>>,
}

impl ReceivedNpdu {
    /// Build an explicitly unverified legacy-origin envelope.
    ///
    /// The one-line helper for plain transports, `AnyTransport` delegation
    /// sites that synthesize, loopback, and test doubles: no data-link
    /// implementation is forced to pretend auth support.
    pub fn unverified(
        npdu: Bytes,
        source_mac: MacAddr,
        link_layer_group: bool,
        data_attributes: Vec<DataAttribute>,
        reply_tx: Option<oneshot::Sender<Bytes>>,
    ) -> Self {
        Self {
            npdu,
            source_mac,
            link_layer_group,
            data_attributes,
            provenance: TransportProvenance::unverified(),
            direct_response: None,
            reply_tx,
        }
    }
}

impl Clone for ReceivedNpdu {
    fn clone(&self) -> Self {
        Self {
            npdu: self.npdu.clone(),
            source_mac: self.source_mac.clone(),
            link_layer_group: self.link_layer_group,
            data_attributes: self.data_attributes.clone(),
            provenance: self.provenance,
            direct_response: self.direct_response.clone(),
            reply_tx: None, // oneshot::Sender is not Clone; clones lose the reply channel
        }
    }
}

impl std::fmt::Debug for ReceivedNpdu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redacted: MAC bytes and NPDU payload lengths only; no key material.
        // Follows the TimeSyncSourceRestriction finish_non_exhaustive pattern.
        f.debug_struct("ReceivedNpdu")
            .field("npdu_len", &self.npdu.len())
            .field("source_mac_len", &self.source_mac.len())
            .field("link_layer_group", &self.link_layer_group)
            .field("data_attributes", &self.data_attributes)
            .field("provenance", &self.provenance)
            .field("reply_tx", &self.reply_tx.as_ref().map(|_| "Some(Sender)"))
            .finish_non_exhaustive()
    }
}

/// What a B/IP transport reports for registration as a Network Port
/// ([`TransportPort::bip_port`]).
#[derive(Debug, Clone)]
pub struct BipPort {
    /// The IPv4 address and UDP port: configured before start, announced
    /// after.
    pub endpoint: std::net::SocketAddrV4,
    /// The B/IP mode, with what its Network Port properties need.
    pub mode: bacnet_types::bip_port::BipPortMode,
}

/// Trait for BACnet data-link transports.
///
/// Implementations handle the data-link framing (e.g., BVLL for BACnet/IP)
/// and expose a simple send/receive interface for NPDU bytes.
pub trait TransportPort: Send + Sync {
    /// Configured broadcast endpoint when this link uses IPv4 BACnet/IP.
    /// `None` means the link does not expose that capability. Wrappers must
    /// forward the underlying value. After `start`, the port is the actual
    /// bound UDP port, including when configuration requested port zero.
    fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        None
    }

    /// Opt in to the existing single-link nonrouter Network Number owner.
    ///
    /// Local What-Is-Network-Number may arrive by unicast or logical broadcast;
    /// Network-Number-Is must be classified as logical broadcast to teach state.
    /// `send_broadcast` must emit the reply on this same local link. This grants
    /// neither configured Network Port authority nor router/control-origin trust.
    /// Built-in B/IP (normal, BBMD or foreign), B/IPv6 (normal or foreign), SC,
    /// MS/TP, and Linux Ethernet opt in. Other
    /// links default to false until qualified. Wrappers must delegate.
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        false
    }

    /// Explicit capability for registration as one IPv4 Network Port, with
    /// the B/IP mode the port reports (Clause 12.56.21).
    ///
    /// Before start the endpoint is the configured interface and UDP port;
    /// after start it is the address the transport announces. In BBMD mode
    /// the tables appear once start creates them, and stay the transport's
    /// own live state. A transport with no single mode to report, such as a
    /// BBMD that also registers as a foreign device, returns None. Custom
    /// implementations asserting this capability must keep the mode they
    /// report through ownership. The default refuses registration; broadcast
    /// support alone is insufficient.
    fn bip_port(&self) -> Option<BipPort> {
        None
    }

    /// Retain selected Network Port protection through the last socket owner.
    /// Must be installed once before start, only on a transport that reports
    /// a [`bip_port`](Self::bip_port).
    /// Successful stop releases it after quiescence; abort/drop must retain it
    /// in every surviving socket worker. Custom implementors own this contract.
    #[doc(hidden)]
    fn retain_network_port_lease_internal(
        &mut self,
        _lease: std::sync::Arc<()>,
    ) -> Result<(), Error> {
        Err(Error::Encoding(
            "transport cannot retain a registered B/IP port".into(),
        ))
    }

    /// Start the transport. Returns a receiver for incoming NPDUs.
    ///
    /// The transport spawns a background receive task that decodes incoming
    /// frames and sends `ReceivedNpdu` through the returned channel.
    fn start(
        &mut self,
    ) -> impl std::future::Future<Output = Result<mpsc::Receiver<ReceivedNpdu>, Error>> + Send;

    /// Stop the transport and clean up resources.
    fn stop(&mut self) -> impl std::future::Future<Output = Result<(), Error>> + Send;

    /// Synchronously abort background work and release owned transport resources.
    ///
    /// This hook is intended for `Drop` paths where async [`Self::stop`] cannot
    /// be awaited. Implementations should not attempt graceful protocol shutdown
    /// here; graceful disconnects remain the responsibility of [`Self::stop`].
    fn abort(&mut self) {}

    /// Send NPDU bytes to a specific MAC address (unicast).
    fn send_unicast(
        &self,
        npdu: &[u8],
        mac: &[u8],
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;

    /// Send NPDU bytes with data attributes to a specific MAC address.
    ///
    /// Transports that cannot carry data attributes ignore them and send the
    /// NPDU normally. Attribute-capable transports should override this.
    fn send_unicast_with_data_attributes<'a>(
        &'a self,
        npdu: &'a [u8],
        mac: &'a [u8],
        _data_attributes: &'a [DataAttribute],
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send + 'a {
        async move { self.send_unicast(npdu, mac).await }
    }

    /// Broadcast NPDU bytes on the local network.
    fn send_broadcast(
        &self,
        npdu: &[u8],
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;

    /// Broadcast NPDU bytes with data attributes on the local network.
    ///
    /// Transports that cannot carry data attributes ignore them and broadcast
    /// the NPDU normally. Attribute-capable transports should override this.
    fn send_broadcast_with_data_attributes<'a>(
        &'a self,
        npdu: &'a [u8],
        _data_attributes: &'a [DataAttribute],
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send + 'a {
        async move { self.send_broadcast(npdu).await }
    }

    /// This transport's local MAC address.
    fn local_mac(&self) -> &[u8];

    /// Stable local APDU receive capacity, independent of peers and egress routes.
    ///
    /// Implementations and wrappers must report their actual local capacity.
    /// Negotiation, reconnects and failover must not change this declaration.
    fn local_receive_apdu_capacity(&self) -> u16;

    /// Current outgoing APDU limit for the transport's ordinary send path.
    /// This may change as a connection negotiates remote receive limits.
    fn egress_apdu_limit(&self) -> u16 {
        1476
    }

    /// Whether `mac` is this data link's broadcast address.
    ///
    /// A destination can spell a broadcast two ways: the network-layer form
    /// (a zero-length MAC) or the medium's literal broadcast MAC — Clause 6.3
    /// names `X'FFFFFFFFFFFF'` for Ethernet, `X'FF'` for MS/TP, and for BACnet/IP
    /// an address whose host bits are all set (a directed broadcast). Only the transport
    /// knows its own literal spelling, so senders that must not unicast to a
    /// broadcast (Clause 6.3 restricts broadcast to Unconfirmed-Request-PDUs)
    /// ask here. The default recognizes nothing, which leaves such a MAC
    /// treated as a unicast.
    fn is_broadcast_mac(&self, _mac: &[u8]) -> bool {
        false
    }

    /// Whether a send to `mac` with no DNET reaches a group of nodes rather
    /// than one: this link's broadcast MAC, or any other broadcast or
    /// multicast address the medium carries, such as a B/IP limited
    /// broadcast, an IPv6 multicast group or an Ethernet multicast MAC
    /// (#1493). Such a send is a local broadcast, which may carry only an
    /// Unconfirmed-Request (Clause 6.3), so a sender that takes a
    /// caller-chosen MAC refuses anything else to it (#1479). The server binds
    /// no device to one, and the confirmed requests it starts itself, to
    /// event recipients, Channel and Command targets, audit recipients and
    /// bound devices, never go to one (#1493). Its replies and COV
    /// notifications go to the source a request came from, and the server,
    /// the client and the endpoint ignore a confirmed request whose source
    /// MAC is one (#1504), so those don't go to a group on this link. A
    /// routed request's SADR names a node on another network, which this
    /// link can't judge: a reply to a group SADR is dropped only where the
    /// final router is a `BACnetRouter`, which delivers only an
    /// Unconfirmed-Request to a DADR that is a group on its delivery port.
    ///
    /// [`Self::is_broadcast_mac`] keeps its narrower meaning, this link's own
    /// broadcast, which routing and recipient checks rely on. The default is
    /// that answer.
    fn is_group_destination(&self, mac: &[u8]) -> bool {
        self.is_broadcast_mac(mac)
    }

    /// [`Self::is_group_destination`] as an owned rule that a sender can keep
    /// once the transport has moved into the task that runs it. The built-in
    /// transports give their rule; the default is
    /// [`GroupDestinations::unknown`], which leaves the check to the send path.
    fn group_destinations(&self) -> GroupDestinations {
        GroupDestinations::unknown()
    }
}

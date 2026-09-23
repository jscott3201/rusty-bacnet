//! TransportPort trait for BACnet data-link transport abstraction.
//!
//! MAC addresses are opaque byte slices whose format depends on the transport:
//! - BACnet/IP (Annex J): 6 bytes (4-byte IPv4 + 2-byte port, big-endian)
//! - BACnet/Ethernet (Clause 7): 6 bytes (IEEE 802 MAC)
//! - MS/TP (Clause 9): 1 byte (station address 0-254)

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

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

/// Honest transport + origin provenance (RB-07, internal contract).
///
/// Immutable, `Copy` by value. Three mutually exclusive assertions:
///
/// - [`TransportProvenance::unverified`] — Unverified legacy origin. No
///   transport or relay authentication is asserted. Covers B/IP, B/IPv6,
///   MS/TP, Ethernet, Loopback, test doubles, and any caller-supplied
///   [`TransportPort`] that does not implement SC-TLS validation. Claimed
///   SNET/SADR/MAC fields remain *claims*, never credentials. Compat mode
///   (RB-09 consumes this later; forwarding/learning/admission unchanged).
/// - Verified direct peer — Authenticated immediate transport peer, direct
///   SC-TLS post-handshake peer only. Scope: the TLS handshake verified the
///   peer's operational certificate and the Connect-Request/Accept exchange
///   completed on *this* connection, and `source_mac` is that peer's VMAC.
///   The VMAC itself is payload-claimed in the Connect-Request inside the
///   TLS channel and is not bound to the operational certificate.
///   It asserts nothing about a routed origin behind the peer (there is none
///   on a direct connection). Expires with the connection (idle timeout,
///   close, or disconnect); snapshots compare by value.
/// - Verified relayed origin — Independently validated relayed origin, SC hub
///   path post `source_admission` only. Scope: the hub TLS connection is
///   authenticated *and* the originating VMAC passed hub source admission
///   (present, non-reserved). The originating VMAC is a hub-relayed claim
///   checked only for presence/non-reserved, not certificate-bound. It asserts
///   the relay validated the origin field. An SC node's authenticated hub peer
///   is NOT the origin leaf:
///   the hub VMAC is never substituted for the leaf origin.
///
/// Only trusted transport/relay validation code may construct a verified
/// assertion. Verified constructors are `pub(crate)` and the only call sites
/// are `sc/*` (hub receive loop + `source_admission`) and `sc_tls/*`
/// (`direct_accept` post-handshake). Everything else — all plain transports,
/// [`crate::any::AnyTransport`] delegation (which preserves the inner
/// value), and test doubles — builds the unverified variant via
/// [`TransportProvenance::unverified`] / [`Default`].
///
/// Trust boundary: callers supplying an explicitly trusted local
/// [`TransportPort`] implementation are part of the trust boundary for the
/// values they construct (they must use `unverified()` unless they implement
/// equivalent TLS + admission validation). A caller-provided untrusted option
/// stays untrusted. No new wire field, cert format, or revision.
///
/// There is no public boolean named `authenticated` anywhere on the
/// [`DataAttribute`] input; `DataAttribute` stays a wire representation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportProvenance {
    kind: ProvenanceKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ProvenanceKind {
    Unverified,
    DirectPeer,
    RelayedOrigin,
}

impl TransportProvenance {
    /// Unverified legacy origin: the only constructor available outside
    /// trusted SC validation code. No authentication asserted.
    pub fn unverified() -> Self {
        Self {
            kind: ProvenanceKind::Unverified,
        }
    }

    /// Verified direct SC-TLS peer (trusted validation code only).
    ///
    /// Confined to `sc/*` and `sc_tls/*` call sites by discipline; `pub(crate)`
    /// keeps it internal to this crate while those modules own the handshake
    /// evidence. No other transport may call this.
    pub(crate) fn verified_direct_peer() -> Self {
        Self {
            kind: ProvenanceKind::DirectPeer,
        }
    }

    /// Verified SC-hub relayed origin (trusted validation code only).
    ///
    /// Confined to `sc/*` (hub receive loop + `source_admission`) by
    /// discipline; `pub(crate)` keeps it internal while that code owns the
    /// admission evidence. No other transport may call this.
    pub(crate) fn verified_relayed_origin() -> Self {
        Self {
            kind: ProvenanceKind::RelayedOrigin,
        }
    }

    /// True only for the unverified legacy variant.
    pub fn is_unverified(self) -> bool {
        self.kind == ProvenanceKind::Unverified
    }

    /// True only for the verified direct-peer variant.
    pub fn is_direct_peer(self) -> bool {
        self.kind == ProvenanceKind::DirectPeer
    }

    /// True only for the verified relayed-origin variant.
    pub fn is_relayed_origin(self) -> bool {
        self.kind == ProvenanceKind::RelayedOrigin
    }

    /// True for either verified variant.
    pub fn is_verified(self) -> bool {
        self.kind != ProvenanceKind::Unverified
    }

    /// Human-readable scope of this assertion (no identity material).
    pub fn scope(self) -> &'static str {
        match self.kind {
            ProvenanceKind::Unverified => "unverified legacy origin (no assertion)",
            ProvenanceKind::DirectPeer => {
                "authenticated immediate direct SC-TLS peer (post-handshake VMAC only)"
            }
            ProvenanceKind::RelayedOrigin => {
                "independently validated SC-hub relayed origin (post source_admission; hub peer is not the leaf)"
            }
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
        // Redacted by construction: kind label only, no key or identity bytes.
        let label = match self.kind {
            ProvenanceKind::Unverified => "unverified",
            ProvenanceKind::DirectPeer => "verified-direct-peer",
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

/// Trait for BACnet data-link transports.
///
/// Implementations handle the data-link framing (e.g., BVLL for BACnet/IP)
/// and expose a simple send/receive interface for NPDU bytes.
pub trait TransportPort: Send + Sync {
    /// Whether this link uses the six-octet IPv4-address/UDP-port BACnet/IP MAC.
    /// Wrappers must report their actual underlying link, never infer it from length.
    fn is_bip_ipv4(&self) -> bool {
        false
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

    /// Maximum APDU length this transport supports.
    /// BIP/SC: 1476 (default), MS/TP: 480.
    fn max_apdu_length(&self) -> u16 {
        1476
    }

    /// Whether `mac` is this data link's broadcast address.
    ///
    /// A destination can spell a broadcast two ways: the network-layer form
    /// (a zero-length MAC) or the medium's literal broadcast MAC — Clause 6.3
    /// names `X'FFFFFFFFFFFF'` for Ethernet, `X'FF'` for MS/TP, an IP address
    /// with all ones in the host portion for BACnet/IP. Only the transport
    /// knows its own literal spelling, so senders that must not unicast to a
    /// broadcast (Clause 6.3 restricts broadcast to Unconfirmed-Request-PDUs)
    /// ask here. The default recognizes nothing, which leaves such a MAC
    /// treated as a unicast.
    fn is_broadcast_mac(&self, _mac: &[u8]) -> bool {
        false
    }
}

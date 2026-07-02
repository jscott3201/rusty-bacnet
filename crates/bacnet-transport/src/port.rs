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

/// A received NPDU from the transport layer.
pub struct ReceivedNpdu {
    /// Raw NPDU bytes (NPDU header + APDU/network-message payload).
    pub npdu: Bytes,
    /// Source MAC address in transport-native format.
    pub source_mac: MacAddr,
    /// Optional data attributes carried by the data link.
    pub data_attributes: Vec<DataAttribute>,
    /// Optional reply channel for MS/TP DataExpectingReply frames.
    /// When present, the application layer should send the reply NPDU bytes
    /// through this channel instead of via normal send_unicast.
    pub reply_tx: Option<oneshot::Sender<Bytes>>,
}

impl Clone for ReceivedNpdu {
    fn clone(&self) -> Self {
        Self {
            npdu: self.npdu.clone(),
            source_mac: self.source_mac.clone(),
            data_attributes: self.data_attributes.clone(),
            reply_tx: None, // oneshot::Sender is not Clone; clones lose the reply channel
        }
    }
}

impl std::fmt::Debug for ReceivedNpdu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivedNpdu")
            .field("npdu", &self.npdu)
            .field("source_mac", &self.source_mac)
            .field("data_attributes", &self.data_attributes)
            .field("reply_tx", &self.reply_tx.as_ref().map(|_| "Some(Sender)"))
            .finish()
    }
}

/// Trait for BACnet data-link transports.
///
/// Implementations handle the data-link framing (e.g., BVLL for BACnet/IP)
/// and expose a simple send/receive interface for NPDU bytes.
pub trait TransportPort: Send + Sync {
    /// Start the transport. Returns a receiver for incoming NPDUs.
    ///
    /// The transport spawns a background receive task that decodes incoming
    /// frames and sends `ReceivedNpdu` through the returned channel.
    fn start(
        &mut self,
    ) -> impl std::future::Future<Output = Result<mpsc::Receiver<ReceivedNpdu>, Error>> + Send;

    /// Stop the transport and clean up resources.
    fn stop(&mut self) -> impl std::future::Future<Output = Result<(), Error>> + Send;

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
}

//! BACnet/IPv6 BVLC codec per ASHRAE 135-2020 Annex U.
//!
//! Frame format: type(1) + function(1) + length(2) + source-vmac(3) + payload
//! Multicast groups: FF02::BAC0 (link-local), FF05::BAC0 (site-local)

use bytes::Bytes;

/// BVLC type byte for BACnet/IPv6 (Annex U).
pub const BVLC6_TYPE: u8 = 0x82;

/// BIP6 virtual MAC address: 3 bytes per Annex U.2.
pub type Bip6Vmac = [u8; 3];

/// Minimum BVLC-IPv6 header length: type(1) + function(1) + length(2) + source-vmac(3).
pub const BVLC6_HEADER_LENGTH: usize = 7;

/// BVLC-IPv6 unicast header length: type(1) + function(1) + length(2) + source-vmac(3) + dest-vmac(3).
pub const BVLC6_UNICAST_HEADER_LENGTH: usize = 10;

/// Maximum number of VMAC collision resolution retries before giving up (Annex U.5).
pub const MAX_VMAC_RETRIES: u32 = 3;

/// BVLC-IPv6 function codes per Annex U.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bvlc6Function {
    /// BVLC-Result (0x00).
    Result,
    /// Original-Unicast-NPDU (0x01).
    OriginalUnicast,
    /// Original-Broadcast-NPDU (0x02).
    OriginalBroadcast,
    /// Address-Resolution (0x03).
    AddressResolution,
    /// Forwarded-Address-Resolution (0x04).
    ForwardedAddressResolution,
    /// Address-Resolution-Ack (0x05).
    AddressResolutionAck,
    /// Virtual-Address-Resolution (0x06).
    VirtualAddressResolution,
    /// Virtual-Address-Resolution-Ack (0x07).
    VirtualAddressResolutionAck,
    /// Forwarded-NPDU (0x08).
    ForwardedNpdu,
    /// Register-Foreign-Device (0x09).
    RegisterForeignDevice,
    /// Delete-Foreign-Device-Table-Entry (0x0A).
    DeleteForeignDeviceEntry,
    /// Distribute-Broadcast-To-Network (0x0C).
    DistributeBroadcastToNetwork,
    /// Unrecognized function code.
    Unknown(u8),
}

impl Bvlc6Function {
    /// Convert a wire byte to a `Bvlc6Function`.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Self::Result,
            0x01 => Self::OriginalUnicast,
            0x02 => Self::OriginalBroadcast,
            0x03 => Self::AddressResolution,
            0x04 => Self::ForwardedAddressResolution,
            0x05 => Self::AddressResolutionAck,
            0x06 => Self::VirtualAddressResolution,
            0x07 => Self::VirtualAddressResolutionAck,
            0x08 => Self::ForwardedNpdu,
            0x09 => Self::RegisterForeignDevice,
            0x0A => Self::DeleteForeignDeviceEntry,
            0x0C => Self::DistributeBroadcastToNetwork,
            other => Self::Unknown(other),
        }
    }

    /// Convert a `Bvlc6Function` to its wire byte.
    pub fn to_byte(self) -> u8 {
        match self {
            Self::Result => 0x00,
            Self::OriginalUnicast => 0x01,
            Self::OriginalBroadcast => 0x02,
            Self::AddressResolution => 0x03,
            Self::ForwardedAddressResolution => 0x04,
            Self::AddressResolutionAck => 0x05,
            Self::VirtualAddressResolution => 0x06,
            Self::VirtualAddressResolutionAck => 0x07,
            Self::ForwardedNpdu => 0x08,
            Self::RegisterForeignDevice => 0x09,
            Self::DeleteForeignDeviceEntry => 0x0A,
            Self::DistributeBroadcastToNetwork => 0x0C,
            Self::Unknown(b) => b,
        }
    }
}

/// A decoded BVLC-IPv6 frame.
#[derive(Debug, Clone)]
pub struct Bvlc6Frame {
    /// BVLC-IPv6 function code.
    pub function: Bvlc6Function,
    /// Source virtual MAC address (3 bytes).
    pub source_vmac: Bip6Vmac,
    /// Destination virtual MAC address (3 bytes, present in unicast only).
    pub destination_vmac: Option<Bip6Vmac>,
    /// Payload after the BVLC-IPv6 header (typically NPDU bytes).
    pub payload: Bytes,
}

mod frame;
pub use frame::*;
mod port;
pub use port::{
    decode_bip6_mac, encode_bip6_mac, generate_random_vmac, Bip6BroadcastScope,
    Bip6ForeignDeviceConfig, Bip6Transport, BACNET_IPV6_MULTICAST,
    BACNET_IPV6_MULTICAST_LINK_LOCAL, BACNET_IPV6_MULTICAST_ORG_LOCAL,
    BACNET_IPV6_MULTICAST_SITE_LOCAL, DEFAULT_BACNET6_PORT,
};

#[cfg(test)]
mod tests;

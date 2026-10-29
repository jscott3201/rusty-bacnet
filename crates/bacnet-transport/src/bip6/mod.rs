//! The BACnet/IPv6 transport (Annex U).
//!
//! All B/IPv6 BVLL parsing and encoding lives in
//! [`bacnet_bip_codec::ipv6`]. This module holds the transport around it:
//! the socket, multicast groups, the neighbour table that maps VMACs to
//! B/IPv6 addresses and resolves the ones it lacks (U.5), foreign-device
//! registration, and [`decode_bvll6`], [`encode_bvll6`] and
//! [`bvll6_error`], which use the transport's
//! [`Error`](bacnet_types::error::Error).
//!
//! The transport's network-layer MAC is the node's 3-octet VMAC (U.1.3):
//! see [`Bip6Transport`].
//!
//! Every codec item that a public transport signature names is re-exported
//! here under its codec name, so a crate that depends on the transport can
//! build and read B/IPv6 messages without depending on the codec too: the
//! message, frame and error types, the function and result codes, the VMAC
//! and address types, the sink trait and the wire-length constants.
//!
//! ```
//! use bacnet_transport::bip6::{
//!     decode_bvll6, encode_bvll6, Bip6Address, Bip6Vmac, Bvlc6Function, Bvll6,
//!     BIP6_ADDRESS_LENGTH, HEADER_LENGTH, VMAC_LENGTH,
//! };
//! use bytes::BytesMut;
//! use std::net::Ipv6Addr;
//!
//! // A Forwarded-NPDU, as a BBMD hands a foreign device a broadcast.
//! let origin = Bip6Address::new(Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 9), 47808);
//! let npdu = [0x01, 0x00, 0x10, 0x08];
//! let forwarded = Bvll6::ForwardedNpdu {
//!     original_source: Bip6Vmac::new([0x00, 0x04, 0xD2]),
//!     original_address: origin,
//!     npdu: &npdu,
//! };
//! let mut wire = BytesMut::new();
//! encode_bvll6(&mut wire, &forwarded)?;
//! assert_eq!(wire.len(), HEADER_LENGTH + VMAC_LENGTH + BIP6_ADDRESS_LENGTH + npdu.len());
//!
//! let received = decode_bvll6(&wire)?;
//! assert_eq!(received.function(), Bvlc6Function::FORWARDED_NPDU);
//! assert_eq!(received.origin(), Some(origin));
//! assert_eq!(received.npdu(), Some(&npdu[..]));
//! # Ok::<(), bacnet_types::error::Error>(())
//! ```

pub use bacnet_bip_codec::ipv6::{
    BIP6_ADDRESS_LENGTH, BVLC_TYPE_BIP6, Bip6Address, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode,
    Bvll6, Bvll6Error, Bvll6Frame, VMAC_LENGTH,
};
pub use bacnet_bip_codec::{BvllSink, HEADER_LENGTH, MAX_BVLL_LENGTH, PayloadShape, SinkFull};

/// Maximum number of random VMAC regenerations before startup fails (Annex U.5/H.7.2).
pub const MAX_VMAC_RETRIES: u32 = 3;

mod addressing;
mod collision_probe;
mod foreign_target;
pub use foreign_target::Bip6ForeignDeviceRefusal;
mod frame;
pub use frame::{bvll6_error, decode_bvll6, encode_bvll6};
mod ingress;
mod link;
mod port;
mod receive;
mod receive_counters;
mod reply_limit;
mod resolve;
mod resolve_counters;
mod resolver;
mod send;
mod socket;
mod vmac_table;
pub use port::{
    BACNET_IPV6_MULTICAST, BACNET_IPV6_MULTICAST_LINK_LOCAL, BACNET_IPV6_MULTICAST_ORG_LOCAL,
    BACNET_IPV6_MULTICAST_SITE_LOCAL, Bip6BroadcastScope, Bip6ForeignDeviceConfig, Bip6Transport,
    DEFAULT_BACNET6_PORT,
};
pub use receive_counters::Bip6ReceiveCounters;
pub use resolve_counters::Bip6ResolutionCounters;
pub use resolver::UnresolvedSend;
pub use vmac_table::{
    VMAC_TEXT_PREFIX, format_vmac_text, generate_random_vmac, parse_vmac_text,
    vmac_from_device_instance,
};

#[cfg(test)]
mod foreign_retarget_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod group_delivery_tests;
#[cfg(all(test, unix))]
mod multicast_resolution_tests;
#[cfg(test)]
mod network_port_tests;
#[cfg(test)]
mod receive_counters_tests;
#[cfg(test)]
mod reply_tests;
#[cfg(test)]
mod resolve_tests;
#[cfg(test)]
mod resolver_tests;
#[cfg(test)]
mod revalidation_tests;
#[cfg(test)]
mod safety_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vmac_generation_tests;

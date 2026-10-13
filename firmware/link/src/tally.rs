//! Tallies the scenarios keep of what they encode: a running FNV-1a hash,
//! and for the B/IP node, the BBMD and the router, [`Sent`], a record of
//! every datagram, or MS/TP frame, they send.

use core::net::SocketAddrV4;

use bacnet_bip_codec::{address_to_octets, Bvll};

/// FNV-1a's 32-bit offset basis.
pub(crate) const FNV_OFFSET: u32 = 0x811C_9DC5;
/// FNV-1a's 32-bit prime.
const FNV_PRIME: u32 = 0x0100_0193;

/// Fold `octets` into a running 32-bit FNV-1a hash.
pub(crate) fn fnv1a(hash: u32, octets: &[u8]) -> u32 {
    octets.iter().fold(hash, |hash, &octet| {
        (hash ^ u32::from(octet)).wrapping_mul(FNV_PRIME)
    })
}

/// The datagrams a scenario sent, in order: how many, their octets, and a
/// 32-bit FNV-1a hash over each one's destination (its six B/IP address
/// octets, the port high octet first) followed by the datagram. Any change
/// to what was sent, or where, changes the hash. The router scenario keeps
/// a second one for its MS/TP frames, each hashed whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    /// Datagrams sent.
    pub count: u8,
    /// Their octets, in all.
    pub octets: u16,
    /// The hash over each destination and datagram.
    pub fnv: u32,
}

impl Sent {
    /// Nothing sent yet.
    pub(crate) const fn new() -> Self {
        Self {
            count: 0,
            octets: 0,
            fnv: FNV_OFFSET,
        }
    }

    /// Encode `message` into `wire` and count it as sent to `to`, as the
    /// host's socket would send it. Returns the encoded datagram, or `None`
    /// when `wire` can't hold it.
    pub(crate) fn send<'w>(
        &mut self,
        wire: &'w mut [u8],
        to: SocketAddrV4,
        message: &Bvll<'_>,
    ) -> Option<&'w [u8]> {
        let length = message.encode_into(wire).ok()?;
        let datagram = wire.get(..length)?;
        self.count = self.count.saturating_add(1);
        self.octets = self
            .octets
            .saturating_add(u16::try_from(length).unwrap_or(u16::MAX));
        self.fnv = fnv1a(fnv1a(self.fnv, &address_to_octets(to)), datagram);
        Some(datagram)
    }

    /// Count `frame`, an MS/TP frame whole as it goes on the line. Its
    /// header names where it goes, so the hash takes the frame alone.
    pub(crate) fn frame(&mut self, frame: &[u8]) {
        self.count = self.count.saturating_add(1);
        self.octets = self
            .octets
            .saturating_add(u16::try_from(frame.len()).unwrap_or(u16::MAX));
        self.fnv = fnv1a(self.fnv, frame);
    }
}

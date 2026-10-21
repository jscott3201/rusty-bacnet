//! A BBMD behind a NAT router (Annex J.7.5).
//!
//! The router port-forwards the B/IP UDP port to the BBMD, so the other
//! sites know the BBMD only by the router's public address and port: its
//! global address. [`Tables::set_nat`](super::Tables::set_nat) makes that
//! the BBMD's own address. From then on:
//!
//! - the BDT must list the global address as this BBMD's own row, and every
//!   row must be two-hop, since a directed broadcast can't cross the router
//!   ([`Bdt::set`](super::Bdt::set) refuses a table that breaks either);
//! - a Forwarded-NPDU this BBMD builds for a broadcast from its own side
//!   names the global address as the originating device, so a device at
//!   another site has an address it can answer;
//! - a Forwarded-NPDU it passes on keeps the originating address it came
//!   with.
//!
//! A foreign device's Distribute-Broadcast-To-Network is the one case where
//! the global address isn't always right. A device that registered from
//! this BBMD's private side can't be reached from another site at its own
//! address, so the global one goes in its place; a device that registered
//! from a public address can, and keeps it. [`Nat::is_private`] tells the
//! two apart: the RFC 1918 ranges, and any range the installation adds.

use core::fmt;
use core::net::{Ipv4Addr, SocketAddrV4};

/// The most private ranges a [`Nat`] holds besides the RFC 1918 ones.
pub const MAX_PRIVATE_RANGES: usize = 4;

/// An IPv4 address range, as a network address and a prefix length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateRange {
    network: Ipv4Addr,
    prefix: u8,
}

impl PrivateRange {
    /// The range of `network` under a prefix of `prefix` bits, from 0 to
    /// 32. Host bits set in `network` are cleared.
    pub const fn new(network: Ipv4Addr, prefix: u8) -> Result<Self, NatError> {
        if prefix > 32 {
            return Err(NatError::Prefix);
        }
        let bits = network.to_bits() & mask_bits(prefix);
        Ok(Self {
            network: Ipv4Addr::from_bits(bits),
            prefix,
        })
    }

    /// The network address, its host bits clear.
    #[must_use]
    pub const fn network(&self) -> Ipv4Addr {
        self.network
    }

    /// The prefix length, in bits.
    #[must_use]
    pub const fn prefix(&self) -> u8 {
        self.prefix
    }

    /// Whether `ip` is in the range.
    #[must_use]
    pub const fn contains(&self, ip: Ipv4Addr) -> bool {
        (ip.to_bits() ^ self.network.to_bits()) & mask_bits(self.prefix) == 0
    }
}

impl fmt::Display for PrivateRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

/// The mask of a `prefix`-bit prefix, `prefix` at most 32.
const fn mask_bits(prefix: u8) -> u32 {
    match prefix {
        0 => 0,
        _ => u32::MAX << (32 - prefix as u32),
    }
}

/// Whether `ip` is in one of the private ranges of RFC 1918: 10.0.0.0/8,
/// 172.16.0.0/12 or 192.168.0.0/16.
#[must_use]
pub const fn is_rfc1918(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 10 || (a == 172 && b & 0xF0 == 16) || (a == 192 && b == 168)
}

/// Why a [`Nat`] setting was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NatError {
    /// A prefix longer than 32 bits.
    Prefix,
    /// More than [`MAX_PRIVATE_RANGES`] private ranges.
    TooManyRanges,
}

impl fmt::Display for NatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prefix => f.write_str("an IPv4 prefix is at most 32 bits"),
            Self::TooManyRanges => write!(
                f,
                "at most {MAX_PRIVATE_RANGES} private ranges besides RFC 1918"
            ),
        }
    }
}

impl core::error::Error for NatError {}

/// How a BBMD behind a NAT router is reached (Annex J.7.5): its global
/// address, and the address ranges that count as private besides RFC 1918.
///
/// It is `#[non_exhaustive]`: outside this crate, build it with
/// [`Nat::new`] and add ranges with [`Nat::add_private_range`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Nat {
    /// The NAT router's public address and port that forward to this BBMD.
    pub global: SocketAddrV4,
    ranges: [PrivateRange; MAX_PRIVATE_RANGES],
    len: u8,
}

/// An unused slot of a [`Nat`]'s range storage.
const UNUSED_RANGE: PrivateRange = PrivateRange {
    network: Ipv4Addr::UNSPECIFIED,
    prefix: 32,
};

impl Nat {
    /// A BBMD reached at `global`, with the RFC 1918 ranges alone private.
    #[must_use]
    pub const fn new(global: SocketAddrV4) -> Self {
        Self {
            global,
            ranges: [UNUSED_RANGE; MAX_PRIVATE_RANGES],
            len: 0,
        }
    }

    /// Count `range` as private too. A range already listed is not added
    /// again.
    pub fn add_private_range(&mut self, range: PrivateRange) -> Result<(), NatError> {
        if self.private_ranges().contains(&range) {
            return Ok(());
        }
        let slot = self
            .ranges
            .get_mut(usize::from(self.len))
            .ok_or(NatError::TooManyRanges)?;
        *slot = range;
        self.len += 1;
        Ok(())
    }

    /// The ranges added with [`add_private_range`](Self::add_private_range).
    #[must_use]
    pub fn private_ranges(&self) -> &[PrivateRange] {
        &self.ranges[..usize::from(self.len)]
    }

    /// Whether `ip` is a private address: RFC 1918, or in an added range.
    #[must_use]
    pub fn is_private(&self, ip: Ipv4Addr) -> bool {
        is_rfc1918(ip) || self.private_ranges().iter().any(|range| range.contains(ip))
    }
}

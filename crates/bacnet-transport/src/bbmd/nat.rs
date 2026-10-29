//! A hosted BBMD behind a NAT router (Annex J.7.5, #940).
//!
//! The NAT router port-forwards the B/IP UDP port to this BBMD, and the
//! other sites know it by the router's public address and port, its global
//! address. That address is configuration: this BBMD doesn't discover it,
//! and takes no host names. With it set, `bacnet-bip-core` runs the BBMD
//! under the global address ([`Tables::set_nat`](bacnet_bip_core::bbmd::Tables::set_nat)):
//! the BDT must list it as the own row, every row is two-hop, and the
//! Forwarded-NPDUs the BBMD builds name it as their origin. See
//! [`bacnet_bip_core::bbmd::Nat`] for the rule on a foreign device's
//! Distribute-Broadcast-To-Network.
//!
//! The socket keeps its private address: the node's own address, its echo
//! filter and its subnet are the private side's.

use std::net::SocketAddrV4;

use bacnet_bip_codec::BdtEntries;
use bacnet_bip_core::bbmd::{BdtError, MAX_PRIVATE_RANGES, Nat};
pub use bacnet_bip_core::bbmd::{NatError, PrivateRange};
use bacnet_types::error::Error;

use super::{BbmdState, bdt_error};

/// How a BBMD behind a NAT router is reached: [`BbmdOptions::nat`](crate::bip::BbmdOptions::nat).
///
/// ```
/// use std::net::{Ipv4Addr, SocketAddrV4};
/// use bacnet_transport::bbmd::{NatError, NatTraversal, PrivateRange};
///
/// // The router forwards 203.0.113.1:47809 to this BBMD, and devices in
/// // the carrier's 100.64.0.0/10 range count as private.
/// let mut nat = NatTraversal::new(SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 1), 47809));
/// nat.private_ranges.push(PrivateRange::new(Ipv4Addr::new(100, 64, 0, 0), 10)?);
/// # Ok::<(), NatError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NatTraversal {
    /// The NAT router's public address and port that forward to this BBMD.
    /// The BDT must list it, with an all-ones mask, as this BBMD's own row.
    pub global_address: SocketAddrV4,
    /// Address ranges to count as private besides RFC 1918's, at most
    /// [`NatTraversal::MAX_PRIVATE_RANGES`]. A foreign device that
    /// registered from a private address has its broadcasts sent on with
    /// the global address as their origin, since other sites can't answer
    /// it at its own.
    pub private_ranges: Vec<PrivateRange>,
}

impl NatTraversal {
    /// The most [`private_ranges`](Self::private_ranges).
    pub const MAX_PRIVATE_RANGES: usize = MAX_PRIVATE_RANGES;

    /// A BBMD reached at `global_address`, with only the RFC 1918 ranges
    /// private.
    pub fn new(global_address: SocketAddrV4) -> Self {
        Self {
            global_address,
            private_ranges: Vec::new(),
        }
    }

    /// This setting with `range` counted as private too.
    #[must_use]
    pub fn with_private_range(mut self, range: PrivateRange) -> Self {
        self.private_ranges.push(range);
        self
    }
}

/// Refuse a global address no peer could send to: the unspecified,
/// broadcast or a multicast address, or port 0. A loopback one is refused
/// too unless `allow_loopback`: the Rust configuration takes it, for
/// tests on one host, but a Network Port write does not.
pub(super) fn check_global(global: SocketAddrV4, allow_loopback: bool) -> Result<(), Error> {
    let ip = global.ip();
    if global.port() == 0
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || (ip.is_loopback() && !allow_loopback)
    {
        return Err(Error::Encoding(format!(
            "NAT global address {global} is not a unicast address and port"
        )));
    }
    Ok(())
}

/// What a BBMD keeps of its NAT setting: whether it runs behind NAT
/// (BACnet_IP_NAT_Traversal), the global address even while it doesn't
/// (BACnet_IP_Global_Address), and the private ranges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct NatSetting {
    pub(super) enabled: bool,
    pub(super) global: Option<SocketAddrV4>,
    pub(super) private_ranges: Vec<PrivateRange>,
    /// Whether the persisted-BDT file carries this setting: set when a
    /// file loaded at start held one, or an activation changed
    /// BACnet_IP_NAT_Traversal or BACnet_IP_Global_Address. Until then a
    /// save writes the rows only, so the configured setting still applies
    /// at the next start.
    pub(super) saved: bool,
}

impl NatSetting {
    /// The core's setting for `enabled` and `global`, with these ranges:
    /// `None` when not enabled; an error when enabled with no global
    /// address.
    pub(super) fn core(
        &self,
        enabled: bool,
        global: Option<SocketAddrV4>,
    ) -> Result<Option<Nat>, Error> {
        if !enabled {
            return Ok(None);
        }
        let global =
            global.ok_or_else(|| Error::Encoding("NAT traversal needs a global address".into()))?;
        let mut nat = Nat::new(global);
        for range in &self.private_ranges {
            nat.add_private_range(*range)
                .map_err(|e| Error::Encoding(format!("NAT private ranges: {e}")))?;
        }
        Ok(Some(nat))
    }
}

/// The host's error for a NAT setting the core refused.
fn nat_error(error: BdtError) -> Error {
    match error {
        BdtError::Invalid { entry, reason } => Error::Encoding(format!(
            "NAT global address {} refused: {reason}",
            entry.address
        )),
        other => bdt_error(other),
    }
}

impl BbmdState {
    /// Run this BBMD behind a NAT router as `nat` says, or, with `None`,
    /// not (Annex J.7.5).
    ///
    /// Behind NAT the BBMD's own address in its BDT is the global address,
    /// which the BDT must list with an all-ones mask, as it must every row
    /// (two-hop only). A BDT already set is checked against that now; a
    /// BDT set later is checked when it is set. On error, a refused global
    /// address, a BDT that breaks the rules or too many private ranges,
    /// nothing changes.
    pub fn set_nat_traversal(&mut self, nat: Option<NatTraversal>) -> Result<(), Error> {
        let setting = match nat {
            Some(nat) => NatSetting {
                enabled: true,
                global: Some(nat.global_address),
                private_ranges: nat.private_ranges,
                saved: self.nat.saved,
            },
            None => NatSetting {
                saved: self.nat.saved,
                ..NatSetting::default()
            },
        };
        let core = setting.core(setting.enabled, setting.global)?;
        self.tables.set_nat(core, self.local).map_err(nat_error)?;
        self.nat = setting;
        Ok(())
    }

    /// Take a NAT setting a persisted-BDT file holds: `enabled` and
    /// `global`, with `private_ranges` from the configuration. On error
    /// nothing changes.
    pub(crate) fn set_persisted_nat(
        &mut self,
        nat: super::PersistedNat,
        private_ranges: Vec<PrivateRange>,
    ) -> Result<(), Error> {
        let setting = NatSetting {
            enabled: nat.enabled,
            global: nat.global,
            private_ranges,
            saved: true,
        };
        if let Some(global) = setting.global {
            check_global(global, true)?;
        }
        let core = setting.core(setting.enabled, setting.global)?;
        self.tables.set_nat(core, self.local).map_err(nat_error)?;
        self.nat = setting;
        Ok(())
    }

    /// The NAT setting a persisted-BDT file saves, or `None` for a
    /// rows-only file: see [`NatSetting::saved`]. `activated` says the
    /// activation just made changed the NAT setting or the global address,
    /// which from then on is always saved.
    pub(crate) fn nat_to_persist(&mut self, activated: bool) -> Option<super::PersistedNat> {
        self.nat.saved |= activated;
        self.nat.saved.then_some(super::PersistedNat {
            enabled: self.nat.enabled,
            global: self.nat.global,
        })
    }

    /// Whether this BBMD runs behind a NAT router.
    pub fn nat_traversal(&self) -> bool {
        self.nat.enabled
    }

    /// The NAT router's public address and port, if one is configured: in
    /// effect while [`nat_traversal`](Self::nat_traversal) is on, and kept
    /// while it is off.
    pub fn global_address(&self) -> Option<SocketAddrV4> {
        self.nat.global
    }

    /// The B/IP address the other BBMDs know this one by, its own BDT row:
    /// the global address behind NAT, the local address otherwise.
    pub fn own_address(&self) -> SocketAddrV4 {
        self.tables.bdt.own_address()
    }

    /// Put a NAT setting and a BDT into effect together, all or nothing:
    /// `enabled` with `global` and the configured private ranges, and
    /// `rows` as the configured BDT.
    pub(super) fn apply_nat_and_bdt(
        &mut self,
        enabled: bool,
        global: Option<SocketAddrV4>,
        rows: &[bacnet_bip_codec::BdtEntry],
    ) -> Result<(), Error> {
        let core = self.nat.core(enabled, global)?;
        self.tables
            .set_nat_and_bdt(core, self.local, BdtEntries::from_entries(rows))
            .map_err(nat_error)?;
        self.nat.enabled = enabled;
        self.nat.global = global;
        Ok(())
    }
}

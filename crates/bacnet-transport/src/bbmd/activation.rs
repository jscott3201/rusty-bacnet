//! Values written to the BBMD's Network Port properties, checked against
//! the BDT policy and put into effect on activation (Clause 12.56.13,
//! 12.56.32 to 12.56.35, #1578, #940).
//!
//! A write is checked alone ([`BbmdState::check_value`]): a BDT against the
//! row policy, a global address as an IPv4 unicast host and port. The rules
//! that tie values together, a NAT BBMD's BDT listing its global address and
//! being two-hop only, are checked at activation, when every value waiting
//! goes in at once ([`BbmdState::check_changes`]). So values can be written
//! in any order, and a new global address and the table that lists it go
//! in together.
//!
//! A BBMD that serves a B/IP-M group gives its own row an all-ones mask
//! (Annex J.8.3), so a change of group can change which BDTs fit: the
//! transport checks and applies the tables under the rule the new group
//! sets (`serves`), together with the group (#1963), and undoes them
//! ([`BbmdState::restore`]) if the group then can't move.

use std::net::{IpAddr, SocketAddrV4};

use bacnet_bip_codec::{self as codec, BdtEntries};
use bacnet_bip_core::bbmd as core_bbmd;
use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::BbmdState;
use super::nat::check_global;

/// The WriteProperty refusal of a value the BBMD can't take.
fn out_of_range() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
}

/// A written BDT as the rows the core takes. A row must name an IPv4 host
/// and carry a mask: no host is never a table entry (Clause 12.56.34), and
/// this BBMD resolves no host names.
fn rows(table: &[BACnetBDTEntry]) -> Result<Vec<codec::BdtEntry>, Error> {
    table
        .iter()
        .map(
            |entry| match (&entry.bbmd_address.host, entry.broadcast_mask) {
                (BACnetHostAddress::Ip(IpAddr::V4(ip)), Some(mask)) => Ok(codec::BdtEntry::new(
                    SocketAddrV4::new(*ip, entry.bbmd_address.port),
                    mask.into(),
                )),
                _ => Err(out_of_range()),
            },
        )
        .collect()
}

/// A written BACnet_IP_Global_Address: an IPv4 unicast host (not the
/// unspecified, broadcast, a multicast or a loopback address) and a nonzero
/// port, or the `None` host with port 0, which clears it (Clause 12.56.33).
/// This BBMD resolves no host names.
fn global_of(address: &BACnetHostNPort) -> Result<Option<SocketAddrV4>, Error> {
    match (&address.host, address.port) {
        (BACnetHostAddress::None, 0) => Ok(None),
        (BACnetHostAddress::Ip(IpAddr::V4(ip)), port) => {
            let global = SocketAddrV4::new(*ip, port);
            check_global(global, false).map_err(|_| out_of_range())?;
            Ok(Some(global))
        }
        _ => Err(out_of_range()),
    }
}

/// The NAT setting and BDT `changes` leave a BBMD with.
struct Candidate {
    enabled: bool,
    global: Option<SocketAddrV4>,
    rows: Vec<codec::BdtEntry>,
}

/// What an activation replaced, for [`BbmdState::restore`].
#[derive(Debug)]
pub(crate) struct Undo {
    serves: bool,
    enabled: bool,
    global: Option<SocketAddrV4>,
    rows: Vec<codec::BdtEntry>,
    accept: bool,
}

impl BbmdState {
    /// Whether `table` would pass as this BBMD's BDT: the row policy and
    /// the NAT rules as the BBMD stands, its own row included, on a copy.
    pub(crate) fn check_written_bdt(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        let mut changes = BbmdChanges::default();
        changes.broadcast_distribution_table = Some(table.to_vec());
        self.check_changes(&changes)
    }

    /// Check written values each on its own, as a WriteProperty takes them:
    /// a BDT's rows against the row policy, without the NAT rules, which
    /// [`Self::check_changes`] applies at activation, and a global address
    /// as an IPv4 unicast host and port.
    pub(crate) fn check_value(&self, changes: &BbmdChanges) -> Result<(), Error> {
        if let Some(table) = &changes.broadcast_distribution_table {
            let rows = rows(table)?;
            let mut out = rows.clone();
            let distinct = core_bbmd::canonicalize(BdtEntries::from_entries(&rows), &mut out)
                .map_err(|_| out_of_range())?;
            if distinct > Self::MAX_BDT_ENTRIES {
                return Err(out_of_range());
            }
        }
        if let Some(address) = &changes.global_address {
            global_of(address)?;
        }
        Ok(())
    }

    /// The NAT setting and BDT after `changes`, when they touch either or
    /// `regroup` asks for the table to be checked again under a new group
    /// rule.
    fn candidate(&self, changes: &BbmdChanges, regroup: bool) -> Result<Option<Candidate>, Error> {
        if changes.broadcast_distribution_table.is_none()
            && changes.nat_traversal.is_none()
            && changes.global_address.is_none()
            && !regroup
        {
            return Ok(None);
        }
        let rows = match &changes.broadcast_distribution_table {
            Some(table) => rows(table)?,
            None => self.tables.bdt.configured_rows().to_vec(),
        };
        let global = match &changes.global_address {
            Some(address) => global_of(address)?,
            None => self.nat.global,
        };
        Ok(Some(Candidate {
            enabled: changes.nat_traversal.unwrap_or(self.nat.enabled),
            global,
            rows,
        }))
    }

    /// Whether [`Self::activate`] would take `changes`, changing nothing:
    /// the BDT, the NAT setting and the global address are checked as they
    /// would go in together, on a copy.
    pub(crate) fn check_changes(&self, changes: &BbmdChanges) -> Result<(), Error> {
        self.check_serving(changes, None)
    }

    /// [`Self::check_changes`], with the BBMD serving a B/IP-M group from
    /// then on or not, when `serves` says (#1963): the table must then fit
    /// that group's own-row rule.
    pub(crate) fn check_serving(
        &self,
        changes: &BbmdChanges,
        serves: Option<bool>,
    ) -> Result<(), Error> {
        let Some(candidate) = self.candidate(changes, serves.is_some())? else {
            return Ok(());
        };
        let nat = self
            .nat
            .core(candidate.enabled, candidate.global)
            .map_err(|_| out_of_range())?;
        let mut probe = self.tables.clone();
        // Leaving the group lifts its rule before the rows go in; joining
        // one checks the rows against it once they are in.
        if serves == Some(false) {
            probe
                .bdt
                .set_serves_group(false)
                .map_err(|_| out_of_range())?;
        }
        probe
            .set_nat_and_bdt(nat, self.local, BdtEntries::from_entries(&candidate.rows))
            .map_err(|_| out_of_range())?;
        if serves == Some(true) {
            probe
                .bdt
                .set_serves_group(true)
                .map_err(|_| out_of_range())?;
        }
        Ok(())
    }

    /// Accept foreign-device registrations or stop. A policy that stops
    /// is kept, and comes back when registrations are accepted again; with
    /// none kept, [`ForeignDevicePolicy::default`](super::ForeignDevicePolicy::default) applies. Registrations
    /// already made stay until they lapse.
    pub fn set_accepts_foreign_device_registrations(&mut self, accept: bool) {
        match (accept, self.foreign_device_policy.is_some()) {
            (true, false) => {
                self.foreign_device_policy = Some(self.parked_policy.take().unwrap_or_default());
            }
            (false, true) => self.parked_policy = self.foreign_device_policy.take(),
            _ => {}
        }
    }

    /// Put `changes` into effect: the BDT and NAT setting together, then
    /// the registration setting, or none when the first are refused.
    pub(crate) fn activate(&mut self, changes: &BbmdChanges) -> Result<(), Error> {
        self.activate_serving(changes, None).map(drop)
    }

    /// [`Self::activate`], serving a B/IP-M group from then on or not when
    /// `serves` says (#1963), checked first as
    /// [`Self::check_serving`] checks it, so it applies whole or not at
    /// all. Returns what it replaced, for [`Self::restore`].
    pub(crate) fn activate_serving(
        &mut self,
        changes: &BbmdChanges,
        serves: Option<bool>,
    ) -> Result<Undo, Error> {
        self.check_serving(changes, serves)?;
        let undo = Undo {
            serves: self.serves_group(),
            enabled: self.nat.enabled,
            global: self.nat.global,
            rows: self.tables.bdt.configured_rows().to_vec(),
            accept: self.accepts_foreign_device_registrations(),
        };
        if let Some(candidate) = self.candidate(changes, serves.is_some())? {
            if serves == Some(false) {
                self.set_serves_group(false)?;
            }
            let applied =
                self.apply_nat_and_bdt(candidate.enabled, candidate.global, &candidate.rows);
            let joined = match (applied, serves) {
                (Ok(()), Some(true)) => self.set_serves_group(true),
                (applied, _) => applied,
            };
            if joined.is_err() {
                self.restore(undo);
                return Err(out_of_range());
            }
        }
        if let Some(accept) = changes.accept_foreign_device_registrations {
            self.set_accepts_foreign_device_registrations(accept);
        }
        Ok(undo)
    }

    /// Put back what an activation replaced: the table, the NAT setting,
    /// the group rule and the registration setting it had. They were in
    /// effect together, so they go back in.
    pub(crate) fn restore(&mut self, undo: Undo) {
        let restored = self
            .set_serves_group(false)
            .and_then(|()| self.apply_nat_and_bdt(undo.enabled, undo.global, &undo.rows))
            .and_then(|()| self.set_serves_group(undo.serves));
        if let Err(error) = restored {
            tracing::warn!(%error, "Could not restore the BBMD tables after a refused activation");
        }
        self.set_accepts_foreign_device_registrations(undo.accept);
    }
}

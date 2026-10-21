//! A BBMD's tables as Network Port property values (Clause 12.56.34 to
//! 12.56.36), read live from the shared state.
//!
//! These are the Clause 21 values the Network Port object serves, not the
//! 10-octet BVLL rows of Read-BDT-Ack and Read-FDT-Ack.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use bacnet_types::bip_port::{BbmdChanges, BbmdTables};
use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
};
use bacnet_types::error::Error;

use super::persist::BdtFile;
use super::BbmdState;

/// Lock shared BBMD state. Every holder keeps the guard for a short,
/// synchronous critical section and never across an await. A panic inside
/// one leaves the tables as consistent as each method keeps them, so a
/// poisoned lock is taken over rather than taking the BBMD down.
pub(crate) fn lock(state: &Mutex<BbmdState>) -> MutexGuard<'_, BbmdState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl BbmdState {
    /// The BDT, this BBMD's own row included, as
    /// BBMD_Broadcast_Distribution_Table entries: each peer as an IPv4 host
    /// and port, with its broadcast distribution mask.
    pub fn bdt_entries(&self) -> Vec<BACnetBDTEntry> {
        self.tables
            .bdt
            .rows()
            .iter()
            .map(|row| BACnetBDTEntry {
                bbmd_address: BACnetHostNPort::from_socket_addr(SocketAddr::V4(row.address)),
                broadcast_mask: Some(row.mask.octets()),
            })
            .collect()
    }

    /// [`Self::global_address`] as BACnet_IP_Global_Address: the `None`
    /// host with port 0 when none is configured (Clause 12.56.33).
    pub fn global_address_value(&self) -> BACnetHostNPort {
        self.global_address().map_or(
            BACnetHostNPort {
                host: BACnetHostAddress::None,
                port: 0,
            },
            |global| BACnetHostNPort::from_socket_addr(SocketAddr::V4(global)),
        )
    }

    /// Whether a foreign device registration policy is set. Without one
    /// every Register-Foreign-Device is refused.
    pub fn accepts_foreign_device_registrations(&self) -> bool {
        self.foreign_device_policy.is_some()
    }

    /// The registrations still live at `now`, as BBMD_Foreign_Device_Table
    /// entries: each with its time to live and the seconds left, grace
    /// period included. Unlike [`Self::fdt`] this purges nothing, so a read
    /// leaves the table and its counters alone.
    pub fn fdt_entries_at(&self, now: Instant) -> Vec<BACnetFDTEntry> {
        let now = self.clock.instant(now);
        self.tables
            .fdt
            .live_at(now)
            .map(|registration| BACnetFDTEntry {
                address: SocketAddr::V4(registration.address),
                time_to_live: registration.ttl,
                remaining_time_to_live: registration.seconds_remaining_at(now),
            })
            .collect()
    }
}

/// The live view a B/IP transport lends a Network Port object in BBMD mode:
/// the transport's own shared state, not a copy. Each read takes the lock
/// for one copy of a table and releases it. With a persisted-BDT file, an
/// activated BDT is saved there too (#1826).
pub(crate) struct LiveTables {
    pub(crate) state: Arc<Mutex<BbmdState>>,
    pub(crate) file: Option<Arc<BdtFile>>,
}

impl BbmdTables for LiveTables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        lock(&self.state).bdt_entries()
    }

    fn accepts_foreign_device_registrations(&self) -> bool {
        lock(&self.state).accepts_foreign_device_registrations()
    }

    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        let now = Instant::now();
        lock(&self.state).fdt_entries_at(now)
    }

    fn check_broadcast_distribution_table(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        lock(&self.state).check_written_bdt(table)
    }

    fn nat_traversal(&self) -> Option<bool> {
        Some(lock(&self.state).nat_traversal())
    }

    fn global_address(&self) -> Option<BACnetHostNPort> {
        Some(lock(&self.state).global_address_value())
    }

    fn check_value(&self, changes: &BbmdChanges) -> Result<(), Error> {
        lock(&self.state).check_value(changes)
    }

    fn check(&self, changes: &BbmdChanges) -> Result<(), Error> {
        lock(&self.state).check_changes(changes)
    }

    /// A new BDT or NAT setting is saved to the persisted-BDT file once it
    /// is in effect (#940). The NAT setting goes with the rows only once it
    /// is the network's to keep: after an activation that changed
    /// NAT_Traversal or Global_Address, or when the file loaded at start
    /// held one. A BDT-only activation otherwise writes the rows alone, so
    /// a later change to the configured NAT setting still applies.
    /// Only the activation and taking the rows happen under the locks the
    /// caller and this view hold; the file is written on a thread of its
    /// own afterwards (`BdtFile::save_later`). A failed save is logged and counted and doesn't undo the
    /// activation.
    fn activate(&self, changes: &BbmdChanges) -> Result<(), Error> {
        let file = self.file.as_ref().filter(|_| {
            changes.broadcast_distribution_table.is_some()
                || changes.nat_traversal.is_some()
                || changes.global_address.is_some()
        });
        let Some(file) = file else {
            return lock(&self.state).activate(changes);
        };
        let nat_changed = changes.nat_traversal.is_some() || changes.global_address.is_some();
        let (generation, rows, nat) = {
            let mut state = lock(&self.state);
            state.activate(changes)?;
            (
                file.next_generation(),
                state.configured_bdt(),
                state.nat_to_persist(nat_changed),
            )
        };
        file.save_later(generation, rows, nat);
        Ok(())
    }

    fn can_change(&self) -> bool {
        true
    }
}

//! A BBMD's tables as Network Port property values (Clause 12.56.34 to
//! 12.56.36), read live from the shared state.
//!
//! These are the Clause 21 values the Network Port object serves, not the
//! 10-octet BVLL rows of Read-BDT-Ack and Read-FDT-Ack.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use bacnet_types::bip_port::BbmdTables;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetFDTEntry, BACnetHostNPort};

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
        self.bdt
            .iter()
            .map(|entry| BACnetBDTEntry {
                bbmd_address: BACnetHostNPort::from_socket_addr(SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::from(entry.ip),
                    entry.port,
                ))),
                broadcast_mask: Some(entry.broadcast_mask),
            })
            .collect()
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
        self.fdt
            .iter()
            .filter(|entry| !entry.is_expired_at(now))
            .map(|entry| BACnetFDTEntry {
                address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(entry.ip), entry.port)),
                time_to_live: entry.ttl,
                remaining_time_to_live: entry.seconds_remaining_at(now),
            })
            .collect()
    }
}

/// The live view a B/IP transport lends a Network Port object in BBMD mode:
/// the transport's own shared state, not a copy. Each read takes the lock
/// for one copy of a table and releases it.
pub(crate) struct LiveTables(pub(crate) Arc<Mutex<BbmdState>>);

impl BbmdTables for LiveTables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        lock(&self.0).bdt_entries()
    }

    fn accepts_foreign_device_registrations(&self) -> bool {
        lock(&self.0).accepts_foreign_device_registrations()
    }

    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        let now = Instant::now();
        lock(&self.0).fdt_entries_at(now)
    }
}

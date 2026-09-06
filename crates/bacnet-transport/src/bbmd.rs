//! BBMD (BACnet/IP Broadcast Management Device) state and tables.
//!
//! Manages the Broadcast Distribution Table (BDT) and Foreign Device Table
//! (FDT) per ASHRAE 135-2020 Annex J. Pure state/logic — no async or I/O.

use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::time::{Duration, Instant};

use bacnet_types::enums::BvlcResultCode;
use bacnet_types::error::Error;
use bytes::{BufMut, BytesMut};

mod policy;
use policy::ForeignDeviceRateTracker;
pub use policy::{FdtCounters, ForeignDevicePolicy};

/// BDT entry wire format size: IP(4) + port(2) + mask(4) = 10 bytes.
pub const BDT_ENTRY_SIZE: usize = 10;

/// FDT entry wire format size: IP(4) + port(2) + TTL(2) + remaining(2) = 10 bytes.
pub const FDT_ENTRY_SIZE: usize = 10;

/// A Broadcast Distribution Table entry — one peer BBMD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BdtEntry {
    pub ip: [u8; 4],
    pub port: u16,
    pub broadcast_mask: [u8; 4],
}

/// A Foreign Device Table entry — one registered foreign device.
#[derive(Debug, Clone)]
pub struct FdtEntry {
    pub ip: [u8; 4],
    pub port: u16,
    pub ttl: u16,
    pub registered_at: Instant,
}

impl FdtEntry {
    /// Grace period in seconds added beyond TTL before expiry.
    const GRACE_PERIOD: u64 = 30;

    /// Whether this entry has expired (TTL + grace period) relative to `now`.
    pub fn is_expired_at(&self, now: Instant) -> bool {
        let total = Duration::from_secs(self.ttl as u64 + Self::GRACE_PERIOD);
        now.saturating_duration_since(self.registered_at) > total
    }

    /// Whether this entry has expired (TTL + grace period).
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Instant::now())
    }

    /// Seconds remaining including the 30-second grace period relative to `now`.
    pub fn seconds_remaining_at(&self, now: Instant) -> u16 {
        let elapsed = now.saturating_duration_since(self.registered_at).as_secs();
        let total = self.ttl as u64 + Self::GRACE_PERIOD;
        total.saturating_sub(elapsed).min(u16::MAX as u64) as u16
    }

    /// Seconds remaining including the 30-second grace period.
    pub fn seconds_remaining(&self) -> u16 {
        self.seconds_remaining_at(Instant::now())
    }
}

/// Prefix length of a contiguous IPv4 netmask, or `None` when non-contiguous.
fn bdt_mask_prefix_len(mask: [u8; 4]) -> Option<u32> {
    let m = u32::from_be_bytes(mask);
    let inv = !m;
    (inv & inv.wrapping_add(1) == 0).then_some(m.count_ones())
}

fn bdt_invalid(e: &BdtEntry, reason: &str) -> Error {
    Error::Encoding(format!("BDT {reason}: {e:?}"))
}

/// Validate one BDT candidate entry against the issue #529 security policy.
fn validate_bdt_entry(e: &BdtEntry) -> Result<(), Error> {
    let mask = u32::from_be_bytes(e.broadcast_mask);
    let ip = u32::from_be_bytes(e.ip);
    let host = !mask;
    let prefix = bdt_mask_prefix_len(e.broadcast_mask).unwrap_or(33);
    let reason = if e.port == 0 {
        "invalid port 0"
    } else if e.ip == [0, 0, 0, 0] {
        "unspecified IP"
    } else if e.ip == [255, 255, 255, 255] {
        "limited-broadcast"
    } else if (224..=239).contains(&e.ip[0]) {
        "multicast IP"
    } else if prefix == 33 {
        "non-contiguous mask"
    } else if prefix < 8 {
        "mask prefix too wide (< /8) or limited-broadcast target"
    } else if prefix < 32 && ip & host == 0 {
        "subnet network address"
    } else if prefix < 32 && ip | mask == u32::MAX {
        "subnet directed-broadcast"
    } else if ip | host == u32::MAX {
        "limited-broadcast target"
    } else {
        ""
    };
    if reason.is_empty() {
        Ok(())
    } else {
        Err(bdt_invalid(e, reason))
    }
}

/// An FDT entry decoded from the wire (Read-FDT-ACK payload).
///
/// Unlike [`FdtEntry`], this does not contain an `Instant` field — it carries
/// only the wire-format TTL and seconds-remaining values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdtEntryWire {
    pub ip: [u8; 4],
    pub port: u16,
    pub ttl: u16,
    pub seconds_remaining: u16,
}

/// Encode a slice of BDT entries into wire format without needing a `BbmdState` instance.
pub fn encode_bdt_entries(entries: &[BdtEntry], buf: &mut BytesMut) {
    buf.reserve(entries.len() * BDT_ENTRY_SIZE);
    for entry in entries {
        buf.put_slice(&entry.ip);
        buf.put_u16(entry.port);
        buf.put_slice(&entry.broadcast_mask);
    }
}

/// Decode FDT entries from a Read-FDT-ACK payload.
///
/// Wire format per entry: ip(4) + port(2) + ttl(2) + remaining(2) = 10 bytes.
pub fn decode_fdt(data: &[u8]) -> Result<Vec<FdtEntryWire>, Error> {
    if !data.len().is_multiple_of(FDT_ENTRY_SIZE) {
        return Err(Error::decoding(
            0,
            format!(
                "FDT data length {} not a multiple of {}",
                data.len(),
                FDT_ENTRY_SIZE
            ),
        ));
    }
    let count = data.len() / FDT_ENTRY_SIZE;
    if count > BbmdState::MAX_FDT_ENTRIES {
        let max = BbmdState::MAX_FDT_ENTRIES;
        let msg = format!("FDT entry count {count} exceeds maximum of {max}");
        return Err(Error::decoding(0, msg));
    }
    let mut entries = Vec::with_capacity(count);
    for chunk in data.chunks_exact(FDT_ENTRY_SIZE) {
        entries.push(FdtEntryWire {
            ip: [chunk[0], chunk[1], chunk[2], chunk[3]],
            port: u16::from_be_bytes([chunk[4], chunk[5]]),
            ttl: u16::from_be_bytes([chunk[6], chunk[7]]),
            seconds_remaining: u16::from_be_bytes([chunk[8], chunk[9]]),
        });
    }
    Ok(entries)
}

/// BBMD state — BDT and FDT tables with forwarding logic.
#[derive(Debug)]
pub struct BbmdState {
    bdt: Vec<BdtEntry>,
    fdt: Vec<FdtEntry>,
    local_ip: [u8; 4],
    local_port: u16,
    /// Allowed source IPs for Delete-Foreign-Device-Table-Entry.
    /// Empty means deny all (fail closed).
    management_acl: Vec<[u8; 4]>,
    /// Explicit policy for foreign device registrations.
    /// None means fail closed (deny all registrations).
    foreign_device_policy: Option<ForeignDevicePolicy>,
    rate_tracker: ForeignDeviceRateTracker,
    counters: FdtCounters,
}

impl BbmdState {
    /// Create a new BBMD with the given local address.
    pub fn new(local_ip: [u8; 4], local_port: u16) -> Self {
        Self {
            bdt: Vec::new(),
            fdt: Vec::new(),
            local_ip,
            local_port,
            management_acl: Vec::new(),
            foreign_device_policy: None,
            rate_tracker: ForeignDeviceRateTracker::new(),
            counters: FdtCounters::default(),
        }
    }

    /// Enable foreign device registration with the specified policy.
    pub fn enable_foreign_device_registration(&mut self, policy: ForeignDevicePolicy) {
        self.foreign_device_policy = Some(policy);
    }

    /// Set or clear the foreign device registration policy.
    pub fn set_foreign_device_policy(&mut self, policy: Option<ForeignDevicePolicy>) {
        self.foreign_device_policy = policy;
    }

    /// Current foreign device registration policy, if enabled.
    pub fn foreign_device_policy(&self) -> Option<&ForeignDevicePolicy> {
        self.foreign_device_policy.as_ref()
    }

    /// Operational counters for Foreign Device Table management.
    pub fn fdt_counters(&self) -> FdtCounters {
        self.counters
    }

    // -----------------------------------------------------------------------
    // BDT management
    // -----------------------------------------------------------------------

    /// Maximum number of entries in the Broadcast Distribution Table.
    pub const MAX_BDT_ENTRIES: usize = 128;

    /// Replace the entire BDT.
    ///
    /// Central commit choke point for issue #529: validates and canonicalizes
    /// the complete candidate before changing `self.bdt`. Byte-identical
    /// duplicates collapse in stable first-seen order; the same `(ip, port)`
    /// with different masks rejects the whole candidate. The local BBMD is
    /// auto-inserted as `/32` when absent. Any invalid, conflicting, or
    /// over-capacity candidate returns `Error::Encoding` and preserves the
    /// previously committed table exactly.
    pub fn set_bdt(&mut self, entries: Vec<BdtEntry>) -> Result<(), Error> {
        let cap = entries.len().min(Self::MAX_BDT_ENTRIES);
        let mut seen: HashMap<([u8; 4], u16), [u8; 4]> = HashMap::with_capacity(cap);
        let mut canonical: Vec<BdtEntry> = Vec::with_capacity(cap);
        for entry in entries {
            validate_bdt_entry(&entry)?;
            match seen.entry((entry.ip, entry.port)) {
                Entry::Occupied(o) => {
                    if *o.get() != entry.broadcast_mask {
                        return Err(bdt_invalid(&entry, "conflicting masks"));
                    }
                }
                Entry::Vacant(v) => {
                    v.insert(entry.broadcast_mask);
                    canonical.push(entry);
                }
            }
        }
        let has_self = canonical
            .iter()
            .any(|e| e.ip == self.local_ip && e.port == self.local_port);
        let effective_len = canonical.len() + usize::from(!has_self);
        if effective_len > Self::MAX_BDT_ENTRIES {
            return Err(Error::Encoding(format!(
                "BDT size {effective_len} exceeds maximum of {} after self-entry insertion",
                Self::MAX_BDT_ENTRIES
            )));
        }
        self.bdt = canonical;
        self.ensure_self_in_bdt();
        Ok(())
    }

    /// Ensure the local BBMD is included in its own BDT (spec J.4.2).
    fn ensure_self_in_bdt(&mut self) {
        let has_self = self
            .bdt
            .iter()
            .any(|e| e.ip == self.local_ip && e.port == self.local_port);
        if !has_self {
            self.bdt.push(BdtEntry {
                ip: self.local_ip,
                port: self.local_port,
                broadcast_mask: [0xff, 0xff, 0xff, 0xff],
            });
        }
    }

    /// Get the current BDT.
    pub fn bdt(&self) -> &[BdtEntry] {
        &self.bdt
    }

    /// Encode the BDT for a Read-BDT-ACK payload.
    pub fn encode_bdt(&self, buf: &mut BytesMut) {
        buf.reserve(self.bdt.len() * BDT_ENTRY_SIZE);
        for entry in &self.bdt {
            buf.put_slice(&entry.ip);
            buf.put_u16(entry.port);
            buf.put_slice(&entry.broadcast_mask);
        }
    }

    /// Decode a BDT from wire bytes (Write-BDT payload or Read-BDT-ACK payload).
    pub fn decode_bdt(data: &[u8]) -> Result<Vec<BdtEntry>, Error> {
        if !data.len().is_multiple_of(BDT_ENTRY_SIZE) {
            return Err(Error::decoding(
                0,
                format!(
                    "BDT data length {} not a multiple of {}",
                    data.len(),
                    BDT_ENTRY_SIZE
                ),
            ));
        }
        let count = data.len() / BDT_ENTRY_SIZE;
        if count > Self::MAX_BDT_ENTRIES {
            return Err(Error::decoding(
                0,
                format!(
                    "BDT entry count {} exceeds maximum of {}",
                    count,
                    Self::MAX_BDT_ENTRIES
                ),
            ));
        }
        let mut entries = Vec::with_capacity(count);
        for chunk in data.chunks_exact(BDT_ENTRY_SIZE) {
            entries.push(BdtEntry {
                ip: [chunk[0], chunk[1], chunk[2], chunk[3]],
                port: u16::from_be_bytes([chunk[4], chunk[5]]),
                broadcast_mask: [chunk[6], chunk[7], chunk[8], chunk[9]],
            });
        }
        Ok(entries)
    }

    // -----------------------------------------------------------------------
    // FDT management
    // -----------------------------------------------------------------------

    /// Maximum number of entries in the Foreign Device Table.
    pub const MAX_FDT_ENTRIES: usize = 128;

    /// Register or re-register a foreign device at the current wall-clock instant.
    pub fn register_foreign_device(&mut self, ip: [u8; 4], port: u16, ttl: u16) -> BvlcResultCode {
        self.register_foreign_device_at(ip, port, ttl, Instant::now())
    }

    /// Register or re-register a foreign device at the given `now` instant.
    pub fn register_foreign_device_at(
        &mut self,
        ip: [u8; 4],
        port: u16,
        ttl: u16,
        now: Instant,
    ) -> BvlcResultCode {
        let policy = match &self.foreign_device_policy {
            Some(p) => p.clone(),
            None => {
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
        };

        // Purge expired entries before evaluating admission or capacity
        self.purge_expired_at(now);

        // Validate TTL bounds
        if ttl == 0 || ttl < policy.min_ttl || ttl > policy.max_ttl {
            self.counters.registrations_rejected += 1;
            return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
        }

        // Check source ACL if configured
        if let Some(allowed) = &policy.allowed_sources {
            if !allowed.contains(&ip) {
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
        }

        // Check registration rate limits
        if self.rate_tracker.is_rate_exceeded(
            ip,
            now,
            policy.rate_window,
            policy.registration_rate_per_source,
            policy.registration_rate_global,
        ) {
            self.counters.registrations_rejected += 1;
            return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
        }

        let existing_idx = self.fdt.iter().position(|e| e.ip == ip && e.port == port);

        if existing_idx.is_none() {
            // Check per-source quota for new entries
            let current_for_ip = self.fdt.iter().filter(|e| e.ip == ip).count();
            if current_for_ip >= policy.max_entries_per_source {
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }

            // Check capacity pressure and reserved capacity
            let is_reserved = policy.reserved_sources.contains(&ip);
            let effective_limit = if is_reserved {
                Self::MAX_FDT_ENTRIES
            } else {
                Self::MAX_FDT_ENTRIES.saturating_sub(policy.reserved_capacity)
            };

            if self.fdt.len() >= effective_limit {
                self.counters.capacity_exhausted += 1;
                self.counters.registrations_rejected += 1;
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
        }

        // Admitted: record in rate tracker and mutate table
        self.rate_tracker.record(ip);
        if let Some(idx) = existing_idx {
            self.fdt[idx].ttl = ttl;
            self.fdt[idx].registered_at = now;
        } else {
            self.fdt.push(FdtEntry {
                ip,
                port,
                ttl,
                registered_at: now,
            });
        }

        self.counters.registrations_accepted += 1;
        BvlcResultCode::SUCCESSFUL_COMPLETION
    }

    /// Delete a foreign device entry.
    pub fn delete_foreign_device(&mut self, ip: [u8; 4], port: u16) -> BvlcResultCode {
        let before = self.fdt.len();
        self.fdt.retain(|e| !(e.ip == ip && e.port == port));
        if self.fdt.len() < before {
            BvlcResultCode::SUCCESSFUL_COMPLETION
        } else {
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
        }
    }

    /// Purge expired FDT entries at the given instant and return the number removed.
    pub fn purge_expired_at(&mut self, now: Instant) -> usize {
        let before = self.fdt.len();
        self.fdt.retain(|e| !e.is_expired_at(now));
        let removed = before - self.fdt.len();
        self.counters.registrations_expired += removed as u64;
        removed
    }

    /// Purge expired FDT entries and return the number removed.
    pub fn purge_expired(&mut self) -> usize {
        self.purge_expired_at(Instant::now())
    }

    /// Get the current FDT (purges expired entries first).
    pub fn fdt(&mut self) -> &[FdtEntry] {
        self.purge_expired();
        &self.fdt
    }

    /// Encode the FDT for a Read-FDT-ACK payload.
    pub fn encode_fdt(&mut self, buf: &mut BytesMut) {
        self.purge_expired();
        buf.reserve(self.fdt.len() * FDT_ENTRY_SIZE);
        for entry in &self.fdt {
            buf.put_slice(&entry.ip);
            buf.put_u16(entry.port);
            buf.put_u16(entry.ttl);
            buf.put_u16(entry.seconds_remaining());
        }
    }

    // -----------------------------------------------------------------------
    // Source validation helpers
    // -----------------------------------------------------------------------

    /// Check if a sender is a BDT peer.
    pub fn is_bdt_peer(&self, ip: [u8; 4], port: u16) -> bool {
        self.bdt.iter().any(|e| e.ip == ip && e.port == port)
    }

    /// Whether a Forwarded-NPDU from this BDT peer still needs a local subnet broadcast.
    pub fn forwarded_npdu_needs_local_broadcast(&self, ip: [u8; 4], port: u16) -> bool {
        self.bdt
            .iter()
            .find(|e| e.ip == ip && e.port == port)
            .is_some_and(|e| e.broadcast_mask == [0xff, 0xff, 0xff, 0xff])
    }

    /// Check if a sender is a registered (non-expired) foreign device.
    pub fn is_registered_foreign_device(&mut self, ip: [u8; 4], port: u16) -> bool {
        self.purge_expired();
        self.fdt.iter().any(|e| e.ip == ip && e.port == port)
    }

    // -----------------------------------------------------------------------
    // Management ACL
    // -----------------------------------------------------------------------

    /// Check whether a source IP is allowed to perform Delete-FDT-Entry.
    /// An empty ACL denies all sources (fail closed).
    pub fn is_management_allowed(&self, source_ip: &[u8; 4]) -> bool {
        self.management_acl.contains(source_ip)
    }

    /// Set the Delete-FDT-Entry management ACL. An empty list denies all sources.
    pub fn set_management_acl(&mut self, acl: Vec<[u8; 4]>) {
        self.management_acl = acl;
    }

    #[cfg(test)]
    pub(crate) fn backdate_foreign_device_for_test(
        &mut self,
        ip: [u8; 4],
        port: u16,
        elapsed: Duration,
    ) {
        let entry = self
            .fdt
            .iter_mut()
            .find(|entry| entry.ip == ip && entry.port == port)
            .expect("foreign device entry exists");
        entry.registered_at = Instant::now() - elapsed;
    }

    #[cfg(test)]
    pub(crate) fn fdt_len_for_test(&self) -> usize {
        self.fdt.len()
    }

    // -----------------------------------------------------------------------
    // Forwarding targets
    // -----------------------------------------------------------------------

    /// Get all (ip, port) targets for forwarding a broadcast, excluding the
    /// source device and the local BBMD itself. BDT entries use directed
    /// broadcast: `target = entry.ip | !entry.broadcast_mask`.
    /// Purges expired FDT entries.
    pub fn forwarding_targets(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
    ) -> Vec<([u8; 4], u16)> {
        self.forwarding_targets_at(exclude_ip, exclude_port, Instant::now())
    }

    /// Get all (ip, port) targets for forwarding a broadcast relative to `now`,
    /// excluding the source device and the local BBMD itself. BDT entries use directed
    /// broadcast: `target = entry.ip | !entry.broadcast_mask`.
    /// Purges expired FDT entries and caps FDT targets to `max_fdt_fanout`.
    pub fn forwarding_targets_at(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
        now: Instant,
    ) -> Vec<([u8; 4], u16)> {
        self.purge_expired_at(now);
        let mut targets = Vec::new();
        let mut seen = HashSet::new();

        for entry in &self.bdt {
            // Skip self
            if entry.ip == self.local_ip && entry.port == self.local_port {
                continue;
            }
            // Skip the original sender
            if entry.ip == exclude_ip && entry.port == exclude_port {
                continue;
            }
            let directed_broadcast = [
                entry.ip[0] | !entry.broadcast_mask[0],
                entry.ip[1] | !entry.broadcast_mask[1],
                entry.ip[2] | !entry.broadcast_mask[2],
                entry.ip[3] | !entry.broadcast_mask[3],
            ];
            let target = (directed_broadcast, entry.port);
            if !seen.insert(target) {
                self.counters.destinations_deduplicated += 1;
            } else {
                targets.push(target);
            }
        }

        let max_fdt_fanout = self
            .foreign_device_policy
            .as_ref()
            .map_or(32, |p| p.max_fdt_fanout);

        let mut fdt_count = 0;
        for entry in &self.fdt {
            if entry.ip == exclude_ip && entry.port == exclude_port {
                continue;
            }
            let target = (entry.ip, entry.port);
            if !seen.insert(target) {
                self.counters.destinations_deduplicated += 1;
                continue;
            }
            if fdt_count >= max_fdt_fanout {
                self.counters.fanout_budget_reached += 1;
                break;
            }
            targets.push(target);
            fdt_count += 1;
        }

        targets
    }

    /// Get all (ip, port) FDT targets for forwarding, excluding `(exclude_ip, exclude_port)`.
    /// Purges expired entries, caps results to `max_fdt_fanout`, and increments
    /// `fanout_budget_reached` if capped.
    pub fn fdt_forwarding_targets(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
    ) -> Vec<([u8; 4], u16)> {
        self.fdt_forwarding_targets_at(exclude_ip, exclude_port, Instant::now())
    }

    /// Get all (ip, port) FDT targets for forwarding relative to `now`, excluding `(exclude_ip, exclude_port)`.
    /// Purges expired entries, caps results to `max_fdt_fanout`, and increments
    /// `fanout_budget_reached` if capped.
    pub fn fdt_forwarding_targets_at(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
        now: Instant,
    ) -> Vec<([u8; 4], u16)> {
        self.purge_expired_at(now);

        let max_fdt_fanout = self
            .foreign_device_policy
            .as_ref()
            .map_or(32, |p| p.max_fdt_fanout);

        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        for entry in &self.fdt {
            if entry.ip == exclude_ip && entry.port == exclude_port {
                continue;
            }
            let target = (entry.ip, entry.port);
            if !seen.insert(target) {
                self.counters.destinations_deduplicated += 1;
                continue;
            }
            if targets.len() >= max_fdt_fanout {
                self.counters.fanout_budget_reached += 1;
                break;
            }
            targets.push(target);
        }

        targets
    }
}

#[cfg(test)]
mod foreign_device_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod validation_tests;

//! BBMD (BACnet/IP Broadcast Management Device) state and tables.
//!
//! Manages the Broadcast Distribution Table (BDT) and Foreign Device Table
//! (FDT) per ASHRAE 135-2020 Annex J. Pure state/logic — no async or I/O.

use std::time::{Duration, Instant};

use bacnet_types::enums::BvlcResultCode;
use bacnet_types::error::Error;
use bytes::{BufMut, BytesMut};

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
    /// Grace period in seconds added beyond TTL before expiry (per J.5.2.3).
    const GRACE_PERIOD: u64 = 30;

    /// Whether this entry has expired (TTL + grace period).
    pub fn is_expired(&self) -> bool {
        let total = self.ttl as u64 + Self::GRACE_PERIOD;
        self.registered_at.elapsed() > Duration::from_secs(total)
    }

    /// Seconds remaining of the TTL (wire-facing, per Clause J.5.2.3).
    pub fn seconds_remaining(&self) -> u16 {
        let elapsed = self.registered_at.elapsed().as_secs();
        (self.ttl as u64).saturating_sub(elapsed) as u16
    }
}

/// BBMD state — BDT and FDT tables with forwarding logic.
#[derive(Debug)]
pub struct BbmdState {
    bdt: Vec<BdtEntry>,
    fdt: Vec<FdtEntry>,
    local_ip: [u8; 4],
    local_port: u16,
    /// Allowed source IPs for management operations (Write-BDT, Delete-FDT).
    /// Empty = all sources allowed (legacy/default behavior).
    management_acl: Vec<[u8; 4]>,
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
        }
    }

    // -----------------------------------------------------------------------
    // BDT management
    // -----------------------------------------------------------------------

    /// Maximum number of entries in the Broadcast Distribution Table.
    pub const MAX_BDT_ENTRIES: usize = 128;

    /// Replace the entire BDT.
    ///
    /// Returns an error if the number of entries exceeds `MAX_BDT_ENTRIES`.
    pub fn set_bdt(&mut self, entries: Vec<BdtEntry>) -> Result<(), Error> {
        if entries.len() > Self::MAX_BDT_ENTRIES {
            return Err(Error::Encoding(format!(
                "BDT size {} exceeds maximum of {}",
                entries.len(),
                Self::MAX_BDT_ENTRIES
            )));
        }
        self.bdt = entries;
        Ok(())
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
    const MAX_FDT_ENTRIES: usize = 512;

    /// Register or re-register a foreign device.
    pub fn register_foreign_device(&mut self, ip: [u8; 4], port: u16, ttl: u16) -> BvlcResultCode {
        // Accept any TTL per J.4.3; re-registration interval is clamped on the sender side.
        // Update existing or insert new
        if let Some(entry) = self.fdt.iter_mut().find(|e| e.ip == ip && e.port == port) {
            entry.ttl = ttl;
            entry.registered_at = Instant::now();
        } else {
            if self.fdt.len() >= Self::MAX_FDT_ENTRIES {
                return BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
            }
            self.fdt.push(FdtEntry {
                ip,
                port,
                ttl,
                registered_at: Instant::now(),
            });
        }
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

    /// Purge expired FDT entries.
    pub fn purge_expired(&mut self) {
        self.fdt.retain(|e| !e.is_expired());
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

    /// Check if a sender is a registered (non-expired) foreign device.
    pub fn is_registered_foreign_device(&mut self, ip: [u8; 4], port: u16) -> bool {
        self.purge_expired();
        self.fdt.iter().any(|e| e.ip == ip && e.port == port)
    }

    // -----------------------------------------------------------------------
    // Management ACL
    // -----------------------------------------------------------------------

    /// Check whether a source IP is allowed to perform management operations
    /// (Write-BDT, Delete-FDT-Entry). Returns `true` if the ACL is empty
    /// (all allowed) or the IP is in the ACL.
    pub fn is_management_allowed(&self, source_ip: &[u8; 4]) -> bool {
        self.management_acl.is_empty() || self.management_acl.contains(source_ip)
    }

    /// Set the management ACL. An empty list means all sources are allowed.
    pub fn set_management_acl(&mut self, acl: Vec<[u8; 4]>) {
        self.management_acl = acl;
    }

    // -----------------------------------------------------------------------
    // Forwarding targets
    // -----------------------------------------------------------------------

    /// Get all (ip, port) targets for forwarding a broadcast, excluding the
    /// source device and the local BBMD itself. BDT entries use directed
    /// broadcast per J.4.2.2: `target = entry.ip | !entry.broadcast_mask`.
    /// The local BBMD's own BDT entry is skipped per Annex J.4.2.2 to
    /// prevent self-forwarding loops.
    /// Purges expired FDT entries.
    pub fn forwarding_targets(
        &mut self,
        exclude_ip: [u8; 4],
        exclude_port: u16,
    ) -> Vec<([u8; 4], u16)> {
        self.purge_expired();
        let mut targets = Vec::new();

        // BDT peers: compute directed broadcast per J.4.2.2
        // target = entry.ip | !entry.broadcast_mask
        for entry in &self.bdt {
            // Skip self per Annex J.4.2.2
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
            targets.push((directed_broadcast, entry.port));
        }

        // FDT entries — unicast directly
        for entry in &self.fdt {
            if entry.ip == exclude_ip && entry.port == exclude_port {
                continue;
            }
            targets.push((entry.ip, entry.port));
        }

        targets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bbmd() -> BbmdState {
        BbmdState::new([192, 168, 1, 1], 0xBAC0)
    }

    #[test]
    fn bdt_set_and_get() {
        let mut bbmd = make_bbmd();
        let entries = vec![
            BdtEntry {
                ip: [192, 168, 1, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 255],
            },
            BdtEntry {
                ip: [192, 168, 2, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 255],
            },
        ];
        bbmd.set_bdt(entries.clone()).unwrap();
        assert_eq!(bbmd.bdt().len(), 2);
        assert_eq!(bbmd.bdt()[0], entries[0]);
    }

    #[test]
    fn bdt_encode_decode_round_trip() {
        let entries = vec![
            BdtEntry {
                ip: [10, 0, 1, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 0],
            },
            BdtEntry {
                ip: [10, 0, 2, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 0],
            },
        ];
        let mut bbmd = make_bbmd();
        bbmd.set_bdt(entries.clone()).unwrap();

        let mut buf = BytesMut::new();
        bbmd.encode_bdt(&mut buf);
        assert_eq!(buf.len(), 20); // 2 * 10 bytes

        let decoded = BbmdState::decode_bdt(&buf).unwrap();
        assert_eq!(decoded, entries);
    }

    #[test]
    fn bdt_decode_invalid_length() {
        assert!(BbmdState::decode_bdt(&[0; 7]).is_err());
    }

    #[test]
    fn register_and_lookup_foreign_device() {
        let mut bbmd = make_bbmd();
        let result = bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
        assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
        assert_eq!(bbmd.fdt().len(), 1);
        assert_eq!(bbmd.fdt()[0].ip, [10, 0, 0, 5]);
        assert_eq!(bbmd.fdt()[0].ttl, 60);
    }

    #[test]
    fn re_register_updates_existing() {
        let mut bbmd = make_bbmd();
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 120);
        assert_eq!(bbmd.fdt().len(), 1);
        assert_eq!(bbmd.fdt()[0].ttl, 120);
    }

    #[test]
    fn delete_foreign_device() {
        let mut bbmd = make_bbmd();
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
        let result = bbmd.delete_foreign_device([10, 0, 0, 5], 0xBAC0);
        assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
        assert!(bbmd.fdt().is_empty());
    }

    #[test]
    fn delete_nonexistent_foreign_device_naks() {
        let mut bbmd = make_bbmd();
        let result = bbmd.delete_foreign_device([10, 0, 0, 5], 0xBAC0);
        assert_eq!(
            result,
            BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
        );
    }

    #[test]
    fn expired_entries_purged() {
        let mut bbmd = make_bbmd();
        // Insert an entry that's past TTL + grace period (0 + 30 = 30s, elapsed 40s)
        bbmd.fdt.push(FdtEntry {
            ip: [10, 0, 0, 5],
            port: 0xBAC0,
            ttl: 0,
            registered_at: Instant::now() - Duration::from_secs(40),
        });
        assert!(bbmd.fdt().is_empty());
    }

    #[test]
    fn fdt_encode() {
        let mut bbmd = make_bbmd();
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
        let mut buf = BytesMut::new();
        bbmd.encode_fdt(&mut buf);
        assert_eq!(buf.len(), FDT_ENTRY_SIZE);
        // IP
        assert_eq!(&buf[0..4], &[10, 0, 0, 5]);
        // Port
        assert_eq!(u16::from_be_bytes([buf[4], buf[5]]), 0xBAC0);
        // TTL
        assert_eq!(u16::from_be_bytes([buf[6], buf[7]]), 60);
    }

    #[test]
    fn forwarding_targets_excludes_source() {
        let mut bbmd = BbmdState::new([192, 168, 1, 1], 0xBAC0);
        bbmd.set_bdt(vec![
            BdtEntry {
                ip: [192, 168, 1, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 255],
            },
            BdtEntry {
                ip: [192, 168, 2, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 255],
            },
        ])
        .unwrap();
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);

        // Source is some device on our subnet (not us and not a BDT peer)
        let targets = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);

        // Should include: BDT peer [192.168.2.1] + FDT [10.0.0.5]
        // Self BDT [192.168.1.1] is excluded per Annex J.4.2.2
        // Full mask (255.255.255.255) means directed broadcast = unicast IP
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&([192, 168, 2, 1], 0xBAC0)));
        assert!(targets.contains(&([10, 0, 0, 5], 0xBAC0)));
    }

    #[test]
    fn forwarding_targets_uses_broadcast_mask() {
        let mut bbmd = BbmdState::new([192, 168, 1, 1], 0xBAC0);
        bbmd.set_bdt(vec![
            BdtEntry {
                ip: [192, 168, 1, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 0],
            },
            BdtEntry {
                ip: [192, 168, 2, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 0],
            },
        ])
        .unwrap();

        let targets = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);
        // Self entry [192.168.1.1] excluded per J.4.2.2, only peer remains
        assert_eq!(targets.len(), 1);
        assert!(targets.contains(&([192, 168, 2, 255], 0xBAC0)));
    }

    #[test]
    fn forwarding_targets_unicast_with_full_mask() {
        let mut bbmd = BbmdState::new([192, 168, 1, 1], 0xBAC0);
        bbmd.set_bdt(vec![
            BdtEntry {
                ip: [192, 168, 1, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 255],
            },
            BdtEntry {
                ip: [10, 0, 0, 1],
                port: 0xBAC0,
                broadcast_mask: [255, 255, 255, 255],
            },
        ])
        .unwrap();

        let targets = bbmd.forwarding_targets([192, 168, 1, 100], 0xBAC0);
        // Self entry excluded per J.4.2.2, only the remote peer remains
        assert_eq!(targets.len(), 1);
        assert!(targets.contains(&([10, 0, 0, 1], 0xBAC0)));
    }

    #[test]
    fn ttl_accepted_as_is() {
        let mut bbmd = make_bbmd();
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 1);
        assert_eq!(bbmd.fdt()[0].ttl, 1); // accepted as-is per J.4.3
    }

    #[test]
    fn fdt_grace_period() {
        let mut bbmd = make_bbmd();
        // Insert entry that expired based on TTL alone but within grace period
        bbmd.fdt.push(FdtEntry {
            ip: [10, 0, 0, 5],
            port: 0xBAC0,
            ttl: 60,
            registered_at: Instant::now() - Duration::from_secs(70), // 10s past TTL, but within 30s grace
        });
        assert!(
            !bbmd.fdt().is_empty(),
            "should still be alive during grace period"
        );
    }

    #[test]
    fn is_bdt_peer_check() {
        let mut bbmd = make_bbmd();
        bbmd.set_bdt(vec![BdtEntry {
            ip: [10, 0, 0, 1],
            port: 0xBAC0,
            broadcast_mask: [255, 255, 255, 255],
        }])
        .unwrap();
        assert!(bbmd.is_bdt_peer([10, 0, 0, 1], 0xBAC0));
        assert!(!bbmd.is_bdt_peer([10, 0, 0, 2], 0xBAC0));
    }

    #[test]
    fn is_registered_foreign_device_check() {
        let mut bbmd = make_bbmd();
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
        assert!(bbmd.is_registered_foreign_device([10, 0, 0, 5], 0xBAC0));
        assert!(!bbmd.is_registered_foreign_device([10, 0, 0, 6], 0xBAC0));
    }

    #[test]
    fn seconds_remaining_does_not_exceed_ttl() {
        let mut bbmd = make_bbmd();
        bbmd.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60);
        let remaining = bbmd.fdt()[0].seconds_remaining();
        assert!(
            remaining <= 60,
            "seconds_remaining ({remaining}) must not exceed TTL (60)"
        );
    }

    #[test]
    fn management_acl_empty_allows_all() {
        let bbmd = make_bbmd();
        assert!(bbmd.is_management_allowed(&[10, 0, 0, 1]));
        assert!(bbmd.is_management_allowed(&[192, 168, 1, 1]));
    }

    #[test]
    fn management_acl_restricts_to_listed_ips() {
        let mut bbmd = make_bbmd();
        bbmd.set_management_acl(vec![[10, 0, 0, 1], [10, 0, 0, 2]]);
        assert!(bbmd.is_management_allowed(&[10, 0, 0, 1]));
        assert!(bbmd.is_management_allowed(&[10, 0, 0, 2]));
        assert!(!bbmd.is_management_allowed(&[10, 0, 0, 3]));
        assert!(!bbmd.is_management_allowed(&[192, 168, 1, 1]));
    }

    #[test]
    fn management_acl_clear_restores_open() {
        let mut bbmd = make_bbmd();
        bbmd.set_management_acl(vec![[10, 0, 0, 1]]);
        assert!(!bbmd.is_management_allowed(&[10, 0, 0, 2]));
        bbmd.set_management_acl(Vec::new());
        assert!(bbmd.is_management_allowed(&[10, 0, 0, 2]));
    }
}

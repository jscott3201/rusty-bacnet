//! BACnet/IPv6 VMACs: deriving and generating this node's (H.7.2), and the
//! table that maps its peers' VMACs to their B/IPv6 addresses (U.5).
//!
//! The table is plain state, driven with explicit instants, so that
//! [`Neighbours`](super::resolve::Neighbours) can keep it and its pending
//! resolutions under one lock, and tests can step through time.

use std::collections::HashMap;
use std::net::SocketAddrV6;
use std::time::Duration;

use bacnet_types::bip6_vmac::{VmacHex, device_instance_vmac};
use bacnet_types::error::Error;
use tokio::time::Instant;

use super::Bip6Vmac;

/// Most peers the table maps. A new peer past this evicts the one heard from
/// least recently, so a node that meets more peers than this can still reach
/// the newest ones.
pub(super) const MAX_VMAC_TABLE_ENTRIES: usize = 4096;
/// How long an address heard from a peer directly outranks one a
/// Forwarded-NPDU or Forwarded-Address-Resolution carries for it.
pub(super) const DIRECT_PRECEDENCE: Duration = Duration::from_secs(60);
/// How long a VMAC whose resolution failed fails a send at once.
pub(super) const UNREACHABLE_FOR: Duration = Duration::from_secs(10);
/// Most VMACs held as unreachable; the oldest goes first.
pub(super) const MAX_UNREACHABLE: usize = 256;
/// How long a peer may stay silent before a send to it asks again whether
/// it is still there, and the least time between two such checks.
pub(super) const REVALIDATE_AFTER: Duration = Duration::from_secs(300);
/// The VMAC that device instance `instance` gives a B/IPv6 node, as
/// [`device_instance_vmac`] derives it (H.7.2). Fails for an instance above
/// 4194303.
///
/// ```
/// use bacnet_transport::bip6::{vmac_from_device_instance, Bip6Vmac};
///
/// assert_eq!(vmac_from_device_instance(1234)?, Bip6Vmac::new([0x00, 0x04, 0xD2]));
/// assert!(vmac_from_device_instance(4_194_304).is_err());
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
pub fn vmac_from_device_instance(instance: u32) -> Result<Bip6Vmac, Error> {
    device_instance_vmac(instance)
        .map(Bip6Vmac::new)
        .ok_or_else(|| Error::Encoding("BACnet Device instance must be in 0..=4194303".to_string()))
}

/// What starts a B/IPv6 VMAC written as text: see [`parse_vmac_text`].
pub const VMAC_TEXT_PREFIX: &str = "bip6:";

/// Read a B/IPv6 VMAC written as text, the way the CLI and the Python
/// bindings name a B/IPv6 node: `bip6:` and a device instance in decimal,
/// 0 to 4194303, whose VMAC it is (H.7.2), or `bip6:0x` (or `0X`) and six hex
/// digits for any VMAC, such as a random one. [`format_vmac_text`] writes
/// the hex form, which this reads back.
///
/// Fails with [`Error::OutOfRange`] for an instance past 4194303 and with
/// [`Error::Encoding`] for any other text; both messages say what to write.
///
/// ```
/// use bacnet_transport::bip6::{format_vmac_text, parse_vmac_text, Bip6Vmac};
///
/// let vmac = parse_vmac_text("bip6:1234")?;
/// assert_eq!(vmac, Bip6Vmac::new([0x00, 0x04, 0xD2]));
/// assert_eq!(parse_vmac_text("bip6:0x0004d2")?, vmac);
/// assert_eq!(format_vmac_text(vmac), "bip6:0x0004D2");
/// assert!(parse_vmac_text("bip6:4194304").is_err());
/// assert!(parse_vmac_text("00:04:d2").is_err());
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
pub fn parse_vmac_text(text: &str) -> Result<Bip6Vmac, Error> {
    let malformed = || {
        Error::Encoding(format!(
            "'{text}' is not a B/IPv6 VMAC: write bip6:<device instance 0..=4194303> \
             or bip6:0x<6 hex digits>"
        ))
    };
    let body = text.strip_prefix(VMAC_TEXT_PREFIX).ok_or_else(malformed)?;
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(malformed());
        }
        let value = u32::from_str_radix(hex, 16).map_err(|_| malformed())?;
        let [_, high, middle, low] = value.to_be_bytes();
        return Ok(Bip6Vmac::new([high, middle, low]));
    }
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    // More digits than a u32 holds are out of range like any instance past
    // 4194303.
    let instance = body.parse::<u32>().unwrap_or(u32::MAX);
    vmac_from_device_instance(instance).map_err(|_| {
        Error::OutOfRange(format!(
            "B/IPv6 device instance must be in 0..=4194303, not {body}"
        ))
    })
}

/// A B/IPv6 VMAC as text, `bip6:0x0004D2`, which [`parse_vmac_text`] reads
/// back.
pub fn format_vmac_text(vmac: Bip6Vmac) -> String {
    format!("{VMAC_TEXT_PREFIX}{}", VmacHex(vmac.octets()))
}

/// Generate a Random Device Instance VMAC for collision resolution (Clause H.7.2).
pub fn generate_random_vmac() -> Result<Bip6Vmac, Error> {
    let mut vmac = [0u8; 3];
    getrandom::fill(&mut vmac).map_err(|error| {
        Error::Encoding(format!("failed to generate Random Instance VMAC: {error}"))
    })?;
    vmac[0] = (vmac[0] & 0x3F) | 0x40;
    Ok(Bip6Vmac::new(vmac))
}

/// How the table heard of a peer's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Heard {
    /// From a datagram the peer sent from that address.
    Direct,
    /// From an address a Forwarded-NPDU or a Forwarded-Address-Resolution
    /// carries, which whoever sent the message chose.
    Carried,
}

/// One peer.
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Where the peer is.
    addr: SocketAddrV6,
    /// When the table last learned of the peer, by either path.
    heard: Instant,
    /// When the peer was last heard directly at `addr`.
    direct: Option<Instant>,
    /// When a send last started a check that the peer is still there.
    revalidated: Option<Instant>,
}

/// The peers' VMACs and addresses, and the VMACs recently found
/// unreachable.
#[derive(Debug, Default)]
pub(super) struct VmacTable {
    /// This node's own VMAC, which is never learned.
    own: Option<Bip6Vmac>,
    entries: HashMap<Bip6Vmac, Entry>,
    /// Which VMAC owns each exact address: one at a time.
    owners: HashMap<SocketAddrV6, Bip6Vmac>,
    /// VMACs whose resolution failed, and when they were marked.
    unreachable: HashMap<Bip6Vmac, Instant>,
}

impl VmacTable {
    /// Set this node's own VMAC, which the table then refuses to learn.
    pub(super) fn set_own(&mut self, own: Option<Bip6Vmac>) {
        self.own = own;
        if let Some(own) = own {
            self.forget(own);
        }
    }

    /// Forget every peer, and every VMAC held as unreachable.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.owners.clear();
        self.unreachable.clear();
    }

    /// Learn that `vmac` is at `addr`, heard at `now`, and say whether the
    /// table took it. It refuses this node's own VMAC, and an address
    /// carried in a message where the peer, or another VMAC at that
    /// address, was heard directly within [`DIRECT_PRECEDENCE`]: such a
    /// message would otherwise let its sender steer this node's unicasts.
    /// Taking a peer clears it as unreachable, replaces any other VMAC at
    /// the same address, and when the table is full evicts the peer heard
    /// from least recently.
    pub(super) fn learn(
        &mut self,
        vmac: Bip6Vmac,
        addr: SocketAddrV6,
        heard: Heard,
        now: Instant,
    ) -> bool {
        if self.own == Some(vmac) {
            return false;
        }
        let recent = |entry: &Entry| {
            entry
                .direct
                .is_some_and(|at| now.saturating_duration_since(at) < DIRECT_PRECEDENCE)
        };
        let owner = self
            .owners
            .get(&addr)
            .copied()
            .filter(|owner| *owner != vmac);
        if heard == Heard::Carried {
            let moved = self
                .entries
                .get(&vmac)
                .is_some_and(|entry| entry.addr != addr && recent(entry));
            let taken = owner
                .and_then(|owner| self.entries.get(&owner))
                .is_some_and(recent);
            if moved || taken {
                return false;
            }
        }
        if let Some(owner) = owner {
            self.forget(owner);
        }
        self.unreachable.remove(&vmac);
        if let Some(entry) = self.entries.get_mut(&vmac) {
            if entry.addr != addr {
                self.owners.remove(&entry.addr);
                entry.addr = addr;
                entry.direct = None;
                entry.revalidated = None;
            }
            entry.heard = now;
        } else {
            if self.entries.len() >= MAX_VMAC_TABLE_ENTRIES {
                self.evict_least_recent();
            }
            let entry = Entry {
                addr,
                heard: now,
                direct: None,
                revalidated: None,
            };
            self.entries.insert(vmac, entry);
        }
        if let Some(entry) = self.entries.get_mut(&vmac)
            && heard == Heard::Direct
        {
            entry.direct = Some(now);
        }
        self.owners.insert(addr, vmac);
        true
    }

    fn evict_least_recent(&mut self) {
        let oldest = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.heard)
            .map(|(vmac, _)| *vmac);
        if let Some(oldest) = oldest {
            self.forget(oldest);
        }
    }

    /// Drop `vmac`'s entry, if there is one.
    pub(super) fn forget(&mut self, vmac: Bip6Vmac) {
        if let Some(entry) = self.entries.remove(&vmac)
            && self.owners.get(&entry.addr) == Some(&vmac)
        {
            self.owners.remove(&entry.addr);
        }
    }

    /// Where `vmac` is, if the table knows.
    pub(super) fn lookup(&self, vmac: Bip6Vmac) -> Option<SocketAddrV6> {
        self.entries.get(&vmac).map(|entry| entry.addr)
    }

    /// The one VMAC the table maps to `peer`'s IP address and port, if
    /// exactly one: a scope ID in `peer` must match too. Two VMACs at the
    /// same link-local address on different interfaces answer none.
    pub(super) fn by_address(&self, peer: SocketAddrV6) -> Option<Bip6Vmac> {
        let mut matches = self.entries.iter().filter(|(_, entry)| {
            entry.addr.ip() == peer.ip()
                && entry.addr.port() == peer.port()
                && (peer.scope_id() == 0 || entry.addr.scope_id() == peer.scope_id())
        });
        let (vmac, _) = matches.next()?;
        matches.next().is_none().then_some(*vmac)
    }

    /// Whether a send to `vmac` at `now` should first check the peer is
    /// still there: it has been silent for [`REVALIDATE_AFTER`], and no
    /// check started within that time. Saying yes records the check.
    pub(super) fn revalidation_due(&mut self, vmac: Bip6Vmac, now: Instant) -> bool {
        let Some(entry) = self.entries.get_mut(&vmac) else {
            return false;
        };
        let stale = |at: Instant| now.saturating_duration_since(at) >= REVALIDATE_AFTER;
        let due = stale(entry.heard) && entry.revalidated.is_none_or(stale);
        if due {
            entry.revalidated = Some(now);
        }
        due
    }

    /// Whether `vmac` failed to resolve within [`UNREACHABLE_FOR`] of `now`.
    pub(super) fn is_unreachable(&mut self, vmac: Bip6Vmac, now: Instant) -> bool {
        match self.unreachable.get(&vmac) {
            Some(at) if now.saturating_duration_since(*at) < UNREACHABLE_FOR => true,
            Some(_) => {
                self.unreachable.remove(&vmac);
                false
            }
            None => false,
        }
    }

    /// Hold `vmac` as unreachable from `now`, evicting the oldest such VMAC
    /// when [`MAX_UNREACHABLE`] are held already.
    pub(super) fn mark_unreachable(&mut self, vmac: Bip6Vmac, now: Instant) {
        if !self.unreachable.contains_key(&vmac) && self.unreachable.len() >= MAX_UNREACHABLE {
            let oldest = self
                .unreachable
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(vmac, _)| *vmac);
            if let Some(oldest) = oldest {
                self.unreachable.remove(&oldest);
            }
        }
        self.unreachable.insert(vmac, now);
    }

    /// Move every time the table keeps `by` into the past, as if that long
    /// had gone by, so a test on real sockets needn't wait it out.
    #[cfg(test)]
    pub(super) fn backdate(&mut self, by: Duration) {
        let back = |at: &mut Instant| *at = at.checked_sub(by).unwrap_or(*at);
        for entry in self.entries.values_mut() {
            back(&mut entry.heard);
            for at in [&mut entry.direct, &mut entry.revalidated]
                .into_iter()
                .flatten()
            {
                back(at);
            }
        }
        self.unreachable.values_mut().for_each(back);
    }

    /// How many peers the table maps.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// How many VMACs the table holds as unreachable.
    #[cfg(test)]
    pub(super) fn unreachable_len(&self) -> usize {
        self.unreachable.len()
    }
}

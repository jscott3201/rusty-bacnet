//! The foreign device table (Annex J.5.2), in a fixed array.

use core::fmt;
use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BvlcResultCode, FdtEntries, FdtEntry};
use bacnet_time::{Duration, Instant};

/// How long a BBMD keeps a registration past its time-to-live before it
/// purges it: 30 seconds (Annex J.5.2.3).
pub const GRACE_PERIOD: Duration = Duration::from_secs(30);

/// [`GRACE_PERIOD`] in whole seconds, worked out when the crate is built.
const GRACE_WHOLE_SECONDS: u64 = GRACE_PERIOD.as_secs();

/// One foreign device's registration, as a BBMD keeps it.
///
/// It is `#[non_exhaustive]`, so a new detail of a registration isn't a
/// breaking change: outside this crate, build one for
/// [`Fdt::restore`] with [`Registration::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Registration {
    /// The foreign device's B/IP address: where the registration came from.
    pub address: SocketAddrV4,
    /// The time-to-live it registered with, in seconds.
    pub ttl: u16,
    /// When it registered or last renewed.
    pub registered_at: Instant,
}

/// An unused slot of storage.
const EMPTY_SLOT: Registration = Registration::new(
    SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
    0,
    Instant::ZERO,
);

impl Registration {
    /// The registration of `address` for `ttl` seconds, made or last
    /// renewed at `registered_at`.
    #[must_use]
    pub const fn new(address: SocketAddrV4, ttl: u16, registered_at: Instant) -> Self {
        Self {
            address,
            ttl,
            registered_at,
        }
    }

    /// How long the registration lasts from `registered_at`: its time-to-live
    /// plus the [`GRACE_PERIOD`].
    #[must_use]
    pub const fn lifetime(&self) -> Duration {
        Duration::from_secs(self.ttl as u64 + GRACE_WHOLE_SECONDS)
    }

    /// Whether the registration has lapsed at `now`: more than its
    /// [`lifetime`](Self::lifetime) has passed since it registered. At
    /// exactly the lifetime it is still live.
    #[must_use]
    pub fn is_expired_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.registered_at) > self.lifetime()
    }

    /// Whole seconds left at `now` before the BBMD purges the registration,
    /// grace period included, as a Read-Foreign-Device-Table-Ack reports
    /// them (Annex J.2.8.1): the lifetime in seconds less the whole seconds
    /// passed, at most 65535. It counts down from time-to-live + 30 at
    /// registration, reads 1 through the last second, and reads 0 from
    /// exactly time-to-live + 30 s, the last instant the registration is
    /// live, on.
    #[must_use]
    pub fn seconds_remaining_at(&self, now: Instant) -> u16 {
        let lifetime_secs = u64::from(self.ttl) + GRACE_WHOLE_SECONDS;
        let passed = now.saturating_duration_since(self.registered_at).as_secs();
        u16::try_from(lifetime_secs.saturating_sub(passed)).unwrap_or(u16::MAX)
    }

    /// The registration as a Read-Foreign-Device-Table-Ack row at `now`.
    #[must_use]
    pub fn wire_entry_at(&self, now: Instant) -> FdtEntry {
        FdtEntry::new(self.address, self.ttl, self.seconds_remaining_at(now))
    }
}

/// The admission rules a BBMD applies to a Register-Foreign-Device besides
/// its table's size (Annex J.5.2.2 leaves them to the BBMD).
///
/// It is `#[non_exhaustive]`, so a new limit isn't a breaking change:
/// outside this crate, start from [`RegistrationLimits::DEFAULT`] and set
/// the fields to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegistrationLimits {
    /// The shortest time-to-live accepted, in seconds. A time-to-live of 0
    /// is always refused.
    pub min_ttl: u16,
    /// The longest time-to-live accepted, in seconds.
    pub max_ttl: u16,
    /// Most registrations one IPv4 address may hold, across its ports.
    pub max_entries_per_source: usize,
    /// Slots only a reserved source may fill: an ordinary source is refused
    /// once the table holds its capacity less these.
    pub reserved_capacity: usize,
}

impl RegistrationLimits {
    /// Time-to-live 1 to 3600 s, 8 registrations per address, nothing
    /// reserved.
    pub const DEFAULT: Self = Self {
        min_ttl: 1,
        max_ttl: 3600,
        max_entries_per_source: 8,
        reserved_capacity: 0,
    };
}

/// [`RegistrationLimits::DEFAULT`].
impl Default for RegistrationLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Whether a registrant may use the slots [`RegistrationLimits::
/// reserved_capacity`] keeps back. Which sources may is the caller's policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reservation {
    /// An ordinary source: it may fill all but the reserved slots.
    Ordinary,
    /// A source the reserved slots are kept for: it may fill every slot.
    Reserved,
}

/// An accepted registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// The table had no entry for the address; it has one now.
    New,
    /// The address's entry restarted with the new time-to-live.
    Renewed,
}

/// Why a registration was refused. The table is unchanged apart from the
/// purge that comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistrationRefusal {
    /// The time-to-live is 0 or outside the limits.
    TimeToLive,
    /// The caller's own admission check said no.
    NotAdmitted,
    /// The address's IPv4 address holds its limit of registrations.
    SourceQuota,
    /// No slot is free to this source.
    TableFull,
}

impl RegistrationRefusal {
    /// The BVLC-Result to answer the registrant with: every refusal is a
    /// Register-Foreign-Device NAK.
    #[must_use]
    pub const fn result_code(self) -> BvlcResultCode {
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    }
}

impl fmt::Display for RegistrationRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TimeToLive => "time-to-live outside the accepted range",
            Self::NotAdmitted => "registration not admitted",
            Self::SourceQuota => "too many registrations from this IPv4 address",
            Self::TableFull => "foreign device table full",
        })
    }
}

impl core::error::Error for RegistrationRefusal {}

/// What [`Fdt::register`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub struct RegisterOutcome {
    /// The registration's fate.
    pub result: Result<Registered, RegistrationRefusal>,
    /// Lapsed registrations purged before the request was considered.
    pub purged: usize,
}

/// Storage too small for what was asked of it, counted in entries: the
/// shape of the codec's [`SinkFull`](bacnet_bip_codec::SinkFull), which
/// counts octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageFull {
    /// Entries the call needed room for, in all.
    pub needed: usize,
    /// Entries the storage holds.
    pub available: usize,
}

impl fmt::Display for StorageFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "storage full: need {} entries, have {}",
            self.needed, self.available
        )
    }
}

impl core::error::Error for StorageFull {}

/// A BBMD's foreign device table: up to `N` registrations, one per B/IP
/// address, in the order they first registered.
///
/// Expiry runs on the caller's clock: every call that depends on time takes
/// `now`, and a lapsed registration stays stored, but is never forwarded to
/// or reported, until [`Fdt::purge`] or [`Fdt::register`] removes it.
#[derive(Clone)]
pub struct Fdt<const N: usize> {
    slots: [Registration; N],
    len: usize,
}

/// Shows the registrations in use, not the whole array.
impl<const N: usize> fmt::Debug for Fdt<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.registrations()).finish()
    }
}

impl<const N: usize> Default for Fdt<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Fdt<N> {
    /// Registrations the table holds at most.
    pub const CAPACITY: usize = N;

    /// An empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [EMPTY_SLOT; N],
            len: 0,
        }
    }

    /// Stored registrations, lapsed ones not yet purged included.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether no registration is stored.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The stored registrations, lapsed ones not yet purged included.
    #[must_use]
    pub fn registrations(&self) -> &[Registration] {
        &self.slots[..self.len]
    }

    /// The registrations still live at `now`.
    pub fn live_at(&self, now: Instant) -> impl DoubleEndedIterator<Item = &Registration> + '_ {
        self.registrations()
            .iter()
            .filter(move |entry| !entry.is_expired_at(now))
    }

    /// The stored registration for `address`, lapsed or not.
    #[must_use]
    pub fn get(&self, address: SocketAddrV4) -> Option<&Registration> {
        self.registrations()
            .iter()
            .find(|entry| entry.address == address)
    }

    /// Whether `address` holds a registration that is live at `now`.
    #[must_use]
    pub fn is_registered_at(&self, address: SocketAddrV4, now: Instant) -> bool {
        self.get(address)
            .is_some_and(|entry| !entry.is_expired_at(now))
    }

    /// Handle a Register-Foreign-Device from `source` asking for `ttl`
    /// seconds, at `now` (Annex J.5.2.2, J.5.2.3).
    ///
    /// The steps, in order:
    ///
    /// 1. Purge the registrations lapsed at `now`.
    /// 2. Refuse a time-to-live of 0 or outside `limits`.
    /// 3. Ask `admit`, the caller's own policy (an access list, a rate
    ///    limit): `false` refuses. It is called only for a time-to-live that
    ///    passed.
    /// 4. Renew an existing registration for `source`: its timer restarts
    ///    with the new time-to-live, and no limit below applies.
    /// 5. Otherwise refuse when `source`'s IPv4 address already holds
    ///    `limits.max_entries_per_source`, or when the table has no slot for
    ///    it: an [`Ordinary`](Reservation::Ordinary) source may fill all but
    ///    `limits.reserved_capacity` slots, a
    ///    [`Reserved`](Reservation::Reserved) one all of them. Then append
    ///    it.
    pub fn register(
        &mut self,
        source: SocketAddrV4,
        ttl: u16,
        now: Instant,
        limits: &RegistrationLimits,
        reservation: Reservation,
        admit: impl FnOnce() -> bool,
    ) -> RegisterOutcome {
        let purged = self.purge(now);
        let result = self.admit_and_store(source, ttl, now, limits, reservation, admit);
        RegisterOutcome { result, purged }
    }

    fn admit_and_store(
        &mut self,
        source: SocketAddrV4,
        ttl: u16,
        now: Instant,
        limits: &RegistrationLimits,
        reservation: Reservation,
        admit: impl FnOnce() -> bool,
    ) -> Result<Registered, RegistrationRefusal> {
        if ttl == 0 || ttl < limits.min_ttl || ttl > limits.max_ttl {
            return Err(RegistrationRefusal::TimeToLive);
        }
        if !admit() {
            return Err(RegistrationRefusal::NotAdmitted);
        }
        let registration = Registration::new(source, ttl, now);
        if let Some(slot) = self.slots[..self.len]
            .iter_mut()
            .find(|entry| entry.address == source)
        {
            *slot = registration;
            return Ok(Registered::Renewed);
        }
        let from_source = self
            .registrations()
            .iter()
            .filter(|entry| entry.address.ip() == source.ip())
            .count();
        if from_source >= limits.max_entries_per_source {
            return Err(RegistrationRefusal::SourceQuota);
        }
        let limit = match reservation {
            Reservation::Reserved => N,
            Reservation::Ordinary => N.saturating_sub(limits.reserved_capacity),
        };
        if self.len >= limit {
            return Err(RegistrationRefusal::TableFull);
        }
        self.slots[self.len] = registration;
        self.len += 1;
        Ok(Registered::New)
    }

    /// Remove `address`'s registration, lapsed or not (Annex J.5.2.1.2);
    /// whether there was one.
    pub fn delete(&mut self, address: SocketAddrV4) -> bool {
        self.retain(|entry| entry.address != address) > 0
    }

    /// Remove the registrations lapsed at `now`; how many.
    pub fn purge(&mut self, now: Instant) -> usize {
        self.retain(|entry| !entry.is_expired_at(now))
    }

    /// Store `registration` as given, with no checks: it replaces the entry
    /// for its address, or is appended. For a caller restoring a table it
    /// saved, or placing a registration at a chosen time. Fails only when
    /// the table is full: it needed one entry more than its `N`.
    pub fn restore(&mut self, registration: Registration) -> Result<(), StorageFull> {
        let address = registration.address;
        if let Some(slot) = self.slots[..self.len]
            .iter_mut()
            .find(|entry| entry.address == address)
        {
            *slot = registration;
            return Ok(());
        }
        let Some(slot) = self.slots.get_mut(self.len) else {
            return Err(StorageFull {
                needed: self.len + 1,
                available: N,
            });
        };
        *slot = registration;
        self.len += 1;
        Ok(())
    }

    /// The registrations live at `now` as Read-Foreign-Device-Table-Ack rows
    /// (Annex J.2.8.1), written to `out` and returned as a table the codec
    /// encodes. Fails when `out` can't hold them all, saying how many live
    /// registrations there are.
    pub fn wire_entries<'o>(
        &self,
        now: Instant,
        out: &'o mut [FdtEntry],
    ) -> Result<FdtEntries<'o>, StorageFull> {
        let mut written = 0;
        let mut live = self.live_at(now);
        for entry in live.by_ref() {
            let FdtEntry {
                address,
                ttl,
                remaining,
            } = entry.wire_entry_at(now);
            let Some(row) = out.get_mut(written) else {
                return Err(StorageFull {
                    needed: written + 1 + live.count(),
                    available: out.len(),
                });
            };
            // The address's IP and port one at a time: copied whole, its six
            // octets compile to a memmove call on ARMv6-M, about 1.3 KB of
            // code.
            row.address.set_ip(*address.ip());
            row.address.set_port(address.port());
            row.ttl = ttl;
            row.remaining = remaining;
            written += 1;
        }
        Ok(FdtEntries::from_entries(&out[..written]))
    }

    /// Keep the registrations `keep` accepts, in order; how many went.
    fn retain(&mut self, mut keep: impl FnMut(&Registration) -> bool) -> usize {
        let mut kept = 0;
        for index in 0..self.len {
            let Registration {
                address,
                ttl,
                registered_at,
            } = self.slots[index];
            if keep(&self.slots[index]) {
                // Field by field: a whole-entry copy between two slots of
                // the array compiles to a memmove call, about 1.4 KB of code
                // on a Cortex-M3.
                let slot = &mut self.slots[kept];
                slot.address = address;
                slot.ttl = ttl;
                slot.registered_at = registered_at;
                kept += 1;
            }
        }
        let removed = self.len - kept;
        self.len = kept;
        removed
    }
}

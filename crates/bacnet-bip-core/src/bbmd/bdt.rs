//! The broadcast distribution table (Annex J.4.3), in a fixed array.

use core::fmt;
use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry};

/// The broadcast distribution mask of all ones: forwarded broadcasts go to
/// the peer BBMD itself, which broadcasts them on its subnet (two-hop
/// distribution, Annex J.4.3.1). A peer's subnet mask instead sends them
/// straight to that subnet's directed broadcast address (one-hop).
pub const TWO_HOP_MASK: Ipv4Addr = Ipv4Addr::BROADCAST;

/// An unused row of storage.
const EMPTY_ROW: BdtEntry = BdtEntry::new(
    SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
    Ipv4Addr::UNSPECIFIED,
);

/// Where a broadcast forwarded to `row`'s subnet is sent (Annex J.4.5): the
/// row's address ORed with its inverted mask, at the row's port. A subnet
/// mask gives that subnet's directed broadcast address, and
/// [`TWO_HOP_MASK`] gives the peer BBMD itself.
///
/// ```
/// use core::net::{Ipv4Addr, SocketAddrV4};
/// use bacnet_bip_core::codec::BdtEntry;
/// use bacnet_bip_core::bbmd::distribution_address;
///
/// let peer = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 1), 47808);
/// let one_hop = BdtEntry::new(peer, Ipv4Addr::new(255, 255, 255, 0));
/// assert_eq!(distribution_address(one_hop), SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 255), 47808));
/// let two_hop = BdtEntry::new(peer, Ipv4Addr::BROADCAST);
/// assert_eq!(distribution_address(two_hop), peer);
/// ```
#[must_use]
pub const fn distribution_address(row: BdtEntry) -> SocketAddrV4 {
    let ip = row.address.ip().to_bits() | !row.mask.to_bits();
    SocketAddrV4::new(Ipv4Addr::from_bits(ip), row.address.port())
}

/// Why [`validate_entry`] refused a BDT row.
///
/// The rules are a local security policy (#529), not Annex J's: each keeps a
/// row from turning forwarded broadcasts into traffic for an address that
/// isn't one BBMD or one subnet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryError {
    /// UDP port 0.
    ZeroPort,
    /// The address 0.0.0.0.
    UnspecifiedAddress,
    /// The address 255.255.255.255.
    LimitedBroadcastAddress,
    /// An IPv4 multicast address (224.0.0.0/4).
    MulticastAddress,
    /// A mask whose one bits aren't contiguous from the top.
    NonContiguousMask,
    /// A mask shorter than /8, whose broadcast would reach too wide a range.
    MaskTooWide,
    /// The subnet's network address (all host bits zero) under a mask
    /// shorter than /32.
    NetworkAddress,
    /// The subnet's directed broadcast address (all host bits one) under a
    /// mask shorter than /32.
    DirectedBroadcastAddress,
    /// A row whose [`distribution_address`] is 255.255.255.255.
    LimitedBroadcastTarget,
}

impl EntryError {
    /// A short description, as the host transport's errors print it.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::ZeroPort => "invalid port 0",
            Self::UnspecifiedAddress => "unspecified IP",
            Self::LimitedBroadcastAddress => "limited-broadcast",
            Self::MulticastAddress => "multicast IP",
            Self::NonContiguousMask => "non-contiguous mask",
            Self::MaskTooWide => "mask prefix too wide (< /8)",
            Self::NetworkAddress => "subnet network address",
            Self::DirectedBroadcastAddress => "subnet directed-broadcast",
            Self::LimitedBroadcastTarget => "limited-broadcast target",
        }
    }
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

impl core::error::Error for EntryError {}

/// Why a BDT was refused. A refused table changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BdtError {
    /// A row broke a rule of [`validate_entry`].
    Invalid {
        /// The row.
        entry: BdtEntry,
        /// The rule it broke.
        reason: EntryError,
    },
    /// Two rows for one B/IP address with different masks.
    ConflictingMasks {
        /// The later of the two rows.
        entry: BdtEntry,
    },
    /// Behind a NAT router, a row with a mask other than [`TWO_HOP_MASK`]:
    /// a directed broadcast can't cross the router (Annex J.7.5).
    OneHopBehindNat {
        /// The row.
        entry: BdtEntry,
    },
    /// For a BBMD that serves a B/IP-M group ([`Bdt::set_serves_group`]),
    /// an own row with a mask other than [`TWO_HOP_MASK`]: the peers must
    /// send this BBMD the network's broadcasts itself, for it to pass them
    /// on to the group, rather than to its IP subnet's directed broadcast
    /// address. Annex J.8.3 gives the reason: a member on that subnet would
    /// otherwise get each such broadcast twice, once by the directed
    /// broadcast and once through the group.
    OneHopForGroup {
        /// The row.
        entry: BdtEntry,
    },
    /// Behind a NAT router, a table with no row for this BBMD's global
    /// address, which is how the table names this BBMD there (Annex
    /// J.7.5). It isn't added, since the peers' tables must list it too.
    NoOwnRow {
        /// The global address.
        own: SocketAddrV4,
    },
    /// The distinct rows, with this BBMD's own row where the table needs it
    /// added, would not fit.
    Capacity {
        /// Rows the table would hold.
        needed: usize,
        /// Rows the storage holds.
        available: usize,
    },
}

impl fmt::Display for BdtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { entry, reason } => {
                write!(f, "BDT row {} mask {}: {reason}", entry.address, entry.mask)
            }
            Self::ConflictingMasks { entry } => write!(
                f,
                "BDT rows for {} with conflicting masks (one is {})",
                entry.address, entry.mask
            ),
            Self::OneHopBehindNat { entry } => write!(
                f,
                "BDT row {} mask {}: behind a NAT router every mask must be all ones",
                entry.address, entry.mask
            ),
            Self::OneHopForGroup { entry } => write!(
                f,
                "BDT row {} mask {}: a BBMD that serves a multicast group needs an all-ones mask \
                 on its own row",
                entry.address, entry.mask
            ),
            Self::NoOwnRow { own } => write!(
                f,
                "behind a NAT router the BDT must list the global address {own} as this BBMD's own row"
            ),
            Self::Capacity { needed, available } => {
                write!(
                    f,
                    "BDT of {needed} rows exceeds the capacity of {available}"
                )
            }
        }
    }
}

impl core::error::Error for BdtError {}

/// Check one BDT row against the local security policy (see
/// [`EntryError`]). Contiguous masks of /8 to /32 pass, with any unicast
/// address that isn't its subnet's network or broadcast address.
pub const fn validate_entry(entry: BdtEntry) -> Result<(), EntryError> {
    let ip = entry.address.ip().to_bits();
    let mask = entry.mask.to_bits();
    let host = !mask;
    let contiguous = host & host.wrapping_add(1) == 0;
    let prefix = mask.count_ones();
    let first_octet = ip >> 24;
    let reason = if entry.address.port() == 0 {
        EntryError::ZeroPort
    } else if ip == 0 {
        EntryError::UnspecifiedAddress
    } else if ip == u32::MAX {
        EntryError::LimitedBroadcastAddress
    } else if matches!(first_octet, 224..=239) {
        EntryError::MulticastAddress
    } else if !contiguous {
        EntryError::NonContiguousMask
    } else if prefix < 8 {
        EntryError::MaskTooWide
    } else if prefix < 32 && ip & host == 0 {
        EntryError::NetworkAddress
    } else if prefix < 32 && ip | mask == u32::MAX {
        EntryError::DirectedBroadcastAddress
    } else if ip | host == u32::MAX {
        EntryError::LimitedBroadcastTarget
    } else {
        return Ok(());
    };
    Err(reason)
}

/// What a canonical pass over a candidate table found.
struct Canonical {
    /// Distinct rows written to the output, in first-seen order.
    stored: usize,
    /// Distinct rows in the candidate; more than `stored` when the output
    /// ran out of room.
    distinct: usize,
}

/// Validate `table` and write its distinct rows to `out` in first-seen order.
///
/// Rows identical to an earlier one are dropped; a second row for an address
/// with a different mask refuses the table. Every row is checked even after
/// `out` is full, so a bad row is reported before a lack of room.
fn canonical_rows(table: BdtEntries<'_>, out: &mut [BdtEntry]) -> Result<Canonical, BdtError> {
    let mut stored = 0;
    let mut distinct = 0;
    // The index of the first distinct row that found no room: from there on,
    // earlier rows are looked up in the candidate itself.
    let mut overflow_from = None;
    for (index, entry) in table.iter().enumerate() {
        if let Err(reason) = validate_entry(entry) {
            return Err(BdtError::Invalid { entry, reason });
        }
        let earlier = match overflow_from {
            None => out[..stored]
                .iter()
                .find(|row| row.address == entry.address)
                .copied(),
            Some(from) => out[..stored]
                .iter()
                .copied()
                .chain(table.iter().take(index).skip(from))
                .find(|row| row.address == entry.address),
        };
        match earlier {
            Some(row) if row.mask != entry.mask => {
                return Err(BdtError::ConflictingMasks { entry });
            }
            Some(_) => {}
            None => {
                distinct += 1;
                match out.get_mut(stored) {
                    Some(slot) if overflow_from.is_none() => {
                        *slot = entry;
                        stored += 1;
                    }
                    _ => {
                        overflow_from.get_or_insert(index);
                    }
                }
            }
        }
    }
    Ok(Canonical { stored, distinct })
}

/// Validate a candidate BDT and write its distinct rows to `out`, in
/// first-seen order; returns how many.
///
/// Every row must pass [`validate_entry`]. A row identical to an earlier one
/// is dropped, and two rows for one B/IP address with different masks
/// refuse the table ([`BdtError::ConflictingMasks`]). When the distinct rows
/// don't fit in `out` the result is [`BdtError::Capacity`], but only after
/// every row has been checked. Unlike [`Bdt::set`], this adds no own row.
pub fn canonicalize(table: BdtEntries<'_>, out: &mut [BdtEntry]) -> Result<usize, BdtError> {
    let found = canonical_rows(table, out)?;
    if found.distinct > found.stored {
        return Err(BdtError::Capacity {
            needed: found.distinct,
            available: out.len(),
        });
    }
    Ok(found.stored)
}

/// A BBMD's broadcast distribution table: up to `N` rows, this BBMD's own
/// row included once [`Bdt::set`] has run (Annex J.4.3.2).
///
/// Rows keep the order they were given in. A table that the caller sets
/// without this BBMD's own address gets a row for it, with
/// [`TWO_HOP_MASK`], at the end; that row follows the address when
/// [`Bdt::set_own_address`] changes it. Behind a NAT router
/// ([`Tables::set_nat`](super::Tables::set_nat)) nothing is added: the
/// table must list the own (global) address, and every mask must be
/// [`TWO_HOP_MASK`]. For a BBMD that serves a B/IP-M group
/// ([`Bdt::set_serves_group`]) the own row must be [`TWO_HOP_MASK`] too.
#[derive(Clone)]
pub struct Bdt<const N: usize> {
    rows: [BdtEntry; N],
    len: usize,
    own: SocketAddrV4,
    /// The last row is the own row that `set` appended.
    own_row_appended: bool,
    /// The own address is a NAT router's global address (Annex J.7.5).
    behind_nat: bool,
    /// The BBMD serves a B/IP-M group, so its own row is two-hop (Annex
    /// J.8.3).
    serves_group: bool,
}

/// Shows the rows in use, not the whole array.
impl<const N: usize> fmt::Debug for Bdt<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bdt")
            .field("own", &self.own)
            .field("rows", &self.rows())
            .field("own_row_appended", &self.own_row_appended)
            .field("behind_nat", &self.behind_nat)
            .field("serves_group", &self.serves_group)
            .finish()
    }
}

impl<const N: usize> Bdt<N> {
    /// Rows the table holds at most.
    pub const CAPACITY: usize = N;

    /// An empty table for the BBMD at `own`, the B/IP address its peers know
    /// it by. It has no rows, not even its own, until [`Bdt::set`].
    #[must_use]
    pub const fn new(own: SocketAddrV4) -> Self {
        Self {
            rows: [EMPTY_ROW; N],
            len: 0,
            own,
            own_row_appended: false,
            behind_nat: false,
            serves_group: false,
        }
    }

    /// An empty table for the BBMD at `own`, behind a NAT router or not,
    /// under this table's group rule.
    pub(super) const fn with_nat(&self, own: SocketAddrV4, behind_nat: bool) -> Self {
        let mut bdt = Self::new(own);
        bdt.behind_nat = behind_nat;
        bdt.serves_group = self.serves_group;
        bdt
    }

    /// Whether the BBMD serves a B/IP-M group, so its own row must be
    /// two-hop ([`Bdt::set_serves_group`]).
    #[must_use]
    pub const fn serves_group(&self) -> bool {
        self.serves_group
    }

    /// Say whether the BBMD serves a B/IP-M group (Annex J.8.3), as a BBMD
    /// whose link's subnet is a multicast group does
    /// ([`LocalSubnet::is_group`](crate::LocalSubnet::is_group)).
    ///
    /// Such a BBMD hears the network's broadcasts from its peers only when
    /// they send them to it, not to its IP subnet's directed broadcast
    /// address, which no group member takes. So its own row, the one every
    /// BDT of the network lists for it, must have [`TWO_HOP_MASK`]: from
    /// now on [`Bdt::set`] and [`Bdt::set_own_address`] refuse a table whose
    /// own row has another mask ([`BdtError::OneHopForGroup`]). An own row
    /// the table appends has it already. Turning it on refuses, changing
    /// nothing, while the table's own row has another mask.
    pub fn set_serves_group(&mut self, serves: bool) -> Result<(), BdtError> {
        if serves {
            Self::check_group_row(self.own, self.rows().iter().copied())?;
        }
        self.serves_group = serves;
        Ok(())
    }

    /// The group rule: `rows`' row for `own`, if any, is two-hop.
    fn check_group_row(
        own: SocketAddrV4,
        mut rows: impl Iterator<Item = BdtEntry>,
    ) -> Result<(), BdtError> {
        match rows.find(|row| row.address == own && row.mask != TWO_HOP_MASK) {
            Some(entry) => Err(BdtError::OneHopForGroup { entry }),
            None => Ok(()),
        }
    }

    /// Whether the own address is a NAT router's global address, so the
    /// NAT rules of [`Bdt::set`] apply.
    #[must_use]
    pub const fn behind_nat(&self) -> bool {
        self.behind_nat
    }

    /// This BBMD's own B/IP address.
    #[must_use]
    pub const fn own_address(&self) -> SocketAddrV4 {
        self.own
    }

    /// The rows, the own row included, as a table the codec encodes (for a
    /// Read-Broadcast-Distribution-Table-Ack, Annex J.2.4).
    #[must_use]
    pub fn entries(&self) -> BdtEntries<'_> {
        BdtEntries::from_entries(self.rows())
    }

    /// The rows, the own row included.
    #[must_use]
    pub fn rows(&self) -> &[BdtEntry] {
        &self.rows[..self.len]
    }

    /// The rows as last set, without an own row [`Bdt::set`] appended.
    #[must_use]
    pub fn configured_rows(&self) -> &[BdtEntry] {
        &self.rows[..self.len - usize::from(self.own_row_appended)]
    }

    /// Number of rows.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the table has no rows.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The row for the BBMD at `address`, if any; the own row counts.
    #[must_use]
    pub fn row(&self, address: SocketAddrV4) -> Option<BdtEntry> {
        self.rows()
            .iter()
            .find(|row| row.address == address)
            .copied()
    }

    /// Whether `address` has a row; the own row counts.
    #[must_use]
    pub fn contains(&self, address: SocketAddrV4) -> bool {
        self.row(address).is_some()
    }

    /// Replace the table with `table`, as [`canonicalize`] cleans it, plus a
    /// row for this BBMD's own address with [`TWO_HOP_MASK`] when `table` has
    /// none. Behind a NAT router no own row is added: a table with rows must
    /// list the own (global) address ([`BdtError::NoOwnRow`]), and every
    /// mask must be [`TWO_HOP_MASK`] ([`BdtError::OneHopBehindNat`]). An
    /// empty table is taken there: a BBMD with no peers that serves only
    /// foreign devices (Annex J.7.5). A BBMD that serves a B/IP-M group
    /// needs [`TWO_HOP_MASK`] on its own row ([`BdtError::OneHopForGroup`]).
    /// On error the table is unchanged.
    pub fn set(&mut self, table: BdtEntries<'_>) -> Result<(), BdtError> {
        let mut next = self.with_nat(self.own, self.behind_nat);
        let found = canonical_rows(table, &mut next.rows)?;
        let has_own = table.iter().any(|row| row.address == self.own);
        if self.behind_nat {
            if let Some(entry) = table.iter().find(|row| row.mask != TWO_HOP_MASK) {
                return Err(BdtError::OneHopBehindNat { entry });
            }
            if !has_own && found.distinct > 0 {
                return Err(BdtError::NoOwnRow { own: self.own });
            }
        }
        if self.serves_group {
            Self::check_group_row(self.own, table.iter())?;
        }
        let needed = found.distinct + usize::from(!has_own && !self.behind_nat);
        if needed > N {
            return Err(BdtError::Capacity {
                needed,
                available: N,
            });
        }
        next.len = found.stored;
        if !self.behind_nat {
            next.ensure_own_row();
        }
        *self = next;
        Ok(())
    }

    /// Change this BBMD's own B/IP address, keeping the rows.
    ///
    /// An own row that [`Bdt::set`] appended moves to the new address. A row
    /// the table lists for the old address stays, as an ordinary peer row,
    /// and the new address gets a row unless it has one. On error, for want
    /// of room for that row, nothing changes. Behind a NAT router `own` is
    /// a new global address, which the table must already list
    /// ([`BdtError::NoOwnRow`] otherwise). For a BBMD that serves a B/IP-M
    /// group a row the table lists for `own` must be two-hop
    /// ([`BdtError::OneHopForGroup`]).
    pub fn set_own_address(&mut self, own: SocketAddrV4) -> Result<(), BdtError> {
        if own == self.own {
            return Ok(());
        }
        if self.behind_nat {
            // The table must list the new global address itself.
            return self.reconfigure(own, true, None);
        }
        let configured = self.configured_rows();
        if self.serves_group {
            Self::check_group_row(own, configured.iter().copied())?;
        }
        let has_own = configured.iter().any(|row| row.address == own);
        let needed = configured.len() + usize::from(!has_own);
        if needed > N {
            return Err(BdtError::Capacity {
                needed,
                available: N,
            });
        }
        if self.own_row_appended {
            self.len -= 1;
            self.own_row_appended = false;
        }
        self.own = own;
        self.ensure_own_row();
        Ok(())
    }

    /// Move to the own address `own`, behind a NAT router or not, keeping
    /// the configured rows but `drop`'s: they are set again under the new
    /// rules, so an own row appended for the old address goes, and on error
    /// nothing changes. A table never set takes the new address and rules
    /// alone.
    pub(super) fn reconfigure(
        &mut self,
        own: SocketAddrV4,
        behind_nat: bool,
        drop: Option<SocketAddrV4>,
    ) -> Result<(), BdtError> {
        // Only a table behind NAT is empty once set.
        if self.is_empty() && !self.behind_nat {
            self.own = own;
            self.behind_nat = behind_nat;
            return Ok(());
        }
        let mut next = self.with_nat(own, behind_nat);
        next.set_without(BdtEntries::from_entries(self.configured_rows()), drop)?;
        *self = next;
        Ok(())
    }

    /// [`Bdt::set`] with `table` less any row for `drop`.
    pub(super) fn set_without(
        &mut self,
        table: BdtEntries<'_>,
        drop: Option<SocketAddrV4>,
    ) -> Result<(), BdtError> {
        let Some(drop) = drop else {
            return self.set(table);
        };
        let mut kept = [EMPTY_ROW; N];
        let mut len = 0;
        for row in table.iter().filter(|row| row.address != drop) {
            // A candidate longer than the storage is refused as `set` would.
            let Some(slot) = kept.get_mut(len) else {
                return self.set(table);
            };
            *slot = row;
            len += 1;
        }
        self.set(BdtEntries::from_entries(&kept[..len]))
    }

    /// Append the own row when the table lacks one. The callers have checked
    /// that it fits.
    fn ensure_own_row(&mut self) {
        if !self.contains(self.own) {
            let BdtEntry { address, mask } = BdtEntry::new(self.own, TWO_HOP_MASK);
            let row = &mut self.rows[self.len];
            // The address's IP and port one at a time: copied whole, its six
            // octets compile to a memmove call on ARMv6-M, about 1.3 KB of
            // code.
            row.address.set_ip(*address.ip());
            row.address.set_port(address.port());
            row.mask = mask;
            self.len += 1;
            self.own_row_appended = true;
        }
    }
}

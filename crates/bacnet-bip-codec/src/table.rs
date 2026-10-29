//! B/IP addresses, and the BDT and FDT entries built on them.

use core::fmt;
use core::iter::FusedIterator;
use core::net::{Ipv4Addr, SocketAddrV4};

use crate::{
    BDT_ENTRY_LENGTH, BIP_ADDRESS_LENGTH, BvllError, BvllSink, FDT_ENTRY_LENGTH, SinkFull,
};

/// Octets in one table entry: both tables use the same size (Annex J.2.2.1,
/// J.2.8.1), so one table type serves both.
const ENTRY_LENGTH: usize = BDT_ENTRY_LENGTH;
const _: () = assert!(FDT_ENTRY_LENGTH == ENTRY_LENGTH);

/// The six octets of a B/IP address: the IPv4 address, then the UDP port,
/// both most significant octet first (Annex J.1.2).
///
/// ```
/// use core::net::{Ipv4Addr, SocketAddrV4};
/// use bacnet_bip_codec::{address_from_octets, address_to_octets};
///
/// let address = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 10), 47808);
/// assert_eq!(address_to_octets(address), [192, 168, 1, 10, 0xBA, 0xC0]);
/// assert_eq!(address_from_octets([192, 168, 1, 10, 0xBA, 0xC0]), address);
/// ```
#[must_use]
pub const fn address_to_octets(address: SocketAddrV4) -> [u8; BIP_ADDRESS_LENGTH] {
    let [a, b, c, d] = address.ip().octets();
    let [high, low] = address.port().to_be_bytes();
    [a, b, c, d, high, low]
}

/// The B/IP address six octets hold (Annex J.1.2).
#[must_use]
pub const fn address_from_octets(octets: [u8; BIP_ADDRESS_LENGTH]) -> SocketAddrV4 {
    let [a, b, c, d, high, low] = octets;
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), u16::from_be_bytes([high, low]))
}

/// The B/IP address in `octets`, which must be exactly six octets long.
pub fn address_from_slice(octets: &[u8]) -> Result<SocketAddrV4, BvllError> {
    match <[u8; BIP_ADDRESS_LENGTH]>::try_from(octets) {
        Ok(octets) => Ok(address_from_octets(octets)),
        Err(_) => Err(BvllError::AddressLength {
            length: octets.len(),
        }),
    }
}

/// One row of a broadcast distribution table: a BBMD's B/IP address and the
/// broadcast distribution mask that says how to reach its subnet (Annex
/// J.2.2.1, J.4.3.2).
///
/// A mask of all ones sends forwarded broadcasts to the BBMD itself
/// (two-hop); the subnet's own mask sends them to its directed broadcast
/// address (one-hop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BdtEntry {
    /// The BBMD's B/IP address.
    pub address: SocketAddrV4,
    /// The broadcast distribution mask.
    pub mask: Ipv4Addr,
}

impl BdtEntry {
    /// Construct an entry.
    #[must_use]
    pub const fn new(address: SocketAddrV4, mask: Ipv4Addr) -> Self {
        Self { address, mask }
    }

    /// The entry's ten wire octets: address, then mask.
    #[must_use]
    pub const fn to_octets(self) -> [u8; ENTRY_LENGTH] {
        let [a, b, c, d, high, low] = address_to_octets(self.address);
        let [m0, m1, m2, m3] = self.mask.octets();
        [a, b, c, d, high, low, m0, m1, m2, m3]
    }

    /// The entry ten wire octets hold.
    #[must_use]
    pub const fn from_octets(octets: [u8; ENTRY_LENGTH]) -> Self {
        let [a, b, c, d, high, low, m0, m1, m2, m3] = octets;
        Self {
            address: address_from_octets([a, b, c, d, high, low]),
            mask: Ipv4Addr::new(m0, m1, m2, m3),
        }
    }
}

/// One row of a foreign device table as Read-Foreign-Device-Table-Ack
/// returns it (Annex J.2.8.1): the registrant's B/IP address, the
/// time-to-live it registered with, and the seconds left before the BBMD
/// purges it, which include the 30-second grace period of J.5.2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FdtEntry {
    /// The foreign device's B/IP address.
    pub address: SocketAddrV4,
    /// The time-to-live, in seconds, the device registered with.
    pub ttl: u16,
    /// Seconds until the BBMD purges the entry.
    pub remaining: u16,
}

impl FdtEntry {
    /// Construct an entry.
    #[must_use]
    pub const fn new(address: SocketAddrV4, ttl: u16, remaining: u16) -> Self {
        Self {
            address,
            ttl,
            remaining,
        }
    }

    /// The entry's ten wire octets: address, time-to-live, then remaining
    /// seconds.
    #[must_use]
    pub const fn to_octets(self) -> [u8; ENTRY_LENGTH] {
        let [a, b, c, d, high, low] = address_to_octets(self.address);
        let [t0, t1] = self.ttl.to_be_bytes();
        let [r0, r1] = self.remaining.to_be_bytes();
        [a, b, c, d, high, low, t0, t1, r0, r1]
    }

    /// The entry ten wire octets hold.
    #[must_use]
    pub const fn from_octets(octets: [u8; ENTRY_LENGTH]) -> Self {
        let [a, b, c, d, high, low, t0, t1, r0, r1] = octets;
        Self {
            address: address_from_octets([a, b, c, d, high, low]),
            ttl: u16::from_be_bytes([t0, t1]),
            remaining: u16::from_be_bytes([r0, r1]),
        }
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::BdtEntry {}
    impl Sealed for super::FdtEntry {}
}

/// A table entry type: [`BdtEntry`] or [`FdtEntry`]. Sealed.
pub trait TableEntry: Copy + sealed::Sealed {
    /// The entry ten wire octets hold.
    fn from_wire(octets: [u8; ENTRY_LENGTH]) -> Self;
    /// The entry's ten wire octets.
    fn to_wire(self) -> [u8; ENTRY_LENGTH];
}

impl TableEntry for BdtEntry {
    fn from_wire(octets: [u8; ENTRY_LENGTH]) -> Self {
        Self::from_octets(octets)
    }
    fn to_wire(self) -> [u8; ENTRY_LENGTH] {
        self.to_octets()
    }
}

impl TableEntry for FdtEntry {
    fn from_wire(octets: [u8; ENTRY_LENGTH]) -> Self {
        Self::from_octets(octets)
    }
    fn to_wire(self) -> [u8; ENTRY_LENGTH] {
        self.to_octets()
    }
}

/// A table carried in a BVLL payload, borrowed rather than copied: either
/// the wire octets of a decoded message, or entries a caller wants to
/// encode. Both read the same through [`Entries::iter`], and two tables
/// compare equal when their entries do.
#[derive(Clone, Copy)]
pub struct Entries<'a, T> {
    repr: Repr<'a, T>,
}

#[derive(Clone, Copy)]
enum Repr<'a, T> {
    Wire(&'a [[u8; ENTRY_LENGTH]]),
    Typed(&'a [T]),
}

/// A broadcast distribution table (Annex J.2.2.1, J.2.4.1).
pub type BdtEntries<'a> = Entries<'a, BdtEntry>;
/// A foreign device table (Annex J.2.8.1).
pub type FdtEntries<'a> = Entries<'a, FdtEntry>;

impl<'a, T: TableEntry> Entries<'a, T> {
    /// Read a table from its wire octets, which must be a whole number of
    /// 10-octet entries. An empty table is allowed. The octets are borrowed,
    /// not copied.
    ///
    /// ```
    /// use bacnet_bip_codec::{BdtEntries, BvllError};
    ///
    /// let octets = [10, 0, 0, 1, 0xBA, 0xC0, 255, 255, 255, 255];
    /// let table = BdtEntries::from_wire(&octets)?;
    /// assert_eq!(table.len(), 1);
    /// assert_eq!(table.iter().next().unwrap().address.port(), 0xBAC0);
    ///
    /// assert_eq!(
    ///     BdtEntries::from_wire(&octets[..9]).unwrap_err(),
    ///     BvllError::TableLength { length: 9, entry_length: 10 },
    /// );
    /// # Ok::<(), BvllError>(())
    /// ```
    pub fn from_wire(octets: &'a [u8]) -> Result<Self, BvllError> {
        match octets.as_chunks::<ENTRY_LENGTH>() {
            (entries, []) => Ok(Self {
                repr: Repr::Wire(entries),
            }),
            _ => Err(BvllError::TableLength {
                length: octets.len(),
                entry_length: ENTRY_LENGTH,
            }),
        }
    }

    /// A table of these entries, for encoding.
    #[must_use]
    pub const fn from_entries(entries: &'a [T]) -> Self {
        Self {
            repr: Repr::Typed(entries),
        }
    }

    /// Number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self.repr {
            Repr::Wire(entries) => entries.len(),
            Repr::Typed(entries) => entries.len(),
        }
    }

    /// Whether the table has no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entries, in wire order.
    #[must_use]
    pub fn iter(&self) -> EntryIter<'a, T> {
        EntryIter {
            repr: match self.repr {
                Repr::Wire(entries) => IterRepr::Wire(entries.iter()),
                Repr::Typed(entries) => IterRepr::Typed(entries.iter()),
            },
        }
    }

    /// Octets the table takes on the wire.
    pub(crate) const fn wire_length(&self) -> usize {
        self.len() * ENTRY_LENGTH
    }

    /// Write the table's octets to `sink`.
    pub(crate) fn write_to<S: BvllSink + ?Sized>(&self, sink: &mut S) -> Result<(), SinkFull> {
        match self.repr {
            Repr::Wire(entries) => sink.write(entries.as_flattened()),
            Repr::Typed(entries) => entries
                .iter()
                .try_for_each(|entry| sink.write(&entry.to_wire())),
        }
    }
}

impl<T: TableEntry + PartialEq> PartialEq for Entries<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl<T: TableEntry + Eq> Eq for Entries<'_, T> {}

impl<T: TableEntry + fmt::Debug> fmt::Debug for Entries<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a, T: TableEntry> IntoIterator for Entries<'a, T> {
    type Item = T;
    type IntoIter = EntryIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: TableEntry> IntoIterator for &Entries<'a, T> {
    type Item = T;
    type IntoIter = EntryIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over a table's entries; see [`Entries::iter`].
#[derive(Debug, Clone)]
pub struct EntryIter<'a, T> {
    repr: IterRepr<'a, T>,
}

#[derive(Debug, Clone)]
enum IterRepr<'a, T> {
    Wire(core::slice::Iter<'a, [u8; ENTRY_LENGTH]>),
    Typed(core::slice::Iter<'a, T>),
}

impl<T: TableEntry> Iterator for EntryIter<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match &mut self.repr {
            IterRepr::Wire(entries) => entries.next().map(|octets| T::from_wire(*octets)),
            IterRepr::Typed(entries) => entries.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.repr {
            IterRepr::Wire(entries) => entries.size_hint(),
            IterRepr::Typed(entries) => entries.size_hint(),
        }
    }
}

impl<T: TableEntry> DoubleEndedIterator for EntryIter<'_, T> {
    fn next_back(&mut self) -> Option<T> {
        match &mut self.repr {
            IterRepr::Wire(entries) => entries.next_back().map(|octets| T::from_wire(*octets)),
            IterRepr::Typed(entries) => entries.next_back().copied(),
        }
    }
}

impl<T: TableEntry> ExactSizeIterator for EntryIter<'_, T> {}

impl<T: TableEntry> FusedIterator for EntryIter<'_, T> {}

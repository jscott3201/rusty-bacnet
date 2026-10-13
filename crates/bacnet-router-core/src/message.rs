//! The network-layer messages a router reads and sends (Clause 6.4).
//!
//! [`MessageType`] names their types and [`RejectReason`] the reasons a
//! reject gives, as open sets of wire codes: the values `bacnet-types`
//! gives them, with room for any other octet. The views below read a
//! message's data, the payload of an [`Npdu`](crate::npdu::Npdu) whose
//! message type names it, in place: each parses the whole payload or
//! refuses it with a [`DataError`], so a router never acts on part of a
//! malformed message. The writers fill the caller's buffer and return the
//! length, or a [`DataError`] with nothing written.
//!
//! The views check layout only. Policy, such as which network numbers a
//! router may learn, stays with the router.

mod codes;

use core::fmt;

pub use codes::{MessageType, RejectReason};

/// Why a view refused a message's data, or a writer wrote nothing.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataError {
    /// Read: the data isn't the length its layout gives it. It ends inside
    /// a field, or goes on past the last one.
    Length {
        /// The data's length, in octets.
        length: usize,
    },
    /// Write: the buffer is shorter than the data.
    NoRoom {
        /// The data's length, in octets.
        needed: usize,
        /// The buffer's length.
        available: usize,
    },
    /// Write: more routing entries than a one-octet count can state.
    TooManyEntries {
        /// The entries offered.
        count: usize,
    },
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { length } => {
                write!(f, "{length} octets of message data don't fit its layout")
            }
            Self::NoRoom { needed, available } => {
                write!(
                    f,
                    "message data of {needed} octets doesn't fit in {available}"
                )
            }
            Self::TooManyEntries { count } => {
                write!(f, "{count} routing entries, more than a count can state")
            }
        }
    }
}

impl core::error::Error for DataError {}

/// The refusal of `payload` for its length.
const fn bad_length(payload: &[u8]) -> DataError {
    DataError::Length {
        length: payload.len(),
    }
}

/// A list of two-octet network numbers: the data of I-Am-Router,
/// Router-Busy and Router-Available, and a scoped Who-Is-Router's one
/// network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Networks<'a> {
    octets: &'a [u8],
}

impl<'a> Networks<'a> {
    /// The empty list: for flow control, every network the sender serves.
    pub const EMPTY: Networks<'static> = Networks { octets: &[] };

    /// The list in `payload`, refused for an odd length, which ends inside
    /// a network number.
    pub const fn parse(payload: &'a [u8]) -> Result<Self, DataError> {
        if payload.len().is_multiple_of(2) {
            Ok(Self { octets: payload })
        } else {
            Err(bad_length(payload))
        }
    }

    /// How many networks it lists.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.octets.len() / 2
    }

    /// Whether it lists none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.octets.is_empty()
    }

    /// The octets, as received.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.octets
    }

    /// Whether `network` is listed.
    #[must_use]
    pub fn contains(&self, network: u16) -> bool {
        self.iter().any(|listed| listed == network)
    }

    /// The networks, in the order listed.
    pub fn iter(&self) -> impl Iterator<Item = u16> + 'a {
        self.octets
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
    }

    /// Write `networks` as a list into `out` and return its length. Nothing
    /// is written unless they all fit. The iterator is cloned to count the
    /// networks before any octet is written.
    pub fn write<I>(networks: I, out: &mut [u8]) -> Result<usize, DataError>
    where
        I: IntoIterator<Item = u16>,
        I::IntoIter: Clone,
    {
        let networks = networks.into_iter();
        let available = out.len();
        let needed = networks.clone().count().saturating_mul(2);
        let out = out
            .get_mut(..needed)
            .ok_or(DataError::NoRoom { needed, available })?;
        for (pair, network) in out.as_chunks_mut::<2>().0.iter_mut().zip(networks) {
            *pair = network.to_be_bytes();
        }
        Ok(needed)
    }
}

/// What a Who-Is-Router asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhoIsRouter {
    /// Every network reachable through the router (no data).
    All,
    /// One network.
    Network(u16),
}

impl WhoIsRouter {
    /// Read a Who-Is-Router's data: nothing, or one network number. Any
    /// other length is refused, so a bad tail never widens a scoped
    /// question into one about every network.
    pub const fn parse(payload: &[u8]) -> Result<Self, DataError> {
        match *payload {
            [] => Ok(Self::All),
            [high, low] => Ok(Self::Network(u16::from_be_bytes([high, low]))),
            _ => Err(bad_length(payload)),
        }
    }
}

/// A Reject-Message-To-Network's data: a reason and a network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reject {
    /// The reason.
    pub reason: RejectReason,
    /// The network the refused message was for.
    pub network: u16,
}

impl Reject {
    /// Read exactly a reason octet and a network number.
    pub const fn parse(payload: &[u8]) -> Result<Self, DataError> {
        match *payload {
            [reason, high, low] => Ok(Self {
                reason: RejectReason::from_raw(reason),
                network: u16::from_be_bytes([high, low]),
            }),
            _ => Err(bad_length(payload)),
        }
    }

    /// The data, as sent.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 3] {
        let [high, low] = self.network.to_be_bytes();
        [self.reason.to_raw(), high, low]
    }
}

/// Read three octets as a two-octet network and the octet that follows.
const fn network_and_octet(payload: &[u8]) -> Result<(u16, u8), DataError> {
    match *payload {
        [high, low, octet] => Ok((u16::from_be_bytes([high, low]), octet)),
        _ => Err(bad_length(payload)),
    }
}

/// An I-Could-Be-Router's data: a network and a performance index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ICouldBeRouter {
    /// The network a connection would reach.
    pub network: u16,
    /// The sender's own rating of that connection; lower is better.
    pub performance_index: u8,
}

impl ICouldBeRouter {
    /// Read exactly a network number and the index.
    pub const fn parse(payload: &[u8]) -> Result<Self, DataError> {
        match network_and_octet(payload) {
            Ok((network, performance_index)) => Ok(Self {
                network,
                performance_index,
            }),
            Err(error) => Err(error),
        }
    }
}

/// An Establish-Connection's data: a network and how long to hold the
/// connection without traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstablishConnection {
    /// The network to connect to.
    pub network: u16,
    /// Seconds to hold an idle connection; 0 holds it for good.
    pub termination_time: u8,
}

impl EstablishConnection {
    /// Read exactly a network number and the time.
    pub const fn parse(payload: &[u8]) -> Result<Self, DataError> {
        match network_and_octet(payload) {
            Ok((network, termination_time)) => Ok(Self {
                network,
                termination_time,
            }),
            Err(error) => Err(error),
        }
    }
}

/// A Disconnect-Connection's data: the network whose connection to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisconnectConnection {
    /// The network.
    pub network: u16,
}

impl DisconnectConnection {
    /// Read exactly one network number.
    pub const fn parse(payload: &[u8]) -> Result<Self, DataError> {
        match *payload {
            [high, low] => Ok(Self {
                network: u16::from_be_bytes([high, low]),
            }),
            _ => Err(bad_length(payload)),
        }
    }
}

/// A Network-Number-Is's data: a network number and whether it was
/// configured or learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkNumberIs {
    /// The local network's number.
    pub network: u16,
    /// 1 when configured, 0 when learned from another Network-Number-Is.
    pub flag: u8,
}

impl NetworkNumberIs {
    /// The flag of a configured number.
    pub const CONFIGURED: u8 = 1;

    /// Read exactly a network number and the flag.
    pub const fn parse(payload: &[u8]) -> Result<Self, DataError> {
        match network_and_octet(payload) {
            Ok((network, flag)) => Ok(Self { network, flag }),
            Err(error) => Err(error),
        }
    }

    /// Whether the number was configured.
    #[must_use]
    pub const fn is_configured(self) -> bool {
        self.flag == Self::CONFIGURED
    }

    /// The data, as sent.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 3] {
        let [high, low] = self.network.to_be_bytes();
        [high, low, self.flag]
    }
}

/// What an Initialize-Routing-Table, or its Ack, carries (Clause 6.4.7): a
/// count, then that many entries of a network, a Port ID and Port Info of
/// a stated length.
///
/// A count of 0 in an Initialize-Routing-Table is a query for the whole
/// table. An Ack to an update carries no data at all, which
/// [`RoutingEntries::parse`] refuses: the caller tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingEntries<'a> {
    count: u8,
    entries: &'a [u8],
}

/// One entry of [`RoutingEntries`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingEntry<'a> {
    /// The network.
    pub network: u16,
    /// The sender's Port ID for it; 0 asks that the network's rows go.
    pub port_id: u8,
    /// The Port Info, as received (dial-up details this stack doesn't use).
    pub port_info: &'a [u8],
}

impl<'a> RoutingEntries<'a> {
    /// Octets an entry takes with no Port Info.
    pub const ENTRY_LEN: usize = 4;

    /// Read the count and exactly that many entries, refused when the
    /// payload is empty, ends inside an entry, or runs past the last one.
    pub fn parse(payload: &'a [u8]) -> Result<Self, DataError> {
        let (&count, entries) = payload.split_first().ok_or(bad_length(payload))?;
        let mut rest = entries;
        for _ in 0..count {
            rest = split_entry(rest).ok_or(bad_length(payload))?.1;
        }
        if !rest.is_empty() {
            return Err(bad_length(payload));
        }
        Ok(Self { count, entries })
    }

    /// The count, which is 0 for a query.
    #[must_use]
    pub const fn count(&self) -> u8 {
        self.count
    }

    /// The entries, in order.
    pub fn iter(&self) -> impl Iterator<Item = RoutingEntry<'a>> + 'a {
        let mut rest = self.entries;
        core::iter::from_fn(move || {
            let (entry, after) = split_entry(rest)?;
            rest = after;
            Some(entry)
        })
    }

    /// Write a count and an entry with no Port Info for each (network,
    /// Port ID) pair into `out` and return the length. Nothing is written
    /// unless they all fit and number at most 255. The iterator is cloned
    /// to count the entries before any octet is written.
    pub fn write<I>(entries: I, out: &mut [u8]) -> Result<usize, DataError>
    where
        I: IntoIterator<Item = (u16, u8)>,
        I::IntoIter: Clone,
    {
        let entries = entries.into_iter();
        let count = entries.clone().count();
        let count = u8::try_from(count).map_err(|_| DataError::TooManyEntries { count })?;
        let needed = 1 + usize::from(count) * Self::ENTRY_LEN;
        let available = out.len();
        let room = out
            .get_mut(..needed)
            .ok_or(DataError::NoRoom { needed, available })?;
        // At least the count octet: `needed` is 1 or more.
        let (first, rest) = room
            .split_first_mut()
            .ok_or(DataError::NoRoom { needed, available })?;
        *first = count;
        // Four octets each: ENTRY_LEN.
        for (slot, (network, port_id)) in rest.as_chunks_mut::<4>().0.iter_mut().zip(entries) {
            let [high, low] = network.to_be_bytes();
            *slot = [high, low, port_id, 0];
        }
        Ok(needed)
    }
}

/// The entry at the start of `octets` and what follows it.
fn split_entry(octets: &[u8]) -> Option<(RoutingEntry<'_>, &[u8])> {
    let [high, low, port_id, info_len, ref rest @ ..] = *octets else {
        return None;
    };
    let port_info = rest.get(..usize::from(info_len))?;
    let entry = RoutingEntry {
        network: u16::from_be_bytes([high, low]),
        port_id,
        port_info,
    };
    Some((entry, &rest[port_info.len()..]))
}

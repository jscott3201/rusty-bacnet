//! B/IP broadcast management (Annex J) as plain data: BBMD table rows, what
//! reading a table gave, the breadth-first survey that follows each BDT to
//! the BBMDs it lists, and the consistency matrix over the tables read
//! ([`matrix`]). The TUI's Network screen and the MCP server's network map
//! share it; [`Engine`](crate::engine::Engine) reads the tables.
//!
//! Only the read side of BVLL is used: Read-Broadcast-Distribution-Table
//! and Read-Foreign-Device-Table (Annex J.2.3 and J.2.7). Nothing here
//! writes a table or registers.

pub mod matrix;

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_transport::bbmd::{BdtEntry, FdtEntryWire};
use bacnet_transport::bip::BvlcExchangeError;
use bacnet_transport::bvll::BvlcResultCode;
use futures_util::stream::{FuturesUnordered, StreamExt};

pub use self::matrix::{Cell, Finding, Matrix, Probe};

/// The most BBMDs a survey reads. A larger set comes back with the rest
/// unread ([`Outcome::Skipped`]).
pub const MAX_SURVEY: usize = 16;

/// The most BBMDs a survey reads at once. Each read takes one of the B/IP
/// transport's eight exchange slots (one per peer and request), so this
/// leaves room for the foreign-device registration and other reads, and
/// silent peers wait out their timeouts together.
pub const SURVEY_AT_ONCE: usize = 4;

/// An FDT entry with less than this many seconds left is near expiry. The
/// BBMD keeps an entry 30 s past its time-to-live (Annex J.5.2.3), so under
/// this the device is already past its own time-to-live without having
/// registered again.
pub const FDT_EXPIRING_S: u16 = 30;

/// One BDT entry: a BBMD and the mask its peers forward to it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BdtRow {
    /// The BBMD's B/IP address.
    pub address: SocketAddrV4,
    /// Its broadcast distribution mask.
    pub mask: Ipv4Addr,
}

impl BdtRow {
    /// True for an all-ones mask: peers send forwarded broadcasts to the
    /// BBMD itself, which broadcasts them on its own subnet (Annex J.4.3.1,
    /// the two-hop method).
    pub fn is_unicast(&self) -> bool {
        self.mask == Ipv4Addr::BROADCAST
    }

    /// Where a peer sends a forwarded broadcast for this entry: the address
    /// with every bit the mask clears set (Annex J.4.5). With an all-ones
    /// mask that is the BBMD itself.
    pub fn forward_to(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.address.ip().to_bits() | !self.mask.to_bits())
    }

    /// One line on what the mask means for forwarding.
    pub fn explain(&self) -> String {
        if self.is_unicast() {
            "peers unicast to it, and it rebroadcasts on its subnet (two-hop)".into()
        } else {
            format!(
                "peers send a directed broadcast to {} (one-hop; routers must pass it)",
                self.forward_to()
            )
        }
    }
}

impl From<&BdtEntry> for BdtRow {
    fn from(entry: &BdtEntry) -> Self {
        Self {
            address: SocketAddrV4::new(Ipv4Addr::from(entry.ip), entry.port),
            mask: Ipv4Addr::from(entry.broadcast_mask),
        }
    }
}

/// One FDT entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdtRow {
    /// The address the BBMD recorded for the foreign device, which is the
    /// source address its registration arrived from.
    pub address: SocketAddrV4,
    /// The time-to-live the device asked for, in seconds.
    pub ttl: u16,
    /// Seconds left before the BBMD drops the entry, as of the read. The
    /// BBMD adds a grace period to the time-to-live (Annex J.5.2.3).
    pub remaining: u16,
}

impl FdtRow {
    /// Whether the entry is near expiry: under [`FDT_EXPIRING_S`] left.
    pub fn expiring(&self) -> bool {
        self.remaining < FDT_EXPIRING_S
    }
}

impl From<&FdtEntryWire> for FdtRow {
    fn from(entry: &FdtEntryWire) -> Self {
        Self {
            address: SocketAddrV4::new(Ipv4Addr::from(entry.ip), entry.port),
            ttl: entry.ttl,
            remaining: entry.seconds_remaining,
        }
    }
}

/// The 6-octet B/IP MAC of `address`.
pub fn mac(address: SocketAddrV4) -> [u8; 6] {
    let [a, b, c, d] = address.ip().octets();
    let [high, low] = address.port().to_be_bytes();
    [a, b, c, d, high, low]
}

/// The address in a 6-octet B/IP MAC.
pub fn address_of(mac: &[u8]) -> Option<SocketAddrV4> {
    let [a, b, c, d, high, low] = <[u8; 6]>::try_from(mac).ok()?;
    Some(SocketAddrV4::new(
        Ipv4Addr::new(a, b, c, d),
        u16::from_be_bytes([high, low]),
    ))
}

/// What reading one of a BBMD's tables gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome<R> {
    /// The table.
    Table(R),
    /// It answered with this NAK result: it doesn't carry out the read, as
    /// a node that is no BBMD answers.
    Nak(BvlcResultCode),
    /// It did not answer, or the request could not be sent.
    Unreachable(String),
    /// Not read: the survey reached its limit, or the caller's budget ran
    /// out first.
    Skipped,
}

impl<R> Outcome<R> {
    /// The table, when one was read.
    pub fn table(&self) -> Option<&R> {
        match self {
            Self::Table(rows) => Some(rows),
            _ => None,
        }
    }

    /// Sort a failed table read into a refusal or no answer (#1822): a NAK
    /// is a refusal, while anything else (no answer, a reply the read does
    /// not take, a send failure, a malformed reply) means the BBMD could not
    /// be read.
    pub fn from_error(error: &BvlcExchangeError) -> Self {
        match error {
            BvlcExchangeError::Nak(code) => Self::Nak(*code),
            other => Self::Unreachable(unreachable_reason(other)),
        }
    }
}

/// [`Outcome::from_error`] for a BDT read.
pub fn probe_error(error: &BvlcExchangeError) -> Probe {
    Probe::from_error(error)
}

/// [`unreachable_reason`] for a BBMD that didn't answer in time.
pub const NO_ANSWER: &str = "no answer";

/// Why a BBMD gave no table, as text: no answer in time, or what failed.
pub fn unreachable_reason(error: &BvlcExchangeError) -> String {
    match error {
        BvlcExchangeError::NoReply(_) => NO_ANSWER.into(),
        other => other.to_string(),
    }
}

/// One BBMD a survey read: its BDT (or why there is none) and what the
/// caller's read gave besides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surveyed<X> {
    /// The BBMD.
    pub bbmd: SocketAddrV4,
    /// Its BDT, or why there is none.
    pub probe: Probe,
    /// What else the read gave (the TUI's is nothing, the MCP server's the
    /// FDT).
    pub extra: X,
}

/// Why a BDT entry was not followed: its address is no BBMD a read may go
/// to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotFollowed {
    /// All zeros: a BBMD reports an entry configured by a host name it
    /// couldn't resolve this way (Annex J.4.4.1).
    Unresolved,
    /// An unspecified address or port 0.
    Unspecified,
    /// The limited broadcast address.
    Broadcast,
    /// A subnet's directed broadcast address: a BDT entry always holds a
    /// BBMD's own unicast address, so one that is the local broadcast
    /// address, or has every host bit of its own mask set, is none.
    DirectedBroadcast,
    /// A multicast address.
    Multicast,
    /// A loopback address, while this node isn't on loopback itself.
    Loopback,
    /// This node's own address.
    Own,
}

impl NotFollowed {
    /// The reason as a kebab-case word.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unresolved => "unresolved",
            Self::Unspecified => "unspecified",
            Self::Broadcast => "broadcast",
            Self::DirectedBroadcast => "directed-broadcast",
            Self::Multicast => "multicast",
            Self::Loopback => "loopback",
            Self::Own => "own-address",
        }
    }

    /// Why `row`, a BDT entry, is not read by a node at `here`; `None` when
    /// it may be. BDT entries are device data, so a hostile or broken table
    /// must not turn the survey into a broadcast, a multicast or a send to
    /// this host's other services.
    pub fn check(row: &BdtRow, here: &Here) -> Option<Self> {
        let directed = !row.is_unicast() && row.forward_to() == *row.address.ip();
        Self::check_address(row.address, here).or(directed.then_some(Self::DirectedBroadcast))
    }

    /// [`check`](Self::check) for an address without a mask, such as a
    /// survey's seed.
    pub fn check_address(address: SocketAddrV4, here: &Here) -> Option<Self> {
        let own = here.own;
        let ip = address.ip();
        if ip.is_unspecified() && address.port() == 0 {
            Some(Self::Unresolved)
        } else if ip.is_unspecified() || address.port() == 0 {
            Some(Self::Unspecified)
        } else if ip.is_broadcast() {
            Some(Self::Broadcast)
        } else if ip.is_multicast() {
            Some(Self::Multicast)
        } else if *ip == here.broadcast && here.broadcast != *own.ip() {
            // A "broadcast" address that is our own (a loopback lab's
            // setting) broadcasts nothing.
            Some(Self::DirectedBroadcast)
        } else if address == own {
            Some(Self::Own)
        } else if ip.is_loopback() && !own.ip().is_loopback() {
            Some(Self::Loopback)
        } else {
            None
        }
    }
}

/// Where the surveying node is: its own address, and the broadcast address
/// its transport sends local broadcasts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Here {
    /// This node's B/IP address.
    pub own: SocketAddrV4,
    /// Its transport's broadcast address.
    pub broadcast: Ipv4Addr,
}

/// A BDT entry a survey did not follow, and the first BBMD that listed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unfollowed {
    /// The address listed.
    pub address: SocketAddrV4,
    /// The BBMD whose table listed it first.
    pub listed_by: SocketAddrV4,
    /// Why it was not read.
    pub why: NotFollowed,
}

/// What a survey found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Survey<X> {
    /// The BBMDs read, at most [`MAX_SURVEY`], in breadth-first order.
    pub read: Vec<Surveyed<X>>,
    /// BBMDs the tables list past [`MAX_SURVEY`], in the order found, not
    /// read.
    pub unread: Vec<SocketAddrV4>,
    /// Entries not followed, each address once.
    pub not_followed: Vec<Unfollowed>,
}

/// Read the BDT of each seed, then of every BBMD a table read lists,
/// breadth first, up to [`MAX_SURVEY`] reads; the rest are listed unread.
/// `read` gives each BBMD's BDT and anything else the caller reads of it;
/// `here` is where this node is, for [`NotFollowed::check`], which every
/// listed entry passes before it is read (the seeds are the caller's).
///
/// Up to [`SURVEY_AT_ONCE`] reads run at once, so silent peers wait out
/// their timeouts together. The result is the one reading them one at a
/// time gives: BBMDs are numbered in the order they are found, a read
/// starts only for one of the first [`MAX_SURVEY`], and a table's entries
/// join the list only once every read before it has been taken in, so a
/// fast answer from a later BBMD can't jump the breadth-first order.
pub async fn survey<X, F, R>(seeds: &[SocketAddrV4], here: Here, read: F) -> Survey<X>
where
    F: Fn(SocketAddrV4) -> R,
    R: Future<Output = (Probe, X)>,
{
    let read = &read;
    let mut seen = HashSet::new();
    // Every BBMD found, in breadth-first order.
    let mut found: Vec<SocketAddrV4> = seeds.iter().copied().filter(|s| seen.insert(*s)).collect();
    let mut done: Vec<Surveyed<X>> = Vec::new();
    let mut not_followed = Vec::new();
    // Reads finished ahead of an earlier one, by their place in `found`.
    let mut early = BTreeMap::new();
    let mut started = 0;
    let mut reads = FuturesUnordered::new();
    loop {
        while reads.len() < SURVEY_AT_ONCE && started < found.len().min(MAX_SURVEY) {
            let (index, bbmd) = (started, found[started]);
            reads.push(async move { (index, read(bbmd).await) });
            started += 1;
        }
        let Some((index, outcome)) = reads.next().await else {
            break;
        };
        early.insert(index, outcome);
        while let Some((probe, extra)) = early.remove(&done.len()) {
            let bbmd = found[done.len()];
            if let Probe::Table(rows) = &probe {
                for row in rows {
                    if !seen.insert(row.address) {
                        continue;
                    }
                    match NotFollowed::check(row, &here) {
                        Some(why) => not_followed.push(Unfollowed {
                            address: row.address,
                            listed_by: bbmd,
                            why,
                        }),
                        None => found.push(row.address),
                    }
                }
            }
            done.push(Surveyed { bbmd, probe, extra });
        }
    }
    Survey {
        unread: found.split_off(done.len()),
        read: done,
        not_followed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(last: u8) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(10, 0, last, 1), 47808)
    }

    /// A node at none of the tests' addresses.
    fn nowhere() -> Here {
        Here {
            own: at(250),
            broadcast: Ipv4Addr::BROADCAST,
        }
    }

    fn row(address: &str, mask: [u8; 4]) -> BdtRow {
        BdtRow {
            address: address.parse().unwrap(),
            mask: Ipv4Addr::from(mask),
        }
    }

    #[test]
    fn a_mask_says_how_peers_forward() {
        let unicast = row("10.0.1.1:47808", [255; 4]);
        assert!(unicast.is_unicast());
        assert_eq!(unicast.forward_to(), Ipv4Addr::new(10, 0, 1, 1));
        assert!(unicast.explain().contains("two-hop"));
        let directed = row("10.0.2.1:47808", [255, 255, 255, 0]);
        assert!(!directed.is_unicast());
        assert_eq!(directed.forward_to(), Ipv4Addr::new(10, 0, 2, 255));
        assert!(directed.explain().contains("10.0.2.255"));
    }

    #[test]
    fn an_entry_under_thirty_seconds_is_expiring() {
        let entry = |remaining| FdtRow {
            address: at(9),
            ttl: 60,
            remaining,
        };
        assert!(entry(29).expiring());
        assert!(!entry(30).expiring());
    }

    #[test]
    fn a_nak_and_a_timeout_sort_apart() {
        use bacnet_transport::bvll::BvlcFunction;
        let code = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK;
        assert_eq!(probe_error(&BvlcExchangeError::Nak(code)), Probe::Nak(code));
        let silent = BvlcExchangeError::NoReply(Duration::from_secs(3));
        assert_eq!(probe_error(&silent), Probe::Unreachable("no answer".into()));
        // A success answers no read: not a refusal.
        let success = BvlcExchangeError::UnexpectedResponse {
            expected: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
            got: BvlcFunction::BVLC_RESULT,
        };
        assert!(matches!(probe_error(&success), Probe::Unreachable(_)));
        let local = BvlcExchangeError::Transport(bacnet_types::error::Error::Encoding(
            "B/IP management client is stopped".into(),
        ));
        assert_eq!(
            probe_error(&local),
            Probe::Unreachable("encoding error: B/IP management client is stopped".into())
        );
        let mac = mac("10.0.0.1:47809".parse().unwrap());
        assert_eq!(address_of(&mac), Some("10.0.0.1:47809".parse().unwrap()));
        assert_eq!(address_of(&mac[..5]), None);
    }

    /// Two seeds are read first, in order, then the BBMDs their tables
    /// list; a seed listed by the other is read once.
    #[tokio::test]
    async fn every_seed_is_read_then_what_the_tables_list() {
        let tables = |bbmd: SocketAddrV4| match bbmd.ip().octets()[2] {
            1 => vec![at(1), at(2), at(3)],
            2 => vec![at(2), at(1)],
            _ => vec![],
        };
        let surveyed = survey(&[at(1), at(2), at(1)], nowhere(), |bbmd| async move {
            let rows = tables(bbmd)
                .into_iter()
                .map(|address| BdtRow {
                    address,
                    mask: Ipv4Addr::BROADCAST,
                })
                .collect();
            (Probe::Table(rows), bbmd.port())
        })
        .await;
        let order: Vec<SocketAddrV4> = surveyed.read.iter().map(|s| s.bbmd).collect();
        assert_eq!(order, [at(1), at(2), at(3)]);
        assert!(surveyed.read.iter().all(|s| s.extra == 47808));
        assert!(surveyed.unread.is_empty() && surveyed.not_followed.is_empty());
    }

    /// Each class of address a BDT may list but no read may go to is
    /// reported, once, with the BBMD that listed it, and never read; a
    /// loopback peer is read only by a node on loopback.
    #[tokio::test]
    async fn addresses_no_read_may_go_to_are_not_followed() {
        let here = Here {
            own: at(1),
            broadcast: Ipv4Addr::new(10, 0, 9, 255),
        };
        const ONES: [u8; 4] = [255; 4];
        const SUBNET: [u8; 4] = [255, 255, 255, 0];
        let bad = [
            ("0.0.0.0:0", ONES, NotFollowed::Unresolved),
            ("0.0.0.0:47808", ONES, NotFollowed::Unspecified),
            ("10.0.5.1:0", ONES, NotFollowed::Unspecified),
            ("255.255.255.255:47808", ONES, NotFollowed::Broadcast),
            ("224.0.0.1:47808", ONES, NotFollowed::Multicast),
            ("239.255.255.250:1900", ONES, NotFollowed::Multicast),
            ("127.0.0.1:53", ONES, NotFollowed::Loopback),
            // The local broadcast address, whatever its mask.
            ("10.0.9.255:47808", ONES, NotFollowed::DirectedBroadcast),
            // Every host bit of its own mask set.
            ("10.0.7.255:47808", SUBNET, NotFollowed::DirectedBroadcast),
        ];
        let mut rows: Vec<BdtRow> = bad
            .iter()
            .map(|(address, mask, _)| BdtRow {
                address: address.parse().unwrap(),
                mask: Ipv4Addr::from(*mask),
            })
            .collect();
        // Our own address, and one bad entry listed twice.
        rows.push(BdtRow {
            address: here.own,
            mask: Ipv4Addr::BROADCAST,
        });
        rows.push(rows[0]);
        let reads = std::sync::Mutex::new(Vec::new());
        let surveyed = survey(&[at(2)], here, |bbmd| {
            reads.lock().unwrap().push(bbmd);
            let rows = rows.clone();
            async move { (Probe::Table(rows), ()) }
        })
        .await;
        assert_eq!(*reads.lock().unwrap(), [at(2)], "only the seed is read");
        let mut expected: Vec<Unfollowed> = bad
            .iter()
            .map(|(address, _, why)| Unfollowed {
                address: address.parse().unwrap(),
                listed_by: at(2),
                why: *why,
            })
            .collect();
        expected.push(Unfollowed {
            address: here.own,
            listed_by: at(2),
            why: NotFollowed::Own,
        });
        assert_eq!(surveyed.not_followed, expected);
        // A unicast entry ending in .255, and a subnet mask on a BBMD's own
        // address, are peers.
        let entry = |address: &str, mask: [u8; 4]| BdtRow {
            address: address.parse().unwrap(),
            mask: Ipv4Addr::from(mask),
        };
        assert_eq!(
            NotFollowed::check(&entry("10.0.7.255:47808", ONES), &here),
            None
        );
        assert_eq!(
            NotFollowed::check(&entry("10.0.7.1:47808", SUBNET), &here),
            None
        );
        // On loopback, a loopback peer is a peer.
        let local = Here {
            own: "127.0.0.1:47808".parse().unwrap(),
            broadcast: Ipv4Addr::new(127, 255, 255, 255),
        };
        let peer = entry("127.0.0.2:47809", ONES);
        assert_eq!(NotFollowed::check(&peer, &local), None);
        assert_eq!(
            NotFollowed::check_address(local.own, &local),
            Some(NotFollowed::Own)
        );
        assert_eq!(NotFollowed::check_address(at(3), &here), None);
    }

    /// A survey whose tables name far more BBMDs than it reads stops at
    /// [`MAX_SURVEY`] reads and lists the rest unread.
    #[tokio::test]
    async fn a_large_fan_out_reads_at_most_the_cap() {
        // Every table names 128 peers no other table names.
        let next = std::sync::atomic::AtomicU32::new(0);
        let peers = |_: SocketAddrV4| -> Vec<BdtRow> {
            (0..128)
                .map(|_| {
                    let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    BdtRow {
                        address: SocketAddrV4::new(Ipv4Addr::from(0x0B00_0000 + n), 47808),
                        mask: Ipv4Addr::BROADCAST,
                    }
                })
                .collect()
        };
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let surveyed = survey(&[at(1)], nowhere(), |bbmd| {
            reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let rows = peers(bbmd);
            async move { (Probe::Table(rows), ()) }
        })
        .await;
        assert_eq!(reads.into_inner(), MAX_SURVEY);
        assert_eq!(surveyed.read.len(), MAX_SURVEY);
        assert!(surveyed.unread.len() > 1_000, "{}", surveyed.unread.len());
        // The matrix over what was read stays bounded.
        let probes: Vec<(SocketAddrV4, Probe)> = surveyed
            .read
            .iter()
            .map(|s| (s.bbmd, s.probe.clone()))
            .collect();
        let matrix = Matrix::build(&probes);
        assert!(matrix.bbmds.len() <= matrix::GRID_CAP);
        assert!(matrix.omitted > 1_000, "{}", matrix.omitted);
    }

    /// No seed reads nothing.
    #[tokio::test]
    async fn no_seed_reads_nothing() {
        let surveyed = survey(&[], nowhere(), |_| async { (Probe::Skipped, ()) }).await;
        assert!(surveyed.read.is_empty() && surveyed.unread.is_empty());
    }
}

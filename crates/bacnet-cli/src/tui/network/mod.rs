//! The Network screen's messages and the B/IP facts it explains (#966):
//! what the UI asks the network worker to read from a BBMD, and what comes
//! back, as plain data with no transport type.
//!
//! - [`matrix`]: the BDT consistency matrix, a pure function of the tables
//!   a survey read.
//! - [`discovery`]: Who-Has and router discovery messages, for every
//!   transport (#960).
//! - [`routers`]: the Networks tab's router map, a pure function of the
//!   router announcements heard.

pub(crate) mod discovery;
pub(crate) mod matrix;
pub(crate) mod routers;

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_transport::bbmd::{BdtEntry, FdtEntryWire};
use bacnet_transport::bip::{BvlcExchangeError, ForeignRegistrationSnapshot};

use self::matrix::Probe;

/// UI to the network worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkCommand {
    /// Read this BBMD's BDT and FDT.
    Tables {
        /// The BBMD.
        bbmd: SocketAddrV4,
    },
    /// Read the BDT of `seed` and of every BBMD any BDT read lists, for the
    /// consistency matrix.
    Survey {
        /// Where the survey starts.
        seed: SocketAddrV4,
    },
}

/// The configured foreign-device registration, from the connection plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Foreign {
    /// The BBMD the client registers with.
    pub(crate) bbmd: SocketAddrV4,
    /// The time-to-live it asks for, in seconds.
    pub(crate) ttl: u16,
    /// How often it registers again.
    pub(crate) renewal: Duration,
}

/// One BDT entry: a BBMD and the mask its peers forward to it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BdtRow {
    /// The BBMD's B/IP address.
    pub(crate) address: SocketAddrV4,
    /// Its broadcast distribution mask.
    pub(crate) mask: Ipv4Addr,
}

impl BdtRow {
    /// True for an all-ones mask: peers send forwarded broadcasts to the
    /// BBMD itself, which broadcasts them on its own subnet (Annex J.4.3.1,
    /// the two-hop method).
    pub(crate) fn is_unicast(&self) -> bool {
        self.mask == Ipv4Addr::BROADCAST
    }

    /// Where a peer sends a forwarded broadcast for this entry: the address
    /// with every bit the mask clears set (Annex J.4.5). With an all-ones
    /// mask that is the BBMD itself.
    pub(crate) fn forward_to(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.address.ip().to_bits() | !self.mask.to_bits())
    }

    /// One line on what the mask means for forwarding.
    pub(crate) fn explain(&self) -> String {
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
pub(crate) struct FdtRow {
    /// The address the BBMD recorded for the foreign device, which is the
    /// source address its registration arrived from.
    pub(crate) address: SocketAddrV4,
    /// The time-to-live the device asked for, in seconds.
    pub(crate) ttl: u16,
    /// Seconds left before the BBMD drops the entry, as of the read. The
    /// BBMD adds a grace period to the time-to-live (Annex J.5.2.3).
    pub(crate) remaining: u16,
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

/// The FDT entry that is this client's own: the one recorded at our local
/// address. A client bound to `0.0.0.0` matches on the port alone. `None`
/// when no entry has our address, as when a NAT router rewrote the source
/// of our registration.
pub(crate) fn own_entry(entries: &[FdtRow], local: SocketAddrV4) -> Option<usize> {
    entries.iter().position(|entry| {
        entry.address.port() == local.port()
            && (local.ip().is_unspecified() || entry.address.ip() == local.ip())
    })
}

/// The 6-octet B/IP MAC of `address`.
pub(crate) fn mac(address: SocketAddrV4) -> [u8; 6] {
    let [a, b, c, d] = address.ip().octets();
    let [high, low] = address.port().to_be_bytes();
    [a, b, c, d, high, low]
}

/// The address in a 6-octet B/IP MAC.
pub(crate) fn address_of(mac: &[u8]) -> Option<SocketAddrV4> {
    let [a, b, c, d, high, low] = <[u8; 6]>::try_from(mac).ok()?;
    Some(SocketAddrV4::new(
        Ipv4Addr::new(a, b, c, d),
        u16::from_be_bytes([high, low]),
    ))
}

/// Why a BBMD gave no table, as text: no answer in time, or what failed.
fn unreachable_reason(error: &BvlcExchangeError) -> String {
    match error {
        BvlcExchangeError::NoReply(_) => "no answer".into(),
        other => other.to_string(),
    }
}

/// Sort a failed table read into a refusal or no answer (#1822): a NAK is a
/// refusal, while anything else (no answer, a reply the read does not take,
/// a send failure, a malformed reply) means the BBMD could not be read.
pub(crate) fn probe_error(error: &BvlcExchangeError) -> Probe {
    match error {
        BvlcExchangeError::Nak(code) => Probe::Nak(*code),
        other => Probe::Unreachable(unreachable_reason(other)),
    }
}

/// A table read's outcome as text for the UI.
pub(crate) fn read_error(error: &BvlcExchangeError) -> String {
    match error {
        BvlcExchangeError::Nak(_) => format!("refused: {error}"),
        BvlcExchangeError::UnexpectedResponse { .. } => format!("unexpected reply: {error}"),
        other => format!("unreachable: {}", unreachable_reason(other)),
    }
}

/// Network worker to UI.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NetworkEvent {
    /// How the foreign-device registration is going, polled once a second.
    Registration(ForeignRegistrationSnapshot),
    /// A BBMD's tables, from [`NetworkCommand::Tables`].
    Tables {
        /// The BBMD read.
        bbmd: SocketAddrV4,
        /// This client's own address, to find its FDT entry.
        local: SocketAddrV4,
        /// The BDT, or why it could not be read.
        bdt: Result<Vec<BdtRow>, String>,
        /// The FDT, or why it could not be read.
        fdt: Result<Vec<FdtRow>, String>,
    },
    /// Every BDT a survey read, in the order it found the BBMDs.
    Survey {
        /// Where the survey started.
        seed: SocketAddrV4,
        /// What it read.
        probes: Vec<(SocketAddrV4, Probe)>,
    },
    /// The command was not carried out.
    Refused {
        /// The command refused.
        command: NetworkCommand,
        /// Why.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn our_entry_is_the_one_at_our_address() {
        let entry = |address: &str| FdtRow {
            address: address.parse().unwrap(),
            ttl: 60,
            remaining: 90,
        };
        let fdt = [entry("10.0.0.9:47808"), entry("10.0.0.5:50000")];
        assert_eq!(own_entry(&fdt, "10.0.0.5:50000".parse().unwrap()), Some(1));
        assert_eq!(own_entry(&fdt, "0.0.0.0:50000".parse().unwrap()), Some(1));
        // Behind NAT the BBMD recorded another address: nothing matches.
        assert_eq!(own_entry(&fdt, "192.168.1.5:50000".parse().unwrap()), None);
    }

    #[test]
    fn a_nak_and_a_timeout_sort_apart() {
        use bacnet_transport::bvll::{BvlcFunction, BvlcResultCode};
        let code = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK;
        let nak = BvlcExchangeError::Nak(code);
        assert_eq!(probe_error(&nak), Probe::Nak(code));
        assert_eq!(
            read_error(&nak),
            "refused: BVLC-Result NAK READ_BROADCAST_DISTRIBUTION_TABLE_NAK (0x0020)"
        );
        let silent = BvlcExchangeError::NoReply(Duration::from_secs(3));
        assert_eq!(probe_error(&silent), Probe::Unreachable("no answer".into()));
        assert_eq!(read_error(&silent), "unreachable: no answer");
        // A success answers no read: not a refusal.
        let success = BvlcExchangeError::UnexpectedResponse {
            expected: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
            got: BvlcFunction::BVLC_RESULT,
        };
        assert!(matches!(probe_error(&success), Probe::Unreachable(_)));
        assert!(read_error(&success).starts_with("unexpected reply: "));
        let local = BvlcExchangeError::Transport(bacnet_types::error::Error::Encoding(
            "B/IP management client is stopped".into(),
        ));
        assert_eq!(
            probe_error(&local),
            Probe::Unreachable("encoding error: B/IP management client is stopped".into())
        );
        assert!(read_error(&local).starts_with("unreachable: "));
        let mac = mac("10.0.0.1:47809".parse().unwrap());
        assert_eq!(address_of(&mac), Some("10.0.0.1:47809".parse().unwrap()));
        assert_eq!(address_of(&mac[..5]), None);
    }
}

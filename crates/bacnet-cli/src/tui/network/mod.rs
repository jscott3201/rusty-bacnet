//! The Network screen's messages and the B/IP facts it explains (#966):
//! what the UI asks the network worker to read from a BBMD, and what comes
//! back, as plain data with no transport type.
//!
//! - [`discovery`]: Who-Has and router discovery messages, for every
//!   transport (#960).
//! - [`routers`]: the Networks tab's router map, a pure function of the
//!   router announcements heard.
//!
//! The table rows, the survey and the BDT consistency matrix are
//! [`bacnet_ops::bbmd`]'s, shared with the MCP server's network map.

pub(crate) mod discovery;
pub(crate) mod routers;

use std::net::SocketAddrV4;
use std::time::Duration;

pub(crate) use bacnet_ops::bbmd::{BdtRow, FdtRow, address_of, mac, probe_error};
use bacnet_ops::bbmd::{Probe, Unfollowed, unreachable_reason};
use bacnet_transport::bip::{BvlcExchangeError, ForeignRegistrationSnapshot};

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
        /// The BDT entries it did not follow, and why (#2130).
        not_followed: Vec<Unfollowed>,
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

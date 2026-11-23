//! [`NetworkReject`]: a router's refusal that ended a confirmed request
//! (#2724).

use core::fmt;

use bacnet_npdu::{Mac, RejectMessageReason};

use crate::data_link::DataLink;
use crate::enums::ErrorCode;

/// A Reject-Message-To-Network that ended a confirmed request: the router
/// the request went through would not pass it on toward the station's
/// network (Clause 6.4.4). No device answered, so whether the device would
/// have carried out the request is not known; it never saw it.
///
/// The client takes a reject as a request's own only from the router the
/// request went to, for the network it named, and only once the request
/// was on its way; [`Error::NetworkRejected`](super::Error::NetworkRejected)
/// says which reasons end a request at once.
///
/// It displays the router in its link's own address form (`10.0.0.1:47808`
/// on BACnet/IP, `mstp:5`, `bip6:0x0004D2`, colon-separated hex on
/// BACnet/SC), the network, and the reason by name in lower case with
/// hyphens beside its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NetworkReject {
    /// The router's MAC on the client's link: the link peer the reject came
    /// from, which is the router the request was sent through.
    pub router: Mac,
    /// The client's data link, which says how to write [`router`](Self::router);
    /// `None` when the transport doesn't say.
    pub link: Option<DataLink>,
    /// The network the reject names, the request's DNET.
    pub network: u16,
    /// Why the router refused it. The set is open: an octet the standard
    /// doesn't name is kept as received.
    pub reason: RejectMessageReason,
}

impl NetworkReject {
    /// The reject the router at `router` on `link` sent for `network`.
    pub const fn new(
        router: Mac,
        link: Option<DataLink>,
        network: u16,
        reason: RejectMessageReason,
    ) -> Self {
        Self {
            router,
            link,
            network,
            reason,
        }
    }

    /// The router in its link's own address form, as [`NetworkReject`]
    /// displays it and the tools' address arguments read it back.
    pub fn router(&self) -> impl fmt::Display + '_ {
        RouterText {
            mac: self.router.as_slice(),
            link: self.link,
        }
    }

    /// The COMMUNICATION error code Clause 18 gives a network request that
    /// failed this way: NOT_ROUTER_TO_DNET for reason 1, ROUTER_BUSY for 2,
    /// UNKNOWN_NETWORK_MESSAGE for 3, MESSAGE_TOO_LONG for 4,
    /// SECURITY_ERROR for 5 (a reason earlier revisions defined) and
    /// ADDRESSING_ERROR for 6; OTHER for any other reason. An audit record
    /// of the request carries it as its Result.
    pub const fn error_code(&self) -> ErrorCode {
        match self.reason.to_raw() {
            1 => ErrorCode::NOT_ROUTER_TO_DNET,
            2 => ErrorCode::ROUTER_BUSY,
            3 => ErrorCode::UNKNOWN_NETWORK_MESSAGE,
            4 => ErrorCode::MESSAGE_TOO_LONG,
            5 => ErrorCode::SECURITY_ERROR,
            6 => ErrorCode::ADDRESSING_ERROR,
            _ => ErrorCode::OTHER,
        }
    }
}

impl fmt::Display for NetworkReject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let network = self.network;
        write!(
            f,
            "Reject-Message-To-Network from router {}: ",
            self.router()
        )?;
        match self.reason {
            RejectMessageReason::NOT_DIRECTLY_CONNECTED => {
                write!(f, "no route to network {network}")
            }
            RejectMessageReason::ROUTER_BUSY => write!(f, "busy for network {network}"),
            RejectMessageReason::UNKNOWN_MESSAGE_TYPE => {
                write!(
                    f,
                    "a network message type it does not know (network {network})"
                )
            }
            RejectMessageReason::MESSAGE_TOO_LONG => {
                write!(f, "the message is too long for network {network}")
            }
            RejectMessageReason::ADDRESSING_ERROR => {
                write!(
                    f,
                    "an address length it found invalid, for network {network}"
                )
            }
            reason if reason.to_raw() == 5 => {
                write!(f, "a security refusal for network {network}")
            }
            _ => write!(f, "the message for network {network} was refused"),
        }?;
        f.write_str(" (")?;
        if let Some(name) = self.reason.name() {
            super::remote::write_name(f, Some(name), "reason", 0)?;
            f.write_str(", ")?;
        }
        write!(f, "reason {})", self.reason.to_raw())
    }
}

/// A router MAC in the form the tools write an address on its link: as
/// `bacnet_ops`' address style for the link has it, `ip:port` for six
/// octets unless the link is BACnet/SC or B/IPv6 or MS/TP, `bip6:0x...` for
/// a B/IPv6 VMAC, `mstp:N` for an MS/TP station, and colon-separated hex
/// otherwise.
struct RouterText<'a> {
    mac: &'a [u8],
    link: Option<DataLink>,
}

impl fmt::Display for RouterText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.link, self.mac) {
            (Some(DataLink::Mstp), [station]) => write!(f, "mstp:{station}"),
            (Some(DataLink::Bip6), [high, middle, low]) => {
                write!(
                    f,
                    "bip6:{}",
                    crate::bip6_vmac::VmacHex([*high, *middle, *low])
                )
            }
            (Some(DataLink::Mstp | DataLink::Bip6 | DataLink::Sc), _) => hex(f, self.mac),
            (_, [a, b, c, d, high, low]) => {
                write!(f, "{a}.{b}.{c}.{d}:{}", u16::from_be_bytes([*high, *low]))
            }
            _ => hex(f, self.mac),
        }
    }
}

fn hex(f: &mut fmt::Formatter<'_>, mac: &[u8]) -> fmt::Result {
    for (i, octet) in mac.iter().enumerate() {
        if i > 0 {
            f.write_str(":")?;
        }
        write!(f, "{octet:02x}")?;
    }
    Ok(())
}

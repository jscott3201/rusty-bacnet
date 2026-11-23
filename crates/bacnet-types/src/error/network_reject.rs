//! [`NetworkReject`]: a router's refusal of a confirmed request (#2724).

use core::fmt;

use bacnet_npdu::{Mac, RejectMessageReason};

use crate::data_link::DataLink;
use crate::enums::ErrorCode;

/// A router's reject (Reject-Message-To-Network) of a confirmed request: a
/// router on the way would not pass it on toward the station's network
/// (Clause 6.4.4).
///
/// The client takes a reject as a request's own only when the router the
/// request went to delivered it to this station alone, naming the request's
/// network, after the request was on its way;
/// [`Error::NetworkRejected`](super::Error::NetworkRejected) says which
/// reasons end a request at once, and
/// [`Error::RouterBusyTimeout`](super::Error::RouterBusyTimeout) carries a
/// busy reject (reason 2) the request then timed out after.
///
/// It displays the router in its link's own address form (`10.0.0.1:47808`
/// on BACnet/IP, `mstp:5`, `bip6:0x0004D2`, colon-separated hex on
/// BACnet/SC), the router farther along that sent it when it came through
/// this one, the network, the reason by name in lower case with hyphens
/// beside its number, and whether the device can have received the request.
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
    /// The router that sent the reject, when it came through
    /// [`router`](Self::router) from farther along: the network and MAC its
    /// NPDU named as its source. `None` when `router` sent it itself.
    pub refused_by: Option<(u16, Mac)>,
    /// Whether the request had sent one frame when the reject came, so the
    /// reject was for that frame and the device never received the
    /// request. `false` after a retry or a segment went before it: an
    /// earlier attempt may have reached the device, and a write may have
    /// taken effect.
    pub single_frame: bool,
}

impl NetworkReject {
    /// The reject the router at `router` on `link` sent for `network`,
    /// from that router itself, after more than one frame: the device may
    /// have received the request ([`Self::after_one_frame`] says otherwise).
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
            refused_by: None,
            single_frame: false,
        }
    }

    /// This reject, sent by the router at `mac` on `network` and relayed
    /// through [`router`](Self::router).
    #[must_use]
    pub const fn relayed_from(mut self, network: u16, mac: Mac) -> Self {
        self.refused_by = Some((network, mac));
        self
    }

    /// This reject, for the one frame the request sent.
    #[must_use]
    pub const fn after_one_frame(mut self) -> Self {
        self.single_frame = true;
        self
    }

    /// The router in its link's own address form, as [`NetworkReject`]
    /// displays it and the tools' address arguments read it back.
    pub fn router(&self) -> impl fmt::Display + '_ {
        RouterText {
            mac: self.router.as_slice(),
            link: self.link,
        }
    }

    /// The router farther along that sent the reject, as `5:0x0A` (its
    /// network and MAC), when it came through [`router`](Self::router).
    pub fn refused_by(&self) -> Option<impl fmt::Display + '_> {
        self.refused_by.as_ref().map(|(network, mac)| Station {
            network: *network,
            mac: mac.as_slice(),
        })
    }

    /// What the reject says, without whether the device can have received
    /// the request: the routers, the network and the reason.
    pub fn what(&self) -> impl fmt::Display + '_ {
        What(self)
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
        write!(f, "{}; ", self.what())?;
        f.write_str(if self.single_frame {
            "the request never reached the device"
        } else {
            "an earlier attempt may have reached the device"
        })
    }
}

/// [`NetworkReject::what`].
struct What<'a>(&'a NetworkReject);

impl fmt::Display for What<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reject = self.0;
        let network = reject.network;
        match reject.refused_by() {
            Some(by) => write!(
                f,
                "Reject-Message-To-Network through router {}, from {by}: ",
                reject.router()
            )?,
            None => write!(
                f,
                "Reject-Message-To-Network from router {}: ",
                reject.router()
            )?,
        }
        match reject.reason {
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
        if let Some(name) = reject.reason.name() {
            super::remote::write_name(f, Some(name), "reason", 0)?;
            f.write_str(", ")?;
        }
        write!(f, "reason {})", reject.reason.to_raw())
    }
}

/// A station on a network, as the target grammar writes one: `5:0x0A`.
struct Station<'a> {
    network: u16,
    mac: &'a [u8],
}

impl fmt::Display for Station<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:0x", self.network)?;
        self.mac
            .iter()
            .try_for_each(|octet| write!(f, "{octet:02X}"))
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

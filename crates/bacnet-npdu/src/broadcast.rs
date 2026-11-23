//! The DADR forms that name every station of a remote network, or a group
//! of them, rather than one (#2709): what a node sending a confirmed request
//! to a station on another network refuses, since only an unconfirmed
//! request may go to a broadcast or multicast (Clause 6.3).

use core::fmt;

/// A MAC that names a broadcast or a multicast group on a remote network
/// rather than one station, read from its length and octets alone, since
/// a node can't see a remote network's data link (#2709). A router puts the
/// DADR on its network as the link's destination address (Clause 6.5.4),
/// so a DADR equal to that link's broadcast reaches every station there.
///
/// Each length maps to the links that use it (Table 6-2):
///
/// - One octet: MS/TP, whose broadcast is `0xFF`, and ARCNET, whose
///   broadcast is `0x00` and where `0xFF` is a station. Only a caller that
///   knows the network is ARCNET can say which ([`RemoteBroadcast::of`]'s
///   `arcnet`).
/// - Two octets: LonTalk alone. A first octet of `0x00` is a broadcast and
///   one of `0xFF` a group (Figure 6-4); a unicast's first octet, its
///   subnet, runs from `0x01` to `0xFE`.
/// - Three octets: a VMAC (B/IPv6, ZigBee; Clause H.7.2), whose top bit is
///   never set, so three octets of `0xFF` are no device's.
/// - Six octets: B/IP, Ethernet and BACnet/SC. One whose first four octets
///   are `0xFF` is the limited broadcast at some port on B/IP, a group MAC
///   on Ethernet (the low bit of its first octet is set), and on BACnet/SC
///   either its broadcast VMAC or no device's (Clause H.7.3).
///
/// Seven octets are a LonTalk subnet and Neuron ID, one device, and other
/// lengths are taken as one station too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RemoteBroadcast {
    /// The one octet `0xFF`: the broadcast on an MS/TP network (Clause 9),
    /// whose stations are 0 to 254; on an ARCNET network, a station.
    MstpBroadcast,
    /// The one octet `0x00` on a network the caller knows is ARCNET: its
    /// broadcast (Clause 8).
    ArcnetBroadcast,
    /// Two octets, the first `0x00`: a LonTalk broadcast, the second octet
    /// naming the subnet.
    LonTalkBroadcast {
        /// The second octet.
        subnet: u8,
    },
    /// Two octets, the first `0xFF`: a LonTalk group.
    LonTalkGroup {
        /// The second octet.
        group: u8,
    },
    /// Three octets of `0xFF`: no device's VMAC.
    NoVmac,
    /// Six octets whose first four are `0xFF`.
    SixOctetBroadcast {
        /// The last two octets: on B/IP, the UDP port.
        port: u16,
    },
}

impl RemoteBroadcast {
    /// The form `mac`, a station's MAC on a remote network, takes, if it is
    /// one; `arcnet` when the caller knows the network is ARCNET. A few
    /// comparisons on the slice, nothing more.
    #[must_use]
    pub fn of(mac: &[u8], arcnet: bool) -> Option<Self> {
        match *mac {
            [0xFF] if !arcnet => Some(Self::MstpBroadcast),
            [0x00] if arcnet => Some(Self::ArcnetBroadcast),
            [0x00, subnet] => Some(Self::LonTalkBroadcast { subnet }),
            [0xFF, group] => Some(Self::LonTalkGroup { group }),
            [0xFF, 0xFF, 0xFF] => Some(Self::NoVmac),
            [0xFF, 0xFF, 0xFF, 0xFF, hi, lo] => Some(Self::SixOctetBroadcast {
                port: u16::from_be_bytes([hi, lo]),
            }),
            _ => None,
        }
    }
}

impl fmt::Display for RemoteBroadcast {
    /// What the form is, decoded for each link that uses its length.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MstpBroadcast => f.write_str(
                "station 255, the broadcast of an MS/TP network (on an ARCNET network, where \
                 255 is a station, mark the station ARCNET)",
            ),
            Self::ArcnetBroadcast => f.write_str("station 0, the broadcast of an ARCNET network"),
            Self::LonTalkBroadcast { subnet } => {
                write!(f, "a LonTalk broadcast (subnet {subnet})")
            }
            Self::LonTalkGroup { group } => write!(f, "LonTalk group {group}, a multicast"),
            Self::NoVmac => f.write_str(
                "VMAC 0xFFFFFF, which no device has (a device's VMAC never sets its top bit)",
            ),
            Self::SixOctetBroadcast { port } => write!(
                f,
                "a broadcast on every six-octet link: 255.255.255.255:{port}, the limited \
                 broadcast, on BACnet/IP, a group MAC on Ethernet, and on BACnet/SC no \
                 device's VMAC"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteBroadcast::{self, *};

    #[test]
    fn each_length_names_its_broadcast_forms_and_nothing_else() {
        let cases: [(&[u8], bool, Option<RemoteBroadcast>); 22] = [
            (&[0xFF], false, Some(MstpBroadcast)),
            // Marked ARCNET: 255 is a station and 0 the broadcast.
            (&[0xFF], true, None),
            (&[0x00], true, Some(ArcnetBroadcast)),
            (&[0x00], false, None),
            (&[0xFE], false, None),
            // Two octets are LonTalk: 0x00 and 0xFF first are no unicast.
            (&[0x00, 0x05], false, Some(LonTalkBroadcast { subnet: 5 })),
            (&[0x00, 0x00], false, Some(LonTalkBroadcast { subnet: 0 })),
            (&[0xFF, 0x07], false, Some(LonTalkGroup { group: 7 })),
            (&[0xFF, 0xFF], true, Some(LonTalkGroup { group: 0xFF })),
            (&[0x01, 0x00], false, None),
            (&[0xFE, 0x7F], false, None),
            (&[0xFF, 0xFF, 0xFF], false, Some(NoVmac)),
            (&[0x7F, 0xFF, 0xFF], false, None),
            (&[0xFF; 6], false, Some(SixOctetBroadcast { port: 0xFFFF })),
            (
                &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0],
                false,
                Some(SixOctetBroadcast { port: 47808 }),
            ),
            (&[0xFF, 0xFF, 0xFF, 0xFE, 0xBA, 0xC0], false, None),
            (&[0xC0, 0xA8, 0x01, 0xFF, 0xBA, 0xC0], false, None),
            (&[0xE0, 0x00, 0x00, 0x01, 0xBA, 0xC0], false, None),
            // Seven octets are a LonTalk Neuron ID: one device.
            (&[0xFF; 7], false, None),
            (&[0xFF; 18], false, None),
            (&[0x00; 4], false, None),
            (&[], false, None),
        ];
        for (mac, arcnet, form) in cases {
            assert_eq!(
                RemoteBroadcast::of(mac, arcnet),
                form,
                "{mac:02X?} arcnet {arcnet}"
            );
        }
    }
}

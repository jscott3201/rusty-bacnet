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
///   broadcast is `0x00` and where `0xFF` is a station. `0x00` is MS/TP's
///   station 0, so it is never caught: an ARCNET network's broadcast goes
///   unrefused, with the one-station mark or without.
/// - Two octets: LonTalk, where a first octet of `0x00` is a broadcast (the
///   second octet naming the subnet, or `0x00` the whole domain) and one of
///   `0xFF` a group (Figure 6-4); a unicast's first octet, its subnet, runs
///   from `0x01` to `0xFE`. A gateway's virtual network may number its
///   stations in two octets too, `0x0001` and up.
/// - Three octets: a VMAC (B/IPv6, ZigBee; Clause H.7.2), whose top bit is
///   never set, so three octets of `0xFF` are no device's.
/// - Six octets: B/IP, Ethernet and BACnet/SC. One whose first four octets
///   are `0xFF` is the limited broadcast at some port on B/IP, a group MAC
///   on Ethernet (the low bit of its first octet is set), and on BACnet/SC
///   either its broadcast VMAC or no device's (Clause H.7.3).
///
/// Seven octets are a LonTalk subnet and Neuron ID, one device, and other
/// lengths are taken as one station too.
///
/// # The one-station mark
///
/// Two of the forms are also an ordinary station on some network: the one
/// octet `0xFF` is station 255 on ARCNET, and a gateway that numbers its
/// stations in two octets has stations starting `0x00` or `0xFF`. Only the
/// caller can know, so it says so with a mark on the address meaning "this
/// address is one station" ([`RemoteBroadcast::of`]'s `one_station`, #2738).
/// Unmarked, an address is checked as above, the safe default. The mark
/// only lifts forms; it never refuses an address the unmarked check takes.
/// What each length refuses:
///
/// | MAC | unmarked | marked one station |
/// |-----|----------|--------------------|
/// | one octet `0xFF` | [`MstpBroadcast`](Self::MstpBroadcast) | a station (ARCNET 255) |
/// | one octet `0x00` | a station | a station |
/// | two octets `0x0001` to `0x00FF` | [`LonTalkBroadcast`](Self::LonTalkBroadcast) | a station |
/// | two octets `0xFF00` to `0xFFFE` | [`LonTalkGroup`](Self::LonTalkGroup) | a station |
/// | two octets `0x0000` | [`LonTalkBroadcast`](Self::LonTalkBroadcast) | the same |
/// | two octets `0xFFFF` | [`LonTalkGroup`](Self::LonTalkGroup) | the same |
/// | three octets `0xFFFFFF` | [`NoVmac`](Self::NoVmac) | the same |
/// | six octets `FF:FF:FF:FF:..` | [`SixOctetBroadcast`](Self::SixOctetBroadcast) | the same |
///
/// A two-octet address keeps two values out of the mark's reach: `0x0000`,
/// LonTalk's broadcast to the whole domain, and `0xFFFF`, all ones, the
/// broadcast or no device's address on every link with a fixed MAC length
/// but ARCNET; a numbering of stations in two octets has no need of either.
/// Three and six octets are a VMAC or a six-octet link's MAC, whose
/// broadcast forms are no station on any network, so the mark changes
/// nothing for them. [`lifted_by_one_station`](Self::lifted_by_one_station)
/// says which forms the mark lifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RemoteBroadcast {
    /// The one octet `0xFF`, unmarked: the broadcast on an MS/TP network
    /// (Clause 9), whose stations are 0 to 254; on an ARCNET network, a
    /// station.
    MstpBroadcast,
    /// Two octets, the first `0x00`: a LonTalk broadcast, the second octet
    /// naming the subnet, or `0x00` the whole domain.
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
    /// one; `one_station` when the caller has marked the address one station
    /// (the table in the type's docs). A few comparisons on the slice,
    /// nothing more.
    #[must_use]
    pub fn of(mac: &[u8], one_station: bool) -> Option<Self> {
        match *mac {
            [0xFF] if !one_station => Some(Self::MstpBroadcast),
            [0x00, subnet] if !one_station || subnet == 0x00 => {
                Some(Self::LonTalkBroadcast { subnet })
            }
            [0xFF, group] if !one_station || group == 0xFF => Some(Self::LonTalkGroup { group }),
            [0xFF, 0xFF, 0xFF] => Some(Self::NoVmac),
            [0xFF, 0xFF, 0xFF, 0xFF, hi, lo] => Some(Self::SixOctetBroadcast {
                port: u16::from_be_bytes([hi, lo]),
            }),
            _ => None,
        }
    }

    /// Whether marking the address one station takes it out of this form
    /// ([`RemoteBroadcast::of`]): the one octet `0xFF`, and two octets
    /// starting `0x00` or `0xFF` other than `0x0000` and `0xFFFF`. A caller
    /// that refuses an unmarked address in such a form can name the mark
    /// as the way to reach a station there.
    #[must_use]
    pub const fn lifted_by_one_station(self) -> bool {
        match self {
            Self::MstpBroadcast => true,
            Self::LonTalkBroadcast { subnet } => subnet != 0x00,
            Self::LonTalkGroup { group } => group != 0xFF,
            Self::NoVmac | Self::SixOctetBroadcast { .. } => false,
        }
    }
}

impl fmt::Display for RemoteBroadcast {
    /// What the form is, decoded for each link that uses its length.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MstpBroadcast => {
                f.write_str("station 255, the broadcast of an MS/TP network (a station on ARCNET)")
            }
            Self::LonTalkBroadcast { subnet: 0x00 } => {
                f.write_str("a LonTalk broadcast to the whole domain")
            }
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
        let cases: [(&[u8], bool, Option<RemoteBroadcast>); 36] = [
            (&[0xFF], false, Some(MstpBroadcast)),
            // Marked one station: 255 is a station (ARCNET's). 0 is one
            // either way: ARCNET's broadcast there is not caught.
            (&[0xFF], true, None),
            (&[0x00], true, None),
            (&[0x00], false, None),
            (&[0xFE], false, None),
            (&[0xFE], true, None),
            // Two octets are LonTalk: 0x00 and 0xFF first are no unicast,
            // unless marked one station, but for 0x0000 and 0xFFFF.
            (&[0x00, 0x05], false, Some(LonTalkBroadcast { subnet: 5 })),
            (&[0x00, 0x05], true, None),
            (&[0x00, 0x01], true, None),
            (
                &[0x00, 0xFF],
                false,
                Some(LonTalkBroadcast { subnet: 0xFF }),
            ),
            (&[0x00, 0xFF], true, None),
            (&[0x00, 0x00], false, Some(LonTalkBroadcast { subnet: 0 })),
            (&[0x00, 0x00], true, Some(LonTalkBroadcast { subnet: 0 })),
            (&[0xFF, 0x07], false, Some(LonTalkGroup { group: 7 })),
            (&[0xFF, 0x07], true, None),
            (&[0xFF, 0x00], true, None),
            (&[0xFF, 0xFE], true, None),
            (&[0xFF, 0xFF], false, Some(LonTalkGroup { group: 0xFF })),
            (&[0xFF, 0xFF], true, Some(LonTalkGroup { group: 0xFF })),
            (&[0x01, 0x00], false, None),
            (&[0xFE, 0x7F], false, None),
            (&[0xFE, 0x7F], true, None),
            // Three and six octets: the mark changes nothing.
            (&[0xFF, 0xFF, 0xFF], false, Some(NoVmac)),
            (&[0xFF, 0xFF, 0xFF], true, Some(NoVmac)),
            (&[0x7F, 0xFF, 0xFF], false, None),
            (&[0xFF; 6], false, Some(SixOctetBroadcast { port: 0xFFFF })),
            (&[0xFF; 6], true, Some(SixOctetBroadcast { port: 0xFFFF })),
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
            (&[0xFF; 7], true, None),
            (&[0xFF; 18], false, None),
            (&[0x00; 4], false, None),
            (&[], false, None),
        ];
        for (mac, one_station, form) in cases {
            assert_eq!(
                RemoteBroadcast::of(mac, one_station),
                form,
                "{mac:02X?} one_station {one_station}"
            );
        }
    }

    /// Over every one- and two-octet MAC, the mark refuses exactly the
    /// unmarked forms it doesn't lift: it only lifts, never refuses.
    #[test]
    fn the_mark_refuses_the_unmarked_forms_it_does_not_lift() {
        let check = |mac: &[u8]| {
            let expected =
                RemoteBroadcast::of(mac, false).filter(|form| !form.lifted_by_one_station());
            assert_eq!(RemoteBroadcast::of(mac, true), expected, "{mac:02X?}");
        };
        for octet in 0..=u8::MAX {
            check(&[octet]);
        }
        for value in 0..=u16::MAX {
            check(&value.to_be_bytes());
        }
        for mac in [
            &[0xFF, 0xFF, 0xFF][..],
            &[0xFF; 6],
            &[0xFF, 0xFF, 0xFF, 0xFF, 0, 1],
        ] {
            let form = RemoteBroadcast::of(mac, false).unwrap();
            assert!(!form.lifted_by_one_station(), "{mac:02X?}");
            assert_eq!(RemoteBroadcast::of(mac, true), Some(form));
        }
    }
}

//! The B/IPv6 BVLC function and result codes, and the payload layout each
//! function has.

use super::{BIP6_ADDRESS_LENGTH, VMAC_LENGTH};
use crate::function::named_codes;
use crate::PayloadShape;

named_codes! {
    /// A B/IPv6 BVLC function code: the second octet of every B/IPv6 BVLL
    /// message, which says what the message is (Annex U.2, Table U-1).
    ///
    /// The set is open, so a decoder can report a code it doesn't know. The
    /// constants are the twelve functions of Annex U.2. Code 0x0B is not
    /// among them: the clause that defined it (U.2.12) was deleted, so this
    /// edition treats it like any other unknown code.
    pub struct Bvlc6Function(u8);

    /// Acknowledges a request that needs an answer (U.2.1).
    const BVLC_RESULT = 0x00;
    /// A directed NPDU, naming its recipient's VMAC (U.2.2).
    const ORIGINAL_UNICAST_NPDU = 0x01;
    /// A broadcast NPDU from a node that isn't a foreign device (U.2.3).
    const ORIGINAL_BROADCAST_NPDU = 0x02;
    /// Asks, by multicast, which B/IPv6 address a VMAC is at (U.2.4).
    const ADDRESS_RESOLUTION = 0x03;
    /// An Address-Resolution a BBMD sends on into other multicast domains
    /// (U.2.5).
    const FORWARDED_ADDRESS_RESOLUTION = 0x04;
    /// Answers either kind of Address-Resolution (U.2.6).
    const ADDRESS_RESOLUTION_ACK = 0x05;
    /// Asks a node at a known B/IPv6 address for its VMAC (U.2.7).
    const VIRTUAL_ADDRESS_RESOLUTION = 0x06;
    /// Answers a Virtual-Address-Resolution (U.2.8).
    const VIRTUAL_ADDRESS_RESOLUTION_ACK = 0x07;
    /// An NPDU a BBMD passes on, with its originator's VMAC and address
    /// (U.2.9).
    const FORWARDED_NPDU = 0x08;
    /// Asks a BBMD to take the sender as a foreign device (U.2.10).
    const REGISTER_FOREIGN_DEVICE = 0x09;
    /// Removes one entry from a BBMD's foreign device table (U.2.11).
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY = 0x0A;
    /// A foreign device's broadcast, for its BBMD to distribute (U.2.13).
    const DISTRIBUTE_BROADCAST_TO_NETWORK = 0x0C;
}

named_codes! {
    /// The result code a B/IPv6 BVLC-Result carries (Annex U.2.1.1):
    /// success, or a NAK naming the request that failed.
    pub struct Bvlc6ResultCode(u16);

    /// The request succeeded.
    const SUCCESSFUL_COMPLETION = 0x0000;
    /// A BBMD refused an Address-Resolution sent to it: the sender is not
    /// one of its registered foreign devices, or the BBMD could not pass the
    /// request on (U.4.4).
    const ADDRESS_RESOLUTION_NAK = 0x0030;
    /// A Virtual-Address-Resolution failed.
    const VIRTUAL_ADDRESS_RESOLUTION_NAK = 0x0060;
    /// Register-Foreign-Device failed or is not supported.
    const REGISTER_FOREIGN_DEVICE_NAK = 0x0090;
    /// Delete-Foreign-Device-Table-Entry failed or is not supported.
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK = 0x00A0;
    /// Distribute-Broadcast-To-Network failed or is not supported.
    const DISTRIBUTE_BROADCAST_TO_NETWORK_NAK = 0x00C0;
}

impl Bvlc6Function {
    /// The payload layout of this function, from its format clause in
    /// Annex U.2, or `None` for a code Annex U doesn't define. The payload
    /// is everything after the 4-octet header, so it starts with a VMAC in
    /// every message (Table U-1 gives the whole lengths).
    ///
    /// - Virtual-Address-Resolution: the source VMAC, 3 octets (U.2.7.1).
    /// - BVLC-Result and Register-Foreign-Device: the source VMAC and a
    ///   result code or time-to-live, 5 octets (U.2.1.1, U.2.10.1).
    /// - Address-Resolution and the two ACKs: two VMACs, 6 octets (U.2.4.1,
    ///   U.2.6.1, U.2.8.1).
    /// - Delete-Foreign-Device-Table-Entry: the source VMAC and the entry's
    ///   B/IPv6 address, 21 octets (U.2.11.1).
    /// - Forwarded-Address-Resolution: two VMACs and the originator's
    ///   address, 24 octets (U.2.5.1).
    /// - Original-Broadcast-NPDU and Distribute-Broadcast-To-Network: a VMAC,
    ///   then the NPDU (U.2.3.1, U.2.13.1).
    /// - Original-Unicast-NPDU: two VMACs, then the NPDU (U.2.2.1).
    /// - Forwarded-NPDU: the originator's VMAC and address, then the NPDU
    ///   (U.2.9.1).
    ///
    /// ```
    /// use bacnet_bip_codec::ipv6::Bvlc6Function;
    /// use bacnet_bip_codec::PayloadShape;
    ///
    /// assert_eq!(
    ///     Bvlc6Function::FORWARDED_NPDU.payload_shape(),
    ///     Some(PayloadShape::Npdu { prefix: 21 }),
    /// );
    /// assert_eq!(Bvlc6Function::from_raw(0x0B).payload_shape(), None);
    /// ```
    #[must_use]
    pub const fn payload_shape(self) -> Option<PayloadShape> {
        Some(match self {
            Self::VIRTUAL_ADDRESS_RESOLUTION => PayloadShape::Fixed(VMAC_LENGTH),
            Self::BVLC_RESULT | Self::REGISTER_FOREIGN_DEVICE => {
                PayloadShape::Fixed(VMAC_LENGTH + 2)
            }
            Self::ADDRESS_RESOLUTION
            | Self::ADDRESS_RESOLUTION_ACK
            | Self::VIRTUAL_ADDRESS_RESOLUTION_ACK => PayloadShape::Fixed(2 * VMAC_LENGTH),
            Self::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => {
                PayloadShape::Fixed(VMAC_LENGTH + BIP6_ADDRESS_LENGTH)
            }
            Self::FORWARDED_ADDRESS_RESOLUTION => {
                PayloadShape::Fixed(2 * VMAC_LENGTH + BIP6_ADDRESS_LENGTH)
            }
            Self::ORIGINAL_BROADCAST_NPDU | Self::DISTRIBUTE_BROADCAST_TO_NETWORK => {
                PayloadShape::Npdu {
                    prefix: VMAC_LENGTH,
                }
            }
            Self::ORIGINAL_UNICAST_NPDU => PayloadShape::Npdu {
                prefix: 2 * VMAC_LENGTH,
            },
            Self::FORWARDED_NPDU => PayloadShape::Npdu {
                prefix: VMAC_LENGTH + BIP6_ADDRESS_LENGTH,
            },
            _ => return None,
        })
    }

    /// The NAK named after this function in Annex U.2.1.1, for the five
    /// requests that have one: both address resolutions, foreign-device
    /// registration, FDT entry deletion and broadcast distribution. `None`
    /// for every other code.
    ///
    /// Annex U says when to send four of them. A device that is not a BBMD
    /// answers the last three with their NAKs (U.4.4, U.4.5.3, U.4.5.4), and
    /// so does a BBMD that can't carry them out. A BBMD answers an
    /// Address-Resolution sent to it with its NAK when the sender is not a
    /// registered foreign device, or when the BBMD can't pass the request on
    /// (U.4.4); a broadcast distribution from a node that isn't a registered
    /// foreign device gets its NAK the same way. A
    /// Forwarded-Address-Resolution is never answered with a BVLC-Result.
    #[must_use]
    pub const fn nak(self) -> Option<Bvlc6ResultCode> {
        Some(match self {
            Self::ADDRESS_RESOLUTION => Bvlc6ResultCode::ADDRESS_RESOLUTION_NAK,
            Self::VIRTUAL_ADDRESS_RESOLUTION => Bvlc6ResultCode::VIRTUAL_ADDRESS_RESOLUTION_NAK,
            Self::REGISTER_FOREIGN_DEVICE => Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK,
            Self::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => {
                Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
            }
            Self::DISTRIBUTE_BROADCAST_TO_NETWORK => {
                Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
            }
            _ => return None,
        })
    }
}

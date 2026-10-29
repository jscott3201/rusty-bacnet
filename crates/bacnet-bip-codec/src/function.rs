//! The BVLC function and result codes, and the payload layout each function
//! has.

use crate::{BDT_ENTRY_LENGTH, BIP_ADDRESS_LENGTH, FDT_ENTRY_LENGTH};

/// An open set of wire codes with named constants, the shape `bacnet-types`
/// gives its enumerations: any raw value converts, and the names cover the
/// values the standard defines.
macro_rules! named_codes {
    (
        $(#[$meta:meta])*
        pub struct $Name:ident($inner:ty);
        $(
            $(#[$vmeta:meta])*
            const $VARIANT:ident = $val:expr_2021;
        )*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $Name($inner);

        impl $Name {
            $(
                $(#[$vmeta])*
                #[doc = ""]
                #[doc = concat!("Wire value `", stringify!($val), "`.")]
                pub const $VARIANT: Self = Self($val);
            )*

            /// Every named constant, with its name, in the order declared.
            pub const ALL_NAMED: &[(&str, Self)] = &[
                $( (stringify!($VARIANT), Self($val)), )*
            ];

            /// Wrap a raw wire value, named or not.
            #[inline]
            #[must_use]
            pub const fn from_raw(value: $inner) -> Self {
                Self(value)
            }

            /// The raw wire value.
            #[inline]
            #[must_use]
            pub const fn to_raw(self) -> $inner {
                self.0
            }

            /// The constant's name, or `None` for a value the standard
            /// doesn't define.
            #[must_use]
            pub const fn name(self) -> Option<&'static str> {
                match self.0 {
                    $( $val => Some(stringify!($VARIANT)), )*
                    _ => None,
                }
            }

            /// Whether the standard defines this value.
            #[must_use]
            pub const fn is_defined(self) -> bool {
                self.name().is_some()
            }
        }

        /// `Name::CONSTANT` for a named value, `Name(raw)` otherwise.
        impl core::fmt::Debug for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => write!(f, "{}::{}", stringify!($Name), name),
                    None => write!(f, "{}({})", stringify!($Name), self.0),
                }
            }
        }

        /// The constant's name, or the raw value in decimal.
        impl core::fmt::Display for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => f.write_str(name),
                    None => write!(f, "{}", self.0),
                }
            }
        }
    };
}

pub(crate) use named_codes;

named_codes! {
    /// A BVLC function code: the second octet of every B/IP BVLL message,
    /// which says what the message is (Annex J.2).
    ///
    /// The set is open, so a decoder can report a code it doesn't know. The
    /// constants are the twelve functions Annex J.2.1-J.2.12 define. Code
    /// 0x0C is not among them: the clause that defined it (J.2.13) was
    /// deleted, so this edition treats it like any other unknown code.
    pub struct BvlcFunction(u8);

    /// Acknowledges a BBMD management request (J.2.1).
    const BVLC_RESULT = 0x00;
    /// Writes a BBMD's broadcast distribution table (J.2.2).
    const WRITE_BROADCAST_DISTRIBUTION_TABLE = 0x01;
    /// Asks a BBMD for its broadcast distribution table (J.2.3).
    const READ_BROADCAST_DISTRIBUTION_TABLE = 0x02;
    /// Returns a broadcast distribution table (J.2.4).
    const READ_BROADCAST_DISTRIBUTION_TABLE_ACK = 0x03;
    /// An NPDU a BBMD passes on, with its originator's address (J.2.5).
    const FORWARDED_NPDU = 0x04;
    /// Asks a BBMD to take the sender as a foreign device (J.2.6).
    const REGISTER_FOREIGN_DEVICE = 0x05;
    /// Asks a BBMD for its foreign device table (J.2.7).
    const READ_FOREIGN_DEVICE_TABLE = 0x06;
    /// Returns a foreign device table (J.2.8).
    const READ_FOREIGN_DEVICE_TABLE_ACK = 0x07;
    /// Removes one entry from a BBMD's foreign device table (J.2.9).
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY = 0x08;
    /// A foreign device's broadcast, for its BBMD to distribute (J.2.10).
    const DISTRIBUTE_BROADCAST_TO_NETWORK = 0x09;
    /// A directed NPDU (J.2.11).
    const ORIGINAL_UNICAST_NPDU = 0x0A;
    /// A local broadcast NPDU (J.2.12).
    const ORIGINAL_BROADCAST_NPDU = 0x0B;
}

named_codes! {
    /// The result code a BVLC-Result carries (Annex J.2.1.1): success, or a
    /// NAK naming the request that failed.
    pub struct BvlcResultCode(u16);

    /// The request succeeded.
    const SUCCESSFUL_COMPLETION = 0x0000;
    /// Write-Broadcast-Distribution-Table failed or is not supported.
    const WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK = 0x0010;
    /// Read-Broadcast-Distribution-Table failed or is not supported.
    const READ_BROADCAST_DISTRIBUTION_TABLE_NAK = 0x0020;
    /// Register-Foreign-Device failed or is not supported.
    const REGISTER_FOREIGN_DEVICE_NAK = 0x0030;
    /// Read-Foreign-Device-Table failed or is not supported.
    const READ_FOREIGN_DEVICE_TABLE_NAK = 0x0040;
    /// Delete-Foreign-Device-Table-Entry failed or is not supported.
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK = 0x0050;
    /// Distribute-Broadcast-To-Network failed or is not supported.
    const DISTRIBUTE_BROADCAST_TO_NETWORK_NAK = 0x0060;
}

/// How long the payload of a BVLL message may be: the octets after the
/// 4-octet header, which depend on the function alone (Annex J.2, U.2).
///
/// [`BvlcFunction::payload_shape`] gives it for B/IP and
/// [`Bvlc6Function::payload_shape`](crate::ipv6::Bvlc6Function::payload_shape)
/// for B/IPv6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadShape {
    /// Exactly this many octets.
    Fixed(usize),
    /// A table of whole entries of this many octets each. An empty table is
    /// allowed. Only B/IP messages carry tables.
    Table {
        /// Octets per entry.
        entry_length: usize,
    },
    /// This many octets of fixed fields, then an NPDU of any length. The
    /// codec does not look inside the NPDU; the network layer does.
    Npdu {
        /// Octets before the NPDU.
        prefix: usize,
    },
}

impl PayloadShape {
    /// Whether a payload of `length` octets fits this shape.
    #[must_use]
    pub const fn admits(self, length: usize) -> bool {
        match self {
            Self::Fixed(fixed) => length == fixed,
            Self::Table { entry_length } => length.is_multiple_of(entry_length),
            Self::Npdu { prefix } => length >= prefix,
        }
    }
}

impl BvlcFunction {
    /// The payload layout of this function, from its format clause in
    /// Annex J.2, or `None` for a code Annex J doesn't define.
    ///
    /// - BVLC-Result and Register-Foreign-Device: two octets, a result code or
    ///   a time-to-live (J.2.1.1, J.2.6.1).
    /// - The two table reads: nothing (J.2.3.1, J.2.7.1).
    /// - Delete-Foreign-Device-Table-Entry: the entry's address, six octets
    ///   (J.2.9.1).
    /// - Write-Broadcast-Distribution-Table and both table ACKs: whole
    ///   10-octet entries (J.2.2.1, J.2.4.1, J.2.8.1).
    /// - Forwarded-NPDU: the originator's 6-octet address, then the NPDU
    ///   (J.2.5.1).
    /// - Distribute-Broadcast-To-Network and both Original messages: just the
    ///   NPDU (J.2.10.1-J.2.12.1).
    ///
    /// ```
    /// use bacnet_bip_codec::{BvlcFunction, PayloadShape};
    ///
    /// assert_eq!(
    ///     BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY.payload_shape(),
    ///     Some(PayloadShape::Fixed(6)),
    /// );
    /// assert_eq!(BvlcFunction::from_raw(0x0C).payload_shape(), None);
    /// ```
    #[must_use]
    pub const fn payload_shape(self) -> Option<PayloadShape> {
        Some(match self {
            Self::BVLC_RESULT | Self::REGISTER_FOREIGN_DEVICE => PayloadShape::Fixed(2),
            Self::READ_BROADCAST_DISTRIBUTION_TABLE | Self::READ_FOREIGN_DEVICE_TABLE => {
                PayloadShape::Fixed(0)
            }
            Self::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => PayloadShape::Fixed(BIP_ADDRESS_LENGTH),
            Self::WRITE_BROADCAST_DISTRIBUTION_TABLE
            | Self::READ_BROADCAST_DISTRIBUTION_TABLE_ACK => PayloadShape::Table {
                entry_length: BDT_ENTRY_LENGTH,
            },
            Self::READ_FOREIGN_DEVICE_TABLE_ACK => PayloadShape::Table {
                entry_length: FDT_ENTRY_LENGTH,
            },
            Self::FORWARDED_NPDU => PayloadShape::Npdu {
                prefix: BIP_ADDRESS_LENGTH,
            },
            Self::DISTRIBUTE_BROADCAST_TO_NETWORK
            | Self::ORIGINAL_UNICAST_NPDU
            | Self::ORIGINAL_BROADCAST_NPDU => PayloadShape::Npdu { prefix: 0 },
            _ => return None,
        })
    }

    /// The NAK that answers this function when it fails or isn't supported,
    /// for the six requests J.2.1 says only a BBMD carries out (the BDT
    /// write and read, the FDT read, registration, entry deletion and
    /// broadcast distribution). `None` for every other code.
    ///
    /// A device that is not a BBMD answers a unicast request of these kinds
    /// with this code (J.2.1), and every device answers
    /// Write-Broadcast-Distribution-Table with it (J.4.4.2).
    #[must_use]
    pub const fn nak(self) -> Option<BvlcResultCode> {
        Some(match self {
            Self::WRITE_BROADCAST_DISTRIBUTION_TABLE => {
                BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
            }
            Self::READ_BROADCAST_DISTRIBUTION_TABLE => {
                BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK
            }
            Self::REGISTER_FOREIGN_DEVICE => BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
            Self::READ_FOREIGN_DEVICE_TABLE => BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
            Self::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => {
                BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
            }
            Self::DISTRIBUTE_BROADCAST_TO_NETWORK => {
                BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
            }
            _ => return None,
        })
    }
}

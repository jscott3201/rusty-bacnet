//! B/IPv6 BVLL frames and messages: one-shot decode, and checked encode.

use super::{BVLC_TYPE_BIP6, Bip6Address, Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6Error};
use crate::{BvllSink, HEADER_LENGTH, MAX_BVLL_LENGTH, PayloadShape};

/// A B/IPv6 BVLL message whose header checks out, before its payload is
/// read.
///
/// [`Bvll6Frame::decode`] checks the BVLC type and the Length field and
/// accepts any function code, so a receiver can see a function it doesn't
/// know before [`Bvll6Frame::message`] reads the payload by the function's
/// layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bvll6Frame<'a> {
    /// The function octet.
    pub function: Bvlc6Function,
    /// The octets after the 4-octet header, from the first VMAC on.
    pub payload: &'a [u8],
}

impl<'a> Bvll6Frame<'a> {
    /// Check a datagram's BVLL header and split off its payload.
    ///
    /// The datagram must be one whole message: at least the 4-octet header,
    /// BVLC type 0x82, and a Length field equal to the datagram's length,
    /// since it counts the whole message (Annex U.2). Octets after the
    /// declared length are refused rather than ignored, so a datagram can't
    /// pass for a shorter message.
    ///
    /// ```
    /// use bacnet_bip_codec::ipv6::{Bvlc6Function, Bvll6Error, Bvll6Frame};
    ///
    /// // A Virtual-Address-Resolution from VMAC 00 04 D2.
    /// let frame = Bvll6Frame::decode(&[0x82, 0x06, 0x00, 0x07, 0x00, 0x04, 0xD2])?;
    /// assert_eq!(frame.function, Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION);
    /// assert_eq!(frame.payload, [0x00, 0x04, 0xD2]);
    ///
    /// // Function 0x0B still has a frame, but no message.
    /// let frame = Bvll6Frame::decode(&[0x82, 0x0B, 0x00, 0x07, 0x00, 0x04, 0xD2])?;
    /// assert_eq!(frame.message(), Err(Bvll6Error::UnknownFunction { function: 0x0B }));
    ///
    /// // A trailing octet the Length field doesn't count.
    /// assert_eq!(
    ///     Bvll6Frame::decode(&[0x82, 0x06, 0x00, 0x07, 0x00, 0x04, 0xD2, 0x00]),
    ///     Err(Bvll6Error::LengthMismatch { declared: 7, received: 8 }),
    /// );
    /// # Ok::<(), Bvll6Error>(())
    /// ```
    pub fn decode(datagram: &'a [u8]) -> Result<Self, Bvll6Error> {
        let [bvlc_type, function, high, low, payload @ ..] = datagram else {
            return Err(Bvll6Error::TooShort {
                length: datagram.len(),
            });
        };
        if *bvlc_type != BVLC_TYPE_BIP6 {
            return Err(Bvll6Error::NotBip6 {
                bvlc_type: *bvlc_type,
            });
        }
        let declared = usize::from(u16::from_be_bytes([*high, *low]));
        if declared != datagram.len() {
            return Err(Bvll6Error::LengthMismatch {
                declared,
                received: datagram.len(),
            });
        }
        Ok(Self {
            function: Bvlc6Function::from_raw(*function),
            payload,
        })
    }

    /// Read the payload by the function's layout (see
    /// [`Bvlc6Function::payload_shape`]).
    ///
    /// Fails with [`Bvll6Error::UnknownFunction`] for a function Annex U
    /// doesn't define, and with [`Bvll6Error::PayloadLength`] when the
    /// payload is the wrong length for its function. Field values are not
    /// judged: any VMAC or address, an undefined result code, a zero
    /// time-to-live and an empty NPDU all decode.
    pub fn message(self) -> Result<Bvll6<'a>, Bvll6Error> {
        let function = self.function;
        match read(function, self.payload) {
            Some(message) => Ok(message),
            None if function.is_defined() => Err(Bvll6Error::PayloadLength {
                function,
                length: self.payload.len(),
            }),
            None => Err(Bvll6Error::UnknownFunction {
                function: function.to_raw(),
            }),
        }
    }
}

/// Reads fields off the front of a payload, in wire order.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (field, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*field)
    }

    fn vmac(&mut self) -> Option<Bip6Vmac> {
        self.take().map(Bip6Vmac::new)
    }

    fn address(&mut self) -> Option<Bip6Address> {
        self.take().map(Bip6Address::from_octets)
    }

    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_be_bytes)
    }

    /// The rest of the payload: the NPDU.
    fn npdu(self) -> &'a [u8] {
        self.0
    }

    /// `message`, when no octets are left over.
    fn end<T>(self, message: T) -> Option<T> {
        self.0.is_empty().then_some(message)
    }
}

/// The message `payload` holds for `function`, or `None` when it is the
/// wrong length or the function is unknown.
fn read(function: Bvlc6Function, payload: &[u8]) -> Option<Bvll6<'_>> {
    let mut fields = Fields(payload);
    let source = fields.vmac()?;
    let message = match function {
        Bvlc6Function::BVLC_RESULT => Bvll6::BvlcResult {
            source,
            code: Bvlc6ResultCode::from_raw(fields.u16()?),
        },
        Bvlc6Function::ORIGINAL_UNICAST_NPDU => {
            let destination = fields.vmac()?;
            return Some(Bvll6::OriginalUnicastNpdu {
                source,
                destination,
                npdu: fields.npdu(),
            });
        }
        Bvlc6Function::ORIGINAL_BROADCAST_NPDU => {
            return Some(Bvll6::OriginalBroadcastNpdu {
                source,
                npdu: fields.npdu(),
            });
        }
        Bvlc6Function::ADDRESS_RESOLUTION => Bvll6::AddressResolution {
            source,
            target: fields.vmac()?,
        },
        Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION => {
            let target = fields.vmac()?;
            Bvll6::ForwardedAddressResolution {
                original_source: source,
                target,
                original_address: fields.address()?,
            }
        }
        Bvlc6Function::ADDRESS_RESOLUTION_ACK => Bvll6::AddressResolutionAck {
            source,
            destination: fields.vmac()?,
        },
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION => Bvll6::VirtualAddressResolution { source },
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK => Bvll6::VirtualAddressResolutionAck {
            source,
            destination: fields.vmac()?,
        },
        Bvlc6Function::FORWARDED_NPDU => {
            let original_address = fields.address()?;
            return Some(Bvll6::ForwardedNpdu {
                original_source: source,
                original_address,
                npdu: fields.npdu(),
            });
        }
        Bvlc6Function::REGISTER_FOREIGN_DEVICE => Bvll6::RegisterForeignDevice {
            source,
            ttl: fields.u16()?,
        },
        Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => Bvll6::DeleteForeignDeviceTableEntry {
            source,
            entry: fields.address()?,
        },
        Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK => {
            return Some(Bvll6::DistributeBroadcastToNetwork {
                original_source: source,
                npdu: fields.npdu(),
            });
        }
        _ => return None,
    };
    fields.end(message)
}

/// Decode one datagram as a B/IPv6 BVLL message: [`Bvll6Frame::decode`],
/// then [`Bvll6Frame::message`].
///
/// ```
/// use core::net::Ipv6Addr;
/// use bacnet_bip_codec::ipv6::{decode, Bip6Address, Bip6Vmac, Bvll6, Bvll6Error};
///
/// // A Forwarded-NPDU from VMAC 00 04 D2 at [2001:db8::1]:47808, carrying
/// // the NPDU 01 00.
/// let mut wire = vec![0x82, 0x08, 0x00, 0x1B, 0x00, 0x04, 0xD2];
/// wire.extend_from_slice(&[0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
/// wire.extend_from_slice(&[0xBA, 0xC0, 0x01, 0x00]);
/// assert_eq!(
///     decode(&wire)?,
///     Bvll6::ForwardedNpdu {
///         original_source: Bip6Vmac::new([0x00, 0x04, 0xD2]),
///         original_address: Bip6Address::new(
///             Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 1),
///             47808,
///         ),
///         npdu: &[0x01, 0x00],
///     },
/// );
///
/// // A Register-Foreign-Device needs exactly a VMAC and a 2-octet
/// // time-to-live.
/// assert!(matches!(
///     decode(&[0x82, 0x09, 0x00, 0x08, 0x00, 0x04, 0xD2, 0x3C]),
///     Err(Bvll6Error::PayloadLength { length: 4, .. }),
/// ));
/// # Ok::<(), Bvll6Error>(())
/// ```
pub fn decode(datagram: &[u8]) -> Result<Bvll6<'_>, Bvll6Error> {
    Bvll6Frame::decode(datagram)?.message()
}

/// One B/IPv6 BVLL message (Annex U.2), borrowing its NPDU.
///
/// [`decode`] produces one from a datagram, and
/// [`encode_into`](Bvll6::encode_into) or
/// [`encode_to_sink`](Bvll6::encode_to_sink) writes one. Each variant holds
/// what its function's format clause lays out after the header, in wire
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bvll6<'a> {
    /// BVLC-Result (U.2.1).
    BvlcResult {
        /// The sender's VMAC.
        source: Bip6Vmac,
        /// Success, or the NAK of the request that failed.
        code: Bvlc6ResultCode,
    },
    /// Original-Unicast-NPDU (U.2.2).
    OriginalUnicastNpdu {
        /// The sender's VMAC.
        source: Bip6Vmac,
        /// The recipient's VMAC. A node discards a message whose destination
        /// VMAC isn't its own (U.3).
        destination: Bip6Vmac,
        /// The NPDU.
        npdu: &'a [u8],
    },
    /// Original-Broadcast-NPDU (U.2.3).
    OriginalBroadcastNpdu {
        /// The sender's VMAC.
        source: Bip6Vmac,
        /// The NPDU.
        npdu: &'a [u8],
    },
    /// Address-Resolution (U.2.4).
    AddressResolution {
        /// The sender's VMAC.
        source: Bip6Vmac,
        /// The VMAC whose B/IPv6 address the sender wants.
        target: Bip6Vmac,
    },
    /// Forwarded-Address-Resolution (U.2.5).
    ForwardedAddressResolution {
        /// The asker's VMAC.
        original_source: Bip6Vmac,
        /// The VMAC whose B/IPv6 address it wants.
        target: Bip6Vmac,
        /// Where the answer goes: the asker's B/IPv6 address.
        original_address: Bip6Address,
    },
    /// Address-Resolution-ACK (U.2.6).
    AddressResolutionAck {
        /// The sender's VMAC: the one that was resolved.
        source: Bip6Vmac,
        /// The asker's VMAC.
        destination: Bip6Vmac,
    },
    /// Virtual-Address-Resolution (U.2.7).
    VirtualAddressResolution {
        /// The sender's VMAC.
        source: Bip6Vmac,
    },
    /// Virtual-Address-Resolution-ACK (U.2.8).
    VirtualAddressResolutionAck {
        /// The sender's VMAC: the answer.
        source: Bip6Vmac,
        /// The asker's VMAC.
        destination: Bip6Vmac,
    },
    /// Forwarded-NPDU (U.2.9).
    ForwardedNpdu {
        /// The originating node's VMAC, not the forwarding BBMD's.
        original_source: Bip6Vmac,
        /// Where the NPDU started: the originating node's B/IPv6 address.
        original_address: Bip6Address,
        /// The NPDU.
        npdu: &'a [u8],
    },
    /// Register-Foreign-Device (U.2.10).
    RegisterForeignDevice {
        /// The sender's VMAC.
        source: Bip6Vmac,
        /// Seconds within which the device will register again.
        ttl: u16,
    },
    /// Delete-Foreign-Device-Table-Entry (U.2.11).
    DeleteForeignDeviceTableEntry {
        /// The sender's VMAC.
        source: Bip6Vmac,
        /// Which entry to remove: its foreign device's B/IPv6 address.
        entry: Bip6Address,
    },
    /// Distribute-Broadcast-To-Network (U.2.13).
    DistributeBroadcastToNetwork {
        /// The sending foreign device's VMAC.
        original_source: Bip6Vmac,
        /// The NPDU.
        npdu: &'a [u8],
    },
}

impl<'a> Bvll6<'a> {
    /// The message's function code.
    #[must_use]
    pub const fn function(&self) -> Bvlc6Function {
        match self {
            Self::BvlcResult { .. } => Bvlc6Function::BVLC_RESULT,
            Self::OriginalUnicastNpdu { .. } => Bvlc6Function::ORIGINAL_UNICAST_NPDU,
            Self::OriginalBroadcastNpdu { .. } => Bvlc6Function::ORIGINAL_BROADCAST_NPDU,
            Self::AddressResolution { .. } => Bvlc6Function::ADDRESS_RESOLUTION,
            Self::ForwardedAddressResolution { .. } => Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION,
            Self::AddressResolutionAck { .. } => Bvlc6Function::ADDRESS_RESOLUTION_ACK,
            Self::VirtualAddressResolution { .. } => Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION,
            Self::VirtualAddressResolutionAck { .. } => {
                Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK
            }
            Self::ForwardedNpdu { .. } => Bvlc6Function::FORWARDED_NPDU,
            Self::RegisterForeignDevice { .. } => Bvlc6Function::REGISTER_FOREIGN_DEVICE,
            Self::DeleteForeignDeviceTableEntry { .. } => {
                Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
            }
            Self::DistributeBroadcastToNetwork { .. } => {
                Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK
            }
        }
    }

    /// The VMAC every message starts with: the sender's, except in the two
    /// messages a BBMD relays (Forwarded-NPDU and
    /// Forwarded-Address-Resolution), where it is the originating node's.
    /// Distribute-Broadcast-To-Network calls its VMAC the original source,
    /// but the foreign device that sends one is that source.
    #[must_use]
    pub const fn source_vmac(&self) -> Bip6Vmac {
        match *self {
            Self::BvlcResult { source, .. }
            | Self::OriginalUnicastNpdu { source, .. }
            | Self::OriginalBroadcastNpdu { source, .. }
            | Self::AddressResolution { source, .. }
            | Self::AddressResolutionAck { source, .. }
            | Self::VirtualAddressResolution { source }
            | Self::VirtualAddressResolutionAck { source, .. }
            | Self::RegisterForeignDevice { source, .. }
            | Self::DeleteForeignDeviceTableEntry { source, .. }
            | Self::ForwardedAddressResolution {
                original_source: source,
                ..
            }
            | Self::ForwardedNpdu {
                original_source: source,
                ..
            }
            | Self::DistributeBroadcastToNetwork {
                original_source: source,
                ..
            } => source,
        }
    }

    /// The recipient's VMAC, for the three messages that name one:
    /// Original-Unicast-NPDU and the two ACKs. A node discards those when it
    /// isn't theirs (U.3).
    #[must_use]
    pub const fn destination_vmac(&self) -> Option<Bip6Vmac> {
        match *self {
            Self::OriginalUnicastNpdu { destination, .. }
            | Self::AddressResolutionAck { destination, .. }
            | Self::VirtualAddressResolutionAck { destination, .. } => Some(destination),
            _ => None,
        }
    }

    /// The NPDU, for the four functions that carry one.
    #[must_use]
    pub const fn npdu(&self) -> Option<&'a [u8]> {
        match *self {
            Self::OriginalUnicastNpdu { npdu, .. }
            | Self::OriginalBroadcastNpdu { npdu, .. }
            | Self::ForwardedNpdu { npdu, .. }
            | Self::DistributeBroadcastToNetwork { npdu, .. } => Some(npdu),
            _ => None,
        }
    }

    /// The originating node's B/IPv6 address, for the two messages a BBMD
    /// relays with one: Forwarded-NPDU and Forwarded-Address-Resolution.
    #[must_use]
    pub const fn origin(&self) -> Option<Bip6Address> {
        match *self {
            Self::ForwardedNpdu {
                original_address, ..
            }
            | Self::ForwardedAddressResolution {
                original_address, ..
            } => Some(original_address),
            _ => None,
        }
    }

    /// Octets after the header: the fixed fields
    /// [`Bvlc6Function::payload_shape`] gives the function, then the NPDU of
    /// the four functions that carry one.
    const fn payload_length(&self) -> usize {
        let fixed = match self.function().payload_shape() {
            Some(PayloadShape::Fixed(length) | PayloadShape::Npdu { prefix: length }) => length,
            // Unreachable: every variant's function is one Annex U defines,
            // and no B/IPv6 message carries a table. The golden vectors
            // check every variant's encoded length.
            Some(PayloadShape::Table { .. }) | None => 0,
        };
        match self.npdu() {
            Some(npdu) => fixed + npdu.len(),
            None => fixed,
        }
    }

    /// Check the message and return its wire length, header included.
    ///
    /// Encoding refuses two things: a message too long for the 16-bit Length
    /// field ([`Bvll6Error::TooLong`]), and a BVLC-Result whose code Annex
    /// U.2.1.1 doesn't define ([`Bvll6Error::UndefinedResultCode`]). Every
    /// other shape the standard forbids can't be built: each variant holds
    /// exactly its function's fields.
    pub fn encoded_len(&self) -> Result<usize, Bvll6Error> {
        if let Self::BvlcResult { code, .. } = self
            && !code.is_defined()
        {
            return Err(Bvll6Error::UndefinedResultCode {
                code: code.to_raw(),
            });
        }
        let length = HEADER_LENGTH + self.payload_length();
        if length > MAX_BVLL_LENGTH {
            return Err(Bvll6Error::TooLong { length });
        }
        Ok(length)
    }

    /// Encode the message at the start of `output` and return its wire
    /// length. Nothing is written unless it passes the checks of
    /// [`Bvll6::encoded_len`] and fits.
    ///
    /// ```
    /// use bacnet_bip_codec::ipv6::{Bip6Vmac, Bvlc6ResultCode, Bvll6, Bvll6Error};
    ///
    /// let source = Bip6Vmac::new([0x00, 0x04, 0xD2]);
    /// let mut wire = [0u8; 9];
    /// let nak = Bvll6::BvlcResult {
    ///     source,
    ///     code: Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK,
    /// };
    /// assert_eq!(nak.encode_into(&mut wire)?, 9);
    /// assert_eq!(wire, [0x82, 0x00, 0x00, 0x09, 0x00, 0x04, 0xD2, 0x00, 0x90]);
    ///
    /// // 0x0010 is a B/IP result code, not one Annex U defines.
    /// let undefined = Bvll6::BvlcResult { source, code: Bvlc6ResultCode::from_raw(0x0010) };
    /// assert_eq!(
    ///     undefined.encode_into(&mut wire),
    ///     Err(Bvll6Error::UndefinedResultCode { code: 0x0010 }),
    /// );
    /// # Ok::<(), Bvll6Error>(())
    /// ```
    pub fn encode_into(&self, output: &mut [u8]) -> Result<usize, Bvll6Error> {
        let mut sink = output;
        self.encode_to_sink(&mut sink)
    }

    /// Encode the message into `sink` and return its wire length.
    ///
    /// The checks are those of [`Bvll6::encoded_len`]. When the sink reports
    /// a fixed capacity, a message that does not fit fails before anything
    /// is written. A growable sink is written directly, so if one of its
    /// writes fails, the octets written before it stay in the sink; see
    /// [`BvllSink`].
    pub fn encode_to_sink<S: BvllSink + ?Sized>(&self, sink: &mut S) -> Result<usize, Bvll6Error> {
        let length = self.encoded_len()?;
        if let Some(available) = sink.remaining_capacity()
            && available < length
        {
            return Err(Bvll6Error::BufferExhausted {
                needed: length,
                available,
            });
        }
        // `encoded_len` capped the length at 0xFFFF.
        let [high, low] = (length as u16).to_be_bytes();
        let [v0, v1, v2] = self.source_vmac().octets();
        sink.write(&[
            BVLC_TYPE_BIP6,
            self.function().to_raw(),
            high,
            low,
            v0,
            v1,
            v2,
        ])?;
        match *self {
            Self::VirtualAddressResolution { .. } => {}
            Self::BvlcResult { code, .. } => sink.write(&code.to_raw().to_be_bytes())?,
            Self::RegisterForeignDevice { ttl, .. } => sink.write(&ttl.to_be_bytes())?,
            Self::AddressResolution { target: second, .. }
            | Self::AddressResolutionAck {
                destination: second,
                ..
            }
            | Self::VirtualAddressResolutionAck {
                destination: second,
                ..
            } => sink.write(&second.octets())?,
            Self::ForwardedAddressResolution {
                target,
                original_address,
                ..
            } => {
                sink.write(&target.octets())?;
                sink.write(&original_address.to_octets())?;
            }
            Self::DeleteForeignDeviceTableEntry { entry, .. } => sink.write(&entry.to_octets())?,
            Self::OriginalUnicastNpdu {
                destination, npdu, ..
            } => {
                sink.write(&destination.octets())?;
                sink.write(npdu)?;
            }
            Self::ForwardedNpdu {
                original_address,
                npdu,
                ..
            } => {
                sink.write(&original_address.to_octets())?;
                sink.write(npdu)?;
            }
            Self::OriginalBroadcastNpdu { npdu, .. }
            | Self::DistributeBroadcastToNetwork { npdu, .. } => sink.write(npdu)?,
        }
        Ok(length)
    }
}

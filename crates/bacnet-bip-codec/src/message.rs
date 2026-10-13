//! BVLL frames and messages: one-shot decode, and checked encode.

use core::net::SocketAddrV4;

use crate::{
    address_from_octets, address_to_octets, BdtEntries, BvlcFunction, BvlcResultCode, BvllError,
    BvllSink, FdtEntries, BVLC_TYPE_BIP, HEADER_LENGTH, MAX_BVLL_LENGTH,
};

/// A BVLL message whose header checks out, before its payload is read.
///
/// [`BvllFrame::decode`] checks the BVLC type and the Length field and
/// accepts any function code, so a receiver can see a function it doesn't
/// know, or decide how to answer a request whose payload is malformed,
/// before [`BvllFrame::message`] reads the payload by the function's
/// layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BvllFrame<'a> {
    /// The function octet.
    pub function: BvlcFunction,
    /// The octets after the 4-octet header.
    pub payload: &'a [u8],
}

impl<'a> BvllFrame<'a> {
    /// Check a datagram's BVLL header and split off its payload.
    ///
    /// The datagram must be one whole message: at least the 4-octet header,
    /// BVLC type 0x81, and a Length field equal to the datagram's length,
    /// since it counts the whole message (Annex J.2). Octets after the
    /// declared length are refused rather than ignored, so a datagram can't
    /// pass for a shorter message.
    ///
    /// ```
    /// use bacnet_bip_codec::{BvlcFunction, BvllError, BvllFrame};
    ///
    /// // A Read-Foreign-Device-Table: header only.
    /// let frame = BvllFrame::decode(&[0x81, 0x06, 0x00, 0x04])?;
    /// assert_eq!(frame.function, BvlcFunction::READ_FOREIGN_DEVICE_TABLE);
    /// assert!(frame.payload.is_empty());
    ///
    /// // An unknown function still has a frame, but no message.
    /// let frame = BvllFrame::decode(&[0x81, 0x0C, 0x00, 0x05, 0xAA])?;
    /// assert_eq!(frame.message(), Err(BvllError::UnknownFunction { function: 0x0C }));
    ///
    /// // A trailing octet the Length field doesn't count.
    /// assert_eq!(
    ///     BvllFrame::decode(&[0x81, 0x06, 0x00, 0x04, 0x00]),
    ///     Err(BvllError::LengthMismatch { declared: 4, received: 5 }),
    /// );
    /// # Ok::<(), BvllError>(())
    /// ```
    pub fn decode(datagram: &'a [u8]) -> Result<Self, BvllError> {
        let [bvlc_type, function, high, low, payload @ ..] = datagram else {
            return Err(BvllError::TooShort {
                length: datagram.len(),
            });
        };
        if *bvlc_type != BVLC_TYPE_BIP {
            return Err(BvllError::NotBip {
                bvlc_type: *bvlc_type,
            });
        }
        let declared = usize::from(u16::from_be_bytes([*high, *low]));
        if declared != datagram.len() {
            return Err(BvllError::LengthMismatch {
                declared,
                received: datagram.len(),
            });
        }
        Ok(Self {
            function: BvlcFunction::from_raw(*function),
            payload,
        })
    }

    /// Read the payload by the function's layout (see
    /// [`BvlcFunction::payload_shape`]).
    ///
    /// Fails with [`BvllError::UnknownFunction`] for a function Annex J
    /// doesn't define, and with [`BvllError::PayloadLength`] when the payload
    /// is the wrong length for its function. Field values are not judged: an
    /// undefined result code, a zero time-to-live, any mask or address, and
    /// an empty NPDU all decode.
    pub fn message(self) -> Result<Bvll<'a>, BvllError> {
        let function = self.function;
        let wrong_length = BvllError::PayloadLength {
            function,
            length: self.payload.len(),
        };
        Ok(match (function, self.payload) {
            (BvlcFunction::BVLC_RESULT, &[high, low]) => {
                Bvll::BvlcResult(BvlcResultCode::from_raw(u16::from_be_bytes([high, low])))
            }
            (BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE, table) => {
                Bvll::WriteBroadcastDistributionTable(
                    BdtEntries::from_wire(table).map_err(|_| wrong_length)?,
                )
            }
            (BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE, []) => {
                Bvll::ReadBroadcastDistributionTable
            }
            (BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK, table) => {
                Bvll::ReadBroadcastDistributionTableAck(
                    BdtEntries::from_wire(table).map_err(|_| wrong_length)?,
                )
            }
            (BvlcFunction::FORWARDED_NPDU, &[a, b, c, d, high, low, ref npdu @ ..]) => {
                Bvll::ForwardedNpdu {
                    origin: address_from_octets([a, b, c, d, high, low]),
                    npdu,
                }
            }
            (BvlcFunction::REGISTER_FOREIGN_DEVICE, &[high, low]) => Bvll::RegisterForeignDevice {
                ttl: u16::from_be_bytes([high, low]),
            },
            (BvlcFunction::READ_FOREIGN_DEVICE_TABLE, []) => Bvll::ReadForeignDeviceTable,
            (BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK, table) => {
                Bvll::ReadForeignDeviceTableAck(
                    FdtEntries::from_wire(table).map_err(|_| wrong_length)?,
                )
            }
            (BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY, &[a, b, c, d, high, low]) => {
                Bvll::DeleteForeignDeviceTableEntry(address_from_octets([a, b, c, d, high, low]))
            }
            (BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK, npdu) => {
                Bvll::DistributeBroadcastToNetwork(npdu)
            }
            (BvlcFunction::ORIGINAL_UNICAST_NPDU, npdu) => Bvll::OriginalUnicastNpdu(npdu),
            (BvlcFunction::ORIGINAL_BROADCAST_NPDU, npdu) => Bvll::OriginalBroadcastNpdu(npdu),
            _ if function.is_defined() => return Err(wrong_length),
            _ => {
                return Err(BvllError::UnknownFunction {
                    function: function.to_raw(),
                })
            }
        })
    }
}

/// Decode one datagram as a BVLL message: [`BvllFrame::decode`], then
/// [`BvllFrame::message`].
///
/// ```
/// use core::net::{Ipv4Addr, SocketAddrV4};
/// use bacnet_bip_codec::{decode, Bvll, BvllError};
///
/// // A Forwarded-NPDU from 192.168.1.20:47808 carrying the NPDU 01 00.
/// let wire = [0x81, 0x04, 0x00, 0x0C, 192, 168, 1, 20, 0xBA, 0xC0, 0x01, 0x00];
/// assert_eq!(
///     decode(&wire)?,
///     Bvll::ForwardedNpdu {
///         origin: SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 47808),
///         npdu: &[0x01, 0x00],
///     },
/// );
///
/// // A Register-Foreign-Device needs exactly a 2-octet time-to-live.
/// assert!(matches!(
///     decode(&[0x81, 0x05, 0x00, 0x05, 0x3C]),
///     Err(BvllError::PayloadLength { length: 1, .. }),
/// ));
/// # Ok::<(), BvllError>(())
/// ```
pub fn decode(datagram: &[u8]) -> Result<Bvll<'_>, BvllError> {
    BvllFrame::decode(datagram)?.message()
}

/// One B/IP BVLL message (Annex J.2), borrowing its NPDU or table.
///
/// [`decode`] produces one from a datagram, and
/// [`encode_into`](Bvll::encode_into) or
/// [`encode_to_sink`](Bvll::encode_to_sink) writes one. Each variant holds
/// what its function's format clause lays out after the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bvll<'a> {
    /// BVLC-Result (J.2.1).
    BvlcResult(BvlcResultCode),
    /// Write-Broadcast-Distribution-Table (J.2.2).
    WriteBroadcastDistributionTable(BdtEntries<'a>),
    /// Read-Broadcast-Distribution-Table (J.2.3).
    ReadBroadcastDistributionTable,
    /// Read-Broadcast-Distribution-Table-Ack (J.2.4).
    ReadBroadcastDistributionTableAck(BdtEntries<'a>),
    /// Forwarded-NPDU (J.2.5).
    ForwardedNpdu {
        /// Where the NPDU started: its originator's B/IP address. A receiver
        /// takes it as the NPDU's source when it differs from the sender's
        /// address (J.2.5).
        origin: SocketAddrV4,
        /// The NPDU.
        npdu: &'a [u8],
    },
    /// Register-Foreign-Device (J.2.6).
    RegisterForeignDevice {
        /// Seconds within which the device will register again.
        ttl: u16,
    },
    /// Read-Foreign-Device-Table (J.2.7).
    ReadForeignDeviceTable,
    /// Read-Foreign-Device-Table-Ack (J.2.8).
    ReadForeignDeviceTableAck(FdtEntries<'a>),
    /// Delete-Foreign-Device-Table-Entry (J.2.9), naming the entry's B/IP
    /// address.
    DeleteForeignDeviceTableEntry(SocketAddrV4),
    /// Distribute-Broadcast-To-Network (J.2.10), carrying an NPDU.
    DistributeBroadcastToNetwork(&'a [u8]),
    /// Original-Unicast-NPDU (J.2.11), carrying an NPDU.
    OriginalUnicastNpdu(&'a [u8]),
    /// Original-Broadcast-NPDU (J.2.12), carrying an NPDU.
    OriginalBroadcastNpdu(&'a [u8]),
}

impl<'a> Bvll<'a> {
    /// The message's function code.
    #[must_use]
    pub const fn function(&self) -> BvlcFunction {
        match self {
            Self::BvlcResult(_) => BvlcFunction::BVLC_RESULT,
            Self::WriteBroadcastDistributionTable(_) => {
                BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE
            }
            Self::ReadBroadcastDistributionTable => BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            Self::ReadBroadcastDistributionTableAck(_) => {
                BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
            }
            Self::ForwardedNpdu { .. } => BvlcFunction::FORWARDED_NPDU,
            Self::RegisterForeignDevice { .. } => BvlcFunction::REGISTER_FOREIGN_DEVICE,
            Self::ReadForeignDeviceTable => BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
            Self::ReadForeignDeviceTableAck(_) => BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
            Self::DeleteForeignDeviceTableEntry(_) => {
                BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
            }
            Self::DistributeBroadcastToNetwork(_) => BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
            Self::OriginalUnicastNpdu(_) => BvlcFunction::ORIGINAL_UNICAST_NPDU,
            Self::OriginalBroadcastNpdu(_) => BvlcFunction::ORIGINAL_BROADCAST_NPDU,
        }
    }

    /// The NPDU, for the four functions that carry one.
    #[must_use]
    pub const fn npdu(&self) -> Option<&'a [u8]> {
        match *self {
            Self::ForwardedNpdu { npdu, .. }
            | Self::DistributeBroadcastToNetwork(npdu)
            | Self::OriginalUnicastNpdu(npdu)
            | Self::OriginalBroadcastNpdu(npdu) => Some(npdu),
            _ => None,
        }
    }

    /// A Forwarded-NPDU's originating address.
    #[must_use]
    pub const fn origin(&self) -> Option<SocketAddrV4> {
        match *self {
            Self::ForwardedNpdu { origin, .. } => Some(origin),
            _ => None,
        }
    }

    /// Octets after the header.
    const fn payload_length(&self) -> usize {
        match self {
            Self::BvlcResult(_) | Self::RegisterForeignDevice { .. } => 2,
            Self::ReadBroadcastDistributionTable | Self::ReadForeignDeviceTable => 0,
            Self::WriteBroadcastDistributionTable(table)
            | Self::ReadBroadcastDistributionTableAck(table) => table.wire_length(),
            Self::ReadForeignDeviceTableAck(table) => table.wire_length(),
            Self::DeleteForeignDeviceTableEntry(_) => 6,
            Self::ForwardedNpdu { npdu, .. } => 6 + npdu.len(),
            Self::DistributeBroadcastToNetwork(npdu)
            | Self::OriginalUnicastNpdu(npdu)
            | Self::OriginalBroadcastNpdu(npdu) => npdu.len(),
        }
    }

    /// Check the message and return its wire length, header included.
    ///
    /// Encoding refuses two things: a message too long for the 16-bit Length
    /// field ([`BvllError::TooLong`]), and a BVLC-Result whose code Annex
    /// J.2.1.1 doesn't define ([`BvllError::UndefinedResultCode`]). Every
    /// other shape the standard forbids can't be built: each variant holds
    /// exactly its function's fields.
    pub fn encoded_len(&self) -> Result<usize, BvllError> {
        if let Self::BvlcResult(code) = self {
            if !code.is_defined() {
                return Err(BvllError::UndefinedResultCode {
                    code: code.to_raw(),
                });
            }
        }
        let length = HEADER_LENGTH + self.payload_length();
        if length > MAX_BVLL_LENGTH {
            return Err(BvllError::TooLong { length });
        }
        Ok(length)
    }

    /// Encode the message at the start of `output` and return its wire
    /// length. Nothing is written unless it passes the checks of
    /// [`Bvll::encoded_len`] and fits.
    ///
    /// ```
    /// use bacnet_bip_codec::{Bvll, BvlcResultCode, BvllError};
    ///
    /// let mut wire = [0u8; 6];
    /// let nak = Bvll::BvlcResult(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
    /// assert_eq!(nak.encode_into(&mut wire)?, 6);
    /// assert_eq!(wire, [0x81, 0x00, 0x00, 0x06, 0x00, 0x30]);
    ///
    /// // 0x0070 is not a result code Annex J defines.
    /// assert_eq!(
    ///     Bvll::BvlcResult(BvlcResultCode::from_raw(0x0070)).encode_into(&mut wire),
    ///     Err(BvllError::UndefinedResultCode { code: 0x0070 }),
    /// );
    /// # Ok::<(), BvllError>(())
    /// ```
    pub fn encode_into(&self, output: &mut [u8]) -> Result<usize, BvllError> {
        let mut sink = output;
        self.encode_to_sink(&mut sink)
    }

    /// Encode the message into `sink` and return its wire length.
    ///
    /// The checks are those of [`Bvll::encoded_len`]. When the sink reports
    /// a fixed capacity, a message that does not fit fails before anything
    /// is written. A growable sink is written directly, so if one of its
    /// writes fails, the octets written before it stay in the sink; see
    /// [`BvllSink`].
    pub fn encode_to_sink<S: BvllSink + ?Sized>(&self, sink: &mut S) -> Result<usize, BvllError> {
        let length = self.encoded_len()?;
        if let Some(available) = sink.remaining_capacity() {
            if available < length {
                return Err(BvllError::BufferExhausted {
                    needed: length,
                    available,
                });
            }
        }
        // `encoded_len` capped the length at 0xFFFF.
        let [high, low] = (length as u16).to_be_bytes();
        sink.write(&[BVLC_TYPE_BIP, self.function().to_raw(), high, low])?;
        match self {
            Self::BvlcResult(code) => sink.write(&code.to_raw().to_be_bytes())?,
            Self::RegisterForeignDevice { ttl } => sink.write(&ttl.to_be_bytes())?,
            Self::ReadBroadcastDistributionTable | Self::ReadForeignDeviceTable => {}
            Self::WriteBroadcastDistributionTable(table)
            | Self::ReadBroadcastDistributionTableAck(table) => table.write_to(sink)?,
            Self::ReadForeignDeviceTableAck(table) => table.write_to(sink)?,
            Self::DeleteForeignDeviceTableEntry(address) => {
                sink.write(&address_to_octets(*address))?
            }
            Self::ForwardedNpdu { origin, npdu } => {
                sink.write(&address_to_octets(*origin))?;
                sink.write(npdu)?;
            }
            Self::DistributeBroadcastToNetwork(npdu)
            | Self::OriginalUnicastNpdu(npdu)
            | Self::OriginalBroadcastNpdu(npdu) => sink.write(npdu)?,
        }
        Ok(length)
    }
}

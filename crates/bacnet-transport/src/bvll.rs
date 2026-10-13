//! The B/IP transport's side of the BVLL codec (Annex J).
//!
//! All BVLL parsing and encoding lives in [`bacnet_bip_codec`]. This module
//! adds what the hosted transport needs around it: an owned copy of a
//! received frame ([`BvllMessage`]) for state that outlives the receive
//! buffer, encoding into `BytesMut`, six-octet B/IP MAC helpers, and
//! [`bvll_error`], which maps a codec error to [`Error`].
//!
//! Every codec item that a public transport signature names is re-exported
//! here under its codec name, so a crate that depends on the transport can
//! build and read B/IP messages without depending on the codec too: the
//! message and error types, the function and result codes, the table views
//! and entries, the address helpers and the wire-length constants.
//!
//! [`BdtEntry`] and [`FdtEntry`] here are table rows as they travel on the
//! wire, with `SocketAddrV4` addresses. They are not
//! [`bbmd::BdtEntry`](crate::bbmd::BdtEntry) and
//! [`bbmd::FdtEntry`](crate::bbmd::FdtEntry), the BBMD's own table rows;
//! `From` impls and [`bbmd::bdt_from_wire`](crate::bbmd::bdt_from_wire)
//! convert between them. Import one pair by its module path where both are
//! in scope.
//!
//! ```
//! use bacnet_transport::bvll::{
//!     decode_bvll, BdtEntries, BdtEntry, Bvll, BvlcFunction, FdtEntries, FdtEntry,
//!     BDT_ENTRY_LENGTH, FDT_ENTRY_LENGTH, HEADER_LENGTH,
//! };
//! use std::net::{Ipv4Addr, SocketAddrV4};
//!
//! // Build a Write-Broadcast-Distribution-Table from wire rows.
//! let peer = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 47808);
//! let rows = [BdtEntry::new(peer, Ipv4Addr::BROADCAST)];
//! let write = Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&rows));
//! let mut wire = [0u8; 64];
//! let length = write.encode_into(&mut wire).unwrap();
//! assert_eq!(length, HEADER_LENGTH + BDT_ENTRY_LENGTH);
//! assert_eq!(decode_bvll(&wire[..length])?.message()?, write);
//!
//! // Read the rows of a Read-Foreign-Device-Table-Ack.
//! let device = FdtEntry::new(peer, 60, 75);
//! let registered = [device];
//! let ack = Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(&registered));
//! let length = ack.encode_into(&mut wire).unwrap();
//! assert_eq!(length, HEADER_LENGTH + FDT_ENTRY_LENGTH);
//! let received = decode_bvll(&wire[..length])?;
//! assert_eq!(received.function, BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK);
//! let Bvll::ReadForeignDeviceTableAck(table) = received.message()? else {
//!     unreachable!("a Read-FDT-Ack");
//! };
//! assert_eq!(table.iter().collect::<Vec<_>>(), [device]);
//! let rows = bacnet_transport::bbmd::fdt_from_wire(table)?;
//! assert_eq!((rows[0].ttl, rows[0].seconds_remaining), (60, 75));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};

pub use bacnet_bip_codec::{
    address_from_octets, address_from_slice, address_to_octets, BdtEntries, BdtEntry, BvlcFunction,
    BvlcResultCode, Bvll, BvllError, BvllFrame, BvllSink, Entries, EntryIter, FdtEntries, FdtEntry,
    PayloadShape, SinkFull, TableEntry, BDT_ENTRY_LENGTH, BIP_ADDRESS_LENGTH, BVLC_TYPE_BIP,
    DEFAULT_PORT, FDT_ENTRY_LENGTH, HEADER_LENGTH, MAX_BVLL_LENGTH,
};

/// Map a codec error to the transport's [`Error`].
///
/// Decode failures become [`Error::Decoding`] at the offset of the field at
/// fault: 0 for the datagram as a whole, its type octet or a B/IP address,
/// 1 for the function, 2 for the Length field, 4 for the payload. Encode
/// failures become [`Error::Encoding`], and so does an error the codec adds
/// later, until it is placed here.
pub fn bvll_error(error: BvllError) -> Error {
    let offset = match error {
        BvllError::TooShort { .. }
        | BvllError::NotBip { .. }
        | BvllError::TableLength { .. }
        | BvllError::AddressLength { .. } => 0,
        BvllError::UnknownFunction { .. } => 1,
        BvllError::LengthMismatch { .. } => 2,
        BvllError::PayloadLength { .. } => 4,
        BvllError::TooLong { .. }
        | BvllError::UndefinedResultCode { .. }
        | BvllError::BufferExhausted { .. } => return Error::Encoding(error.to_string()),
        _ => return Error::Encoding(error.to_string()),
    };
    Error::decoding(offset, error.to_string())
}

/// An owned copy of a received BVLL frame: the header checked, the payload
/// kept as received.
///
/// [`decode_bvll`] makes one; [`BvllMessage::message`] reads the payload by
/// its function's layout. The transport keeps one where a frame outlives the
/// receive buffer, such as a management response handed to the request
/// waiting for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BvllMessage {
    /// The function octet.
    pub function: BvlcFunction,
    /// The octets after the 4-octet header. For a Forwarded-NPDU this
    /// includes the originating address; [`BvllMessage::npdu`] skips it.
    pub payload: Bytes,
}

impl From<BvllFrame<'_>> for BvllMessage {
    fn from(frame: BvllFrame<'_>) -> Self {
        Self {
            function: frame.function,
            payload: Bytes::copy_from_slice(frame.payload),
        }
    }
}

impl BvllMessage {
    /// The frame this message holds.
    pub fn frame(&self) -> BvllFrame<'_> {
        BvllFrame {
            function: self.function,
            payload: &self.payload,
        }
    }

    /// Read the payload by the function's layout (Annex J.2).
    pub fn message(&self) -> Result<Bvll<'_>, Error> {
        self.frame().message().map_err(bvll_error)
    }

    /// The NPDU of a well-formed Forwarded-NPDU,
    /// Distribute-Broadcast-To-Network or Original message, sharing this
    /// message's buffer.
    pub fn npdu(&self) -> Option<Bytes> {
        let npdu = self.message().ok()?.npdu()?;
        Some(self.payload.slice_ref(npdu))
    }

    /// The originating address of a well-formed Forwarded-NPDU.
    pub fn origin(&self) -> Option<SocketAddrV4> {
        self.message().ok()?.origin()
    }
}

/// Decode a datagram's BVLL header into an owned [`BvllMessage`].
///
/// The datagram must be one whole B/IP message: BVLC type 0x81 and a Length
/// field equal to its length. Any function code passes; use
/// [`BvllMessage::message`] to read the payload.
pub fn decode_bvll(data: &[u8]) -> Result<BvllMessage, Error> {
    BvllFrame::decode(data)
        .map(BvllMessage::from)
        .map_err(bvll_error)
}

/// Encode `message` and append it to `buf`.
///
/// Fails, leaving `buf` as it was, when the codec refuses the message: one
/// longer than the 16-bit Length field allows, or a BVLC-Result code that
/// Annex J.2.1.1 doesn't define.
pub fn encode_bvll(buf: &mut BytesMut, message: &Bvll<'_>) -> Result<(), Error> {
    let length = message.encoded_len().map_err(bvll_error)?;
    buf.reserve(length);
    message.encode_to_sink(buf).map_err(bvll_error)?;
    Ok(())
}

/// Encode a 6-byte BACnet/IP MAC address from IPv4 + port (Annex J.1.2).
pub fn encode_bip_mac(ip: [u8; 4], port: u16) -> [u8; 6] {
    bacnet_bip_codec::address_to_octets(SocketAddrV4::new(Ipv4Addr::from(ip), port))
}

/// Decode a 6-byte BACnet/IP MAC address into IPv4 + port (Annex J.1.2).
pub fn decode_bip_mac(mac: &[u8]) -> Result<([u8; 4], u16), Error> {
    let address = bacnet_bip_codec::address_from_slice(mac).map_err(bvll_error)?;
    Ok((address.ip().octets(), address.port()))
}

/// A frame with any function and payload and a matching Length field,
/// bypassing the codec's checks, for tests that send malformed messages.
#[cfg(test)]
pub(crate) fn raw_frame(function: BvlcFunction, payload: &[u8]) -> BytesMut {
    let length = u16::try_from(4 + payload.len()).expect("test frame fits a Length field");
    let mut frame = BytesMut::with_capacity(4 + payload.len());
    frame.extend_from_slice(&[bacnet_bip_codec::BVLC_TYPE_BIP, function.to_raw()]);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A received Forwarded-NPDU from `origin` carrying `npdu`, for tests.
#[cfg(test)]
pub(crate) fn forwarded_message(origin: ([u8; 4], u16), npdu: &[u8]) -> BvllMessage {
    let mut payload = encode_bip_mac(origin.0, origin.1).to_vec();
    payload.extend_from_slice(npdu);
    BvllMessage {
        function: BvlcFunction::FORWARDED_NPDU,
        payload: payload.into(),
    }
}

/// The wire octets of a Forwarded-NPDU from `origin` carrying `npdu`, for
/// tests.
#[cfg(test)]
pub(crate) fn forwarded_wire(origin: ([u8; 4], u16), npdu: &[u8]) -> BytesMut {
    let mut wire = BytesMut::new();
    let message = Bvll::ForwardedNpdu {
        origin: SocketAddrV4::new(Ipv4Addr::from(origin.0), origin.1),
        npdu,
    };
    encode_bvll(&mut wire, &message).expect("test Forwarded-NPDU encodes");
    wire
}

/// A Forwarded-NPDU's origin as the transport's `(ip, port)` pair, for tests.
#[cfg(test)]
pub(crate) fn origin_of(message: &BvllMessage) -> Option<([u8; 4], u16)> {
    message
        .origin()
        .map(|origin| (origin.ip().octets(), origin.port()))
}

#[cfg(test)]
mod tests;

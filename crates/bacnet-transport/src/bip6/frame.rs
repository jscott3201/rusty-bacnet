//! The B/IPv6 transport's side of the BVLL codec: decoding and encoding with
//! the transport's [`Error`].

use bacnet_bip_codec::ipv6::{decode, Bvlc6Function, Bvll6, Bvll6Error};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

/// Map a codec error to the transport's [`Error`].
///
/// Decode failures become a static [`Error::Decoding`] at the offset of the
/// field at fault: 0 for the datagram as a whole, its type octet or a
/// B/IPv6 address, 1 for the function, 2 for the Length field, 4 for the
/// payload. The text is the codec's, octets in upper-case hexadecimal
/// (#1803). Encode failures become [`Error::Encoding`], and so does an error
/// the codec adds later, until it is placed here.
pub fn bvll6_error(error: Bvll6Error) -> Error {
    let (offset, label, values) = match error {
        Bvll6Error::TooShort { length } => (
            0,
            "BVLC6 datagram too short: {} octets, header needs 4",
            [length as u64, 0],
        ),
        Bvll6Error::NotBip6 { bvlc_type } => (
            0,
            "BVLC type {:#04X} is not B/IPv6 (0x82)",
            [bvlc_type.into(), 0],
        ),
        Bvll6Error::AddressLength { length } => (
            0,
            "B/IPv6 address must be 18 octets, got {}",
            [length as u64, 0],
        ),
        Bvll6Error::UnknownFunction { function } => {
            (1, "unknown BVLC6 function {:#04X}", [function.into(), 0])
        }
        Bvll6Error::LengthMismatch { declared, received } => (
            2,
            "BVLC6 length {} does not match the {}-octet datagram",
            [declared as u64, received as u64],
        ),
        Bvll6Error::PayloadLength { function, length } => {
            let label = crate::bvll::payload_label!(
                function,
                Bvlc6Function: BVLC_RESULT,
                ORIGINAL_UNICAST_NPDU,
                ORIGINAL_BROADCAST_NPDU,
                ADDRESS_RESOLUTION,
                FORWARDED_ADDRESS_RESOLUTION,
                ADDRESS_RESOLUTION_ACK,
                VIRTUAL_ADDRESS_RESOLUTION,
                VIRTUAL_ADDRESS_RESOLUTION_ACK,
                FORWARDED_NPDU,
                REGISTER_FOREIGN_DEVICE,
                DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
                DISTRIBUTE_BROADCAST_TO_NETWORK,
            );
            return crate::bvll::payload_refusal(label, function.to_raw(), length);
        }
        _ => return Error::Encoding(error.to_string()),
    };
    let mut refused = DecodeError::new(DecodingKind::InvalidEncoding, offset, label);
    refused.values = values;
    refused.into()
}

/// Decode one datagram as a B/IPv6 BVLL message, borrowing its NPDU.
///
/// The datagram must be one whole message: BVLC type 0x82, a Length field
/// equal to its length, a function Annex U defines, and a payload of that
/// function's length. Field values are not judged.
pub fn decode_bvll6(data: &[u8]) -> Result<Bvll6<'_>, Error> {
    decode(data).map_err(bvll6_error)
}

/// Encode `message` and append it to `buf`.
///
/// Fails, leaving `buf` as it was, when the codec refuses the message: one
/// longer than the 16-bit Length field allows, or a BVLC-Result code that
/// Annex U.2.1.1 doesn't define.
pub fn encode_bvll6(buf: &mut BytesMut, message: &Bvll6<'_>) -> Result<(), Error> {
    let length = message.encoded_len().map_err(bvll6_error)?;
    buf.reserve(length);
    message.encode_to_sink(buf).map_err(bvll6_error)?;
    Ok(())
}

/// `message` encoded into a buffer of its own.
pub(super) fn encoded(message: &Bvll6<'_>) -> Result<BytesMut, Error> {
    let mut buf = BytesMut::new();
    encode_bvll6(&mut buf, message)?;
    Ok(buf)
}

/// A datagram with any function and payload and a matching Length field,
/// bypassing the codec's checks, for tests that send malformed messages.
#[cfg(test)]
pub(super) fn raw_frame(function: u8, payload: &[u8]) -> BytesMut {
    let length = u16::try_from(4 + payload.len()).expect("test frame fits a Length field");
    let mut frame = BytesMut::with_capacity(4 + payload.len());
    frame.extend_from_slice(&[bacnet_bip_codec::ipv6::BVLC_TYPE_BIP6, function]);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_errors_map_to_their_field() {
        let decoding = [
            (
                Bvll6Error::TooShort { length: 2 },
                0,
                "BVLC6 datagram too short: 2 octets, header needs 4",
            ),
            (
                Bvll6Error::NotBip6 { bvlc_type: 0x81 },
                0,
                "BVLC type 0x81 is not B/IPv6 (0x82)",
            ),
            (
                Bvll6Error::AddressLength { length: 6 },
                0,
                "B/IPv6 address must be 18 octets, got 6",
            ),
            (
                Bvll6Error::UnknownFunction { function: 0x0B },
                1,
                "unknown BVLC6 function 0x0B",
            ),
            (
                Bvll6Error::LengthMismatch {
                    declared: 7,
                    received: 8,
                },
                2,
                "BVLC6 length 7 does not match the 8-octet datagram",
            ),
            (
                Bvll6Error::PayloadLength {
                    function: Bvlc6Function::FORWARDED_NPDU,
                    length: 3,
                },
                4,
                "FORWARDED_NPDU cannot carry a 3-octet payload",
            ),
            (
                Bvll6Error::PayloadLength {
                    function: Bvlc6Function::from_raw(0x7F),
                    length: 3,
                },
                4,
                "127 cannot carry a 3-octet payload",
            ),
        ];
        for (error, offset, text) in decoding {
            match bvll6_error(error) {
                Error::Decoding(refused) => {
                    assert_eq!(refused.offset, offset, "{error:?}");
                    assert_eq!(refused.message().to_string(), text, "{error:?}");
                }
                other => panic!("{error:?} mapped to {other:?}"),
            }
            // The hex octets read as the codec's own text does (#1803).
            if matches!(
                error,
                Bvll6Error::NotBip6 { .. } | Bvll6Error::UnknownFunction { .. }
            ) {
                assert_eq!(text, error.to_string());
            }
        }
        let encoding = [
            Bvll6Error::TooLong { length: 65536 },
            Bvll6Error::UndefinedResultCode { code: 0x10 },
            Bvll6Error::BufferExhausted {
                needed: 9,
                available: 0,
            },
        ];
        for error in encoding {
            assert!(
                matches!(bvll6_error(error), Error::Encoding(ref message) if *message == error.to_string()),
                "{error:?}"
            );
        }
    }

    #[test]
    fn a_refused_message_leaves_the_buffer_as_it_was() {
        use bacnet_bip_codec::ipv6::{Bip6Vmac, Bvlc6ResultCode};

        let mut buf = BytesMut::from(&[0xAA][..]);
        let undefined = Bvll6::BvlcResult {
            source: Bip6Vmac::new([1, 2, 3]),
            code: Bvlc6ResultCode::from_raw(0x0010),
        };
        assert!(matches!(
            encode_bvll6(&mut buf, &undefined),
            Err(Error::Encoding(_))
        ));
        assert_eq!(buf[..], [0xAA]);
    }
}

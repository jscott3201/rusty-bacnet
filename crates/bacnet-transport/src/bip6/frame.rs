//! The B/IPv6 transport's side of the BVLL codec: decoding and encoding with
//! the transport's [`Error`].

use bacnet_bip_codec::ipv6::{decode, Bvll6, Bvll6Error};
use bacnet_types::error::Error;
use bytes::BytesMut;

/// Map a codec error to the transport's [`Error`].
///
/// Decode failures become [`Error::Decoding`] at the offset of the field at
/// fault: 0 for the datagram as a whole, its type octet or a B/IPv6 address,
/// 1 for the function, 2 for the Length field, 4 for the payload. Encode
/// failures become [`Error::Encoding`], and so does an error the codec adds
/// later, until it is placed here.
pub fn bvll6_error(error: Bvll6Error) -> Error {
    let offset = match error {
        Bvll6Error::TooShort { .. }
        | Bvll6Error::NotBip6 { .. }
        | Bvll6Error::AddressLength { .. } => 0,
        Bvll6Error::UnknownFunction { .. } => 1,
        Bvll6Error::LengthMismatch { .. } => 2,
        Bvll6Error::PayloadLength { .. } => 4,
        Bvll6Error::TooLong { .. }
        | Bvll6Error::UndefinedResultCode { .. }
        | Bvll6Error::BufferExhausted { .. } => return Error::Encoding(error.to_string()),
        _ => return Error::Encoding(error.to_string()),
    };
    Error::decoding(offset, error.to_string())
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
    use bacnet_bip_codec::ipv6::Bvlc6Function;

    #[test]
    fn codec_errors_map_to_their_field() {
        let decoding = [
            (Bvll6Error::TooShort { length: 2 }, 0),
            (Bvll6Error::NotBip6 { bvlc_type: 0x81 }, 0),
            (Bvll6Error::AddressLength { length: 6 }, 0),
            (Bvll6Error::UnknownFunction { function: 0x0B }, 1),
            (
                Bvll6Error::LengthMismatch {
                    declared: 7,
                    received: 8,
                },
                2,
            ),
            (
                Bvll6Error::PayloadLength {
                    function: Bvlc6Function::FORWARDED_NPDU,
                    length: 3,
                },
                4,
            ),
        ];
        for (error, offset) in decoding {
            match bvll6_error(error) {
                Error::Decoding {
                    offset: at,
                    message,
                    ..
                } => {
                    assert_eq!(at, offset, "{error:?}");
                    assert_eq!(message, error.to_string());
                }
                other => panic!("{error:?} mapped to {other:?}"),
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

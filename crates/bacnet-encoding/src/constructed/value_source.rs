//! BACnetValueSource CHOICE framing (ASHRAE 135-2020 Clause 21).
use bacnet_types::constructed::{BACnetAddress, BACnetValueSource};
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

use super::recipient::{check_encoded_mac_len, decode_app_mac_address};
use super::tagged::{decode_app_unsigned, expect_closing};
use super::{decode_device_object_reference, encode_device_object_reference};
use crate::{primitives, tags};

const WHAT: &str = "BACnetValueSource";

/// Encode one ValueSource CHOICE, appending to `buf`.
///
/// The generic codec preserves legal wire object types, wildcard instances,
/// local network zero and empty (broadcast) MAC addresses. Source identity and
/// authorization are mechanism-level concerns, not datatype restrictions.
///
/// A MAC longer than [`BACnetAddress::MAX_MAC_LEN`] octets, which the decoder
/// refuses, returns an error before modifying the output buffer (#1156).
pub fn encode_value_source(buf: &mut BytesMut, source: &BACnetValueSource) -> Result<(), Error> {
    if let BACnetValueSource::Address(address) = source {
        check_encoded_mac_len(&address.mac_address, WHAT)?;
    }
    match source {
        BACnetValueSource::None => tags::encode_tag(buf, 0, tags::TagClass::Context, 0),
        BACnetValueSource::Object(reference) => {
            tags::encode_opening_tag(buf, 1);
            encode_device_object_reference(buf, reference);
            tags::encode_closing_tag(buf, 1);
        }
        BACnetValueSource::Address(address) => {
            tags::encode_opening_tag(buf, 2);
            primitives::encode_app_unsigned(buf, u64::from(address.network_number));
            primitives::encode_app_octet_string(buf, &address.mac_address);
            tags::encode_closing_tag(buf, 2);
        }
    }
    Ok(())
}

/// Decode one ValueSource CHOICE at `offset`, returning its value and next offset.
///
/// Bytes after that CHOICE are left for the caller, allowing concatenated array
/// elements. A full-property consumer must check that the returned offset equals
/// the payload length. Structural acceptance does not establish a valid or
/// authorized command source. An address MAC longer than
/// [`BACnetAddress::MAX_MAC_LEN`] octets is refused before it is copied (#1156).
pub fn decode_value_source(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetValueSource, usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.is_context(0) && tag.length == 0 {
        return Ok((BACnetValueSource::None, pos));
    }
    if tag.is_opening_tag(1) {
        let (reference, end) = decode_device_object_reference(data, pos)?;
        let end = expect_closing(data, end, 1, WHAT)?;
        return Ok((BACnetValueSource::Object(reference), end));
    }
    if tag.is_opening_tag(2) {
        let (network, pos) = decode_app_unsigned::<u64>(data, pos, WHAT)?;
        let network_number = u16::try_from(network).map_err(|_| {
            DecodeError::new(
                DecodingKind::OutOfRange,
                pos,
                "network number exceeds Unsigned16",
            )
            .within(WHAT)
        })?;
        let (mac_address, pos) = decode_app_mac_address(data, pos, WHAT)?;
        let end = expect_closing(data, pos, 2, WHAT)?;
        return Ok((
            BACnetValueSource::Address(BACnetAddress {
                network_number,
                mac_address,
            }),
            end,
        ));
    }
    Err(DecodeError::new(
        DecodingKind::InvalidEncoding,
        offset,
        "expected NULL [0], object [1], or address [2]",
    )
    .within(WHAT)
    .into())
}

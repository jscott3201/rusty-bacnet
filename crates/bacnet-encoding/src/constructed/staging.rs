//! Clause 21 codecs used by the Staging object.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

pub use super::reference_sinks::encode_device_object_reference_to_sink;
use super::tagged::{decode_ctx_object_id, decode_optional_ctx};
use crate::primitives;

/// Encode one unframed `BACnetStageLimitValue` SEQUENCE.
pub fn encode_stage_limit_value(buf: &mut BytesMut, value: &BACnetStageLimitValue) {
    primitives::encode_app_real(buf, value.limit);
    let (unused_bits, data) = pack_bits(&value.values);
    primitives::encode_app_bit_string(buf, unused_bits, &data);
    primitives::encode_app_real(buf, value.deadband);
}

/// Decode one unframed `BACnetStageLimitValue` SEQUENCE at `offset`.
pub fn decode_stage_limit_value(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetStageLimitValue, usize), Error> {
    let (limit, offset) = primitives::decode_application_value(data, offset)?;
    let PropertyValue::Real(limit) = limit else {
        return Err(Error::decoding(
            offset,
            "stage limit must be application REAL",
        ));
    };

    let (values, offset) = primitives::decode_application_value(data, offset)?;
    let PropertyValue::BitString {
        unused_bits,
        data: packed,
    } = values
    else {
        return Err(Error::decoding(
            offset,
            "stage values must be application BIT STRING",
        ));
    };
    let values = unpack_bits(unused_bits, &packed, offset)?;

    let (deadband, offset) = primitives::decode_application_value(data, offset)?;
    let PropertyValue::Real(deadband) = deadband else {
        return Err(Error::decoding(
            offset,
            "stage deadband must be application REAL",
        ));
    };

    Ok((
        BACnetStageLimitValue {
            limit,
            values,
            deadband,
        },
        offset,
    ))
}

/// Encode one unframed `BACnetDeviceObjectReference` SEQUENCE.
pub fn encode_device_object_reference(buf: &mut BytesMut, reference: &BACnetDeviceObjectReference) {
    encode_device_object_reference_to_sink(buf, reference)
        .expect("BytesMut accepts each reference");
}

/// Decode one unframed `BACnetDeviceObjectReference` SEQUENCE at `offset`.
pub fn decode_device_object_reference(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetDeviceObjectReference, usize), Error> {
    const WHAT: &str = "BACnetDeviceObjectReference";
    let (device_identifier, offset) =
        decode_optional_ctx(data, offset, 0, WHAT, decode_ctx_object_id)?;
    let (object_identifier, end) = decode_ctx_object_id(data, offset, 1, WHAT)?;
    Ok((
        BACnetDeviceObjectReference {
            device_identifier,
            object_identifier,
        },
        end,
    ))
}

fn pack_bits(values: &[bool]) -> (u8, Vec<u8>) {
    if values.is_empty() {
        return (0, Vec::new());
    }
    let mut data = vec![0; values.len().div_ceil(8)];
    for (index, value) in values.iter().enumerate() {
        if *value {
            data[index / 8] |= 0x80 >> (index % 8);
        }
    }
    ((data.len() * 8 - values.len()) as u8, data)
}

fn unpack_bits(unused_bits: u8, data: &[u8], offset: usize) -> Result<Vec<bool>, Error> {
    if data.is_empty() {
        if unused_bits == 0 {
            return Ok(Vec::new());
        }
        return Err(Error::decoding(
            offset,
            "empty stage values BIT STRING must declare zero unused bits",
        ));
    }
    let mask = if unused_bits == 0 {
        0
    } else {
        (1_u8 << unused_bits) - 1
    };
    if data.last().copied().unwrap_or_default() & mask != 0 {
        return Err(Error::decoding(
            offset,
            "stage values BIT STRING has nonzero padding bits",
        ));
    }
    let bit_len = data
        .len()
        .checked_mul(8)
        .and_then(|bits| bits.checked_sub(unused_bits as usize))
        .ok_or_else(|| Error::decoding(offset, "invalid stage values BIT STRING length"))?;
    Ok((0..bit_len)
        .map(|index| data[index / 8] & (0x80 >> (index % 8)) != 0)
        .collect())
}

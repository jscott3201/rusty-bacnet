//! Clause 21 codecs used by the Staging object.

use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use crate::primitives;
use crate::tags;

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
    if let Some(device) = &reference.device_identifier {
        primitives::encode_ctx_object_id(buf, 0, device);
    }
    primitives::encode_ctx_object_id(buf, 1, &reference.object_identifier);
}

/// Decode one unframed `BACnetDeviceObjectReference` SEQUENCE at `offset`.
pub fn decode_device_object_reference(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetDeviceObjectReference, usize), Error> {
    let mut offset = offset;
    let mut device_identifier = None;
    let (first, first_content) = tags::decode_tag(data, offset)?;
    if first.is_context(0) {
        let end = object_id_end(data, offset, first_content, first.length, 0)?;
        device_identifier = Some(ObjectIdentifier::decode(&data[first_content..end])?);
        offset = end;
    }

    let (object_tag, object_content) = tags::decode_tag(data, offset)?;
    if !object_tag.is_context(1) || object_tag.length != 4 {
        return Err(Error::decoding(
            offset,
            "device-object reference requires object-identifier [1]",
        ));
    }
    let end = object_id_end(data, offset, object_content, object_tag.length, 1)?;
    let object_identifier = ObjectIdentifier::decode(&data[object_content..end])?;

    Ok((
        BACnetDeviceObjectReference {
            device_identifier,
            object_identifier,
        },
        end,
    ))
}

fn object_id_end(
    data: &[u8],
    tag_offset: usize,
    content: usize,
    length: u32,
    tag: u8,
) -> Result<usize, Error> {
    if length != 4 {
        return Err(Error::decoding(
            tag_offset,
            format!("device-object reference [{tag}] must contain a 4-octet object identifier"),
        ));
    }
    let end = content
        .checked_add(4)
        .ok_or_else(|| Error::decoding(content, "device-object reference length overflow"))?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok(end)
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

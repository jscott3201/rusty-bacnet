//! Private bounded assembly for the five public reference sink encoders.

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetObjectPropertyReference, PropertyReference,
};
use bacnet_types::primitives::ObjectIdentifier;

use crate::primitives::{encode_ctx_object_id_to_sink, encode_ctx_unsigned_to_sink};
use crate::sink::{EncodeSink, SinkFull};
use crate::tags::{encode_closing_tag_to_sink, encode_opening_tag_to_sink};

/// Append one complete `BACnetPropertyReference` in one sink write.
///
/// This preserves [`super::encode_property_reference`]'s accepted inputs and
/// wire bytes. Its bounded assembly stages at most ten encoded bytes. A known
/// shortage returns the total required and available lengths without calling
/// `write`; an arbitrary sink refusal is returned unchanged. On failure, earlier
/// output, the cursor, container length and unused tail remain unchanged.
/// Earlier successful encoder calls stay committed if a later call fails.
///
/// No semantic validation is added. The staging bound is not a whole-stack
/// bound, and the crate still requires `std` or `alloc` with a bounded sink.
///
/// ```
/// use bacnet_encoding::constructed::encode_property_reference_to_sink;
/// use bacnet_types::constructed::PropertyReference;
/// use bacnet_types::enums::PropertyIdentifier;
///
/// let reference = PropertyReference {
///     property_identifier: PropertyIdentifier::PRESENT_VALUE,
///     property_array_index: None,
/// };
/// let mut storage = [0xA5; 2];
/// let mut tail = &mut storage[..];
/// assert_eq!(encode_property_reference_to_sink(&mut tail, &reference)?, 2);
/// assert!(tail.is_empty());
/// assert_eq!(storage, [0x09, 0x55]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub fn encode_property_reference_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    reference: &PropertyReference,
) -> Result<usize, SinkFull> {
    stage_and_write::<S, 10>(sink, |tail| {
        encode_ctx_unsigned_to_sink(tail, 0, u64::from(reference.property_identifier.to_raw()))?;
        if let Some(index) = reference.property_array_index {
            encode_ctx_unsigned_to_sink(tail, 1, u64::from(index))?;
        }
        Ok(())
    })
}

/// Append a complete bare `BACnetObjectPropertyReference` in one sink write.
///
/// This retains [`super::encode_object_property_reference`]'s bytes and input
/// handling with at most fifteen staging bytes. The [reference storage
/// contract](encode_property_reference_to_sink) applies to the whole sequence,
/// including its optional index. An absent index and `Some(0)` remain distinct.
pub fn encode_object_property_reference_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    reference: &BACnetObjectPropertyReference,
) -> Result<usize, SinkFull> {
    stage_and_write::<S, 15>(sink, |tail| {
        stage_object_property(
            tail,
            &reference.object_identifier,
            reference.property_identifier,
            reference.property_array_index,
        )
    })
}

/// Append a complete context-0 `BACnetSetpointReference` frame in one write.
///
/// This preserves [`super::encode_setpoint_reference`]'s bytes and input handling
/// with at most seventeen staging bytes. The [reference storage
/// contract](encode_property_reference_to_sink) includes both frame markers.
/// Callers still represent an absent optional setpoint by emitting nothing;
/// this function encodes a supplied reference and adds no optional-argument API.
pub fn encode_setpoint_reference_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    reference: &BACnetObjectPropertyReference,
) -> Result<usize, SinkFull> {
    stage_and_write::<S, 17>(sink, |tail| {
        encode_opening_tag_to_sink(tail, 0)?;
        stage_object_property(
            tail,
            &reference.object_identifier,
            reference.property_identifier,
            reference.property_array_index,
        )?;
        encode_closing_tag_to_sink(tail, 0)?;
        Ok(())
    })
}

/// Append a complete bare `BACnetDeviceObjectPropertyReference` in one write.
///
/// This preserves [`super::encode_device_object_property_reference`]'s bytes
/// and inputs with at most twenty staging bytes. The optional device remains
/// context 3 after the object, property and optional index. The [reference
/// storage contract](encode_property_reference_to_sink) covers all members;
/// it does not make enclosing variable-length encoders atomic.
pub fn encode_device_object_property_reference_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    reference: &BACnetDeviceObjectPropertyReference,
) -> Result<usize, SinkFull> {
    stage_and_write::<S, 20>(sink, |tail| {
        stage_object_property(
            tail,
            &reference.object_identifier,
            reference.property_identifier,
            reference.property_array_index,
        )?;
        if let Some(device) = &reference.device_identifier {
            encode_ctx_object_id_to_sink(tail, 3, device)?;
        }
        Ok(())
    })
}

/// Append a complete bare `BACnetDeviceObjectReference` in one sink write.
///
/// This retains [`super::encode_device_object_reference`]'s bytes and input
/// handling with at most ten staging bytes. The optional device uses context 0
/// before the required object in context 1. The [reference storage
/// contract](encode_property_reference_to_sink) applies to the whole sequence.
/// The encoder adds no validation of the supplied device identifier's type.
pub fn encode_device_object_reference_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    reference: &BACnetDeviceObjectReference,
) -> Result<usize, SinkFull> {
    stage_and_write::<S, 10>(sink, |tail| {
        if let Some(device) = &reference.device_identifier {
            encode_ctx_object_id_to_sink(tail, 0, device)?;
        }
        encode_ctx_object_id_to_sink(tail, 1, &reference.object_identifier)?;
        Ok(())
    })
}

fn stage_object_property(
    tail: &mut &mut [u8],
    object: &ObjectIdentifier,
    property: u32,
    index: Option<u32>,
) -> Result<(), SinkFull> {
    encode_ctx_object_id_to_sink(tail, 0, object)?;
    encode_ctx_unsigned_to_sink(tail, 1, u64::from(property))?;
    if let Some(index) = index {
        encode_ctx_unsigned_to_sink(tail, 2, u64::from(index))?;
    }
    Ok(())
}

fn stage_and_write<S: EncodeSink + ?Sized, const N: usize>(
    sink: &mut S,
    stage: impl FnOnce(&mut &mut [u8]) -> Result<(), SinkFull>,
) -> Result<usize, SinkFull> {
    let mut encoded = [0; N];
    let mut tail = &mut encoded[..];
    stage(&mut tail).expect("each reference fits its bounded staging buffer");
    let used = N - tail.len();
    if let Some(available) = sink.remaining_capacity()
        && available < used {
            return Err(SinkFull {
                needed: used,
                available,
            });
        }
    sink.write(&encoded[..used])?;
    Ok(used)
}

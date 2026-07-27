//! Shared `PropertyValue` encode/decode helpers for the event and fault
//! parameter CHOICEs.
//!
//! Split out of `mod.rs` to keep every file under the 700-LOC cap.

use super::*;

// ---- encode/decode helpers ----

/// Build a [`PropertyValue::BitString`] from an `(unused_bits, data)` pair.
pub(super) fn bitstring_pv((unused_bits, data): (u8, Vec<u8>)) -> PropertyValue {
    PropertyValue::BitString { unused_bits, data }
}

/// Build a [`PropertyValue`] for a [`BACnetPropertyStates`] tag.
///
/// `BACnetPropertyStates` is itself a CHOICE; absent full context-tagged
/// framing we carry its discriminant as the low byte and the raw payload as an
/// octet string. This round-trips through the flat-`List` encoding.
pub(super) fn property_state_pv(state: &BACnetPropertyStates) -> PropertyValue {
    let (tag, data) = property_state_parts(state);
    PropertyValue::List(vec![
        PropertyValue::Unsigned(tag as u64),
        PropertyValue::OctetString(data),
    ])
}

/// Extract `(tag, raw data)` from a [`BACnetPropertyStates`].
pub(super) fn property_state_parts(state: &BACnetPropertyStates) -> (u8, Vec<u8>) {
    match state {
        BACnetPropertyStates::BooleanValue(v) => (0u8, vec![u8::from(*v)]),
        BACnetPropertyStates::BinaryValue(v) => (1, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::EventType(v) => (2, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Polarity(v) => (3, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ProgramChange(v) => (4, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ProgramState(v) => (5, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ReasonForHalt(v) => (6, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Reliability(v) => (7, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::State(v) => (8, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::SystemStatus(v) => (9, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Units(v) => (10, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::UnsignedValue(v) => (11, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LifeSafetyMode(v) => (12, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LifeSafetyState(v) => (13, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorAlarmState(v) => (14, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Action(v) => (15, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorSecuredStatus(v) => (16, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorStatus(v) => (17, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorValue(v) => (18, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarDirection(v) => (40, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarDoorCommand(v) => (42, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::TimerState(v) => (38, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::TimerTransition(v) => (39, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Other { tag, data } => (*tag, data.clone()),
    }
}

/// Build a [`PropertyValue`] for a [`BACnetDeviceObjectPropertyReference`].
pub(super) fn device_object_property_reference_pv(
    r: &BACnetDeviceObjectPropertyReference,
) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::ObjectIdentifier(r.object_identifier),
        PropertyValue::Unsigned(r.property_identifier as u64),
        match r.property_array_index {
            Some(idx) => PropertyValue::Unsigned(idx as u64),
            None => PropertyValue::Null,
        },
        match r.device_identifier {
            Some(dev) => PropertyValue::ObjectIdentifier(dev),
            None => PropertyValue::Null,
        },
    ])
}

/// Read the next [`PropertyValue`] at `idx`, advancing it.
pub(super) fn take_pv<'a>(
    items: &'a [PropertyValue],
    idx: &mut usize,
) -> Option<&'a PropertyValue> {
    let v = items.get(*idx)?;
    *idx += 1;
    Some(v)
}

/// Read a `Unsigned` as a `u32`.
pub(super) fn take_u32(items: &[PropertyValue], idx: &mut usize) -> Result<u32, Error> {
    match items.get(*idx) {
        Some(PropertyValue::Unsigned(v)) => {
            *idx += 1;
            u32::try_from(*v).map_err(|_| Error::decoding(*idx, "Unsigned exceeds u32"))
        }
        _ => Err(Error::decoding(*idx, "expected Unsigned")),
    }
}

/// Read a `Unsigned` as a `u16`.
pub(super) fn take_u16(items: &[PropertyValue], idx: &mut usize) -> Result<u16, Error> {
    match items.get(*idx) {
        Some(PropertyValue::Unsigned(v)) => {
            *idx += 1;
            u16::try_from(*v).map_err(|_| Error::decoding(*idx, "Unsigned exceeds u16"))
        }
        _ => Err(Error::decoding(*idx, "expected Unsigned")),
    }
}

/// Read a `Real`/`Double`/`Unsigned` as `f32`.
pub(super) fn take_real(items: &[PropertyValue], idx: &mut usize) -> Result<f32, Error> {
    let v = match items.get(*idx) {
        Some(PropertyValue::Real(v)) => *v,
        Some(PropertyValue::Double(v)) => *v as f32,
        Some(PropertyValue::Unsigned(v)) => *v as f32,
        _ => return Err(Error::decoding(*idx, "expected Real")),
    };
    *idx += 1;
    Ok(v)
}

/// Read a bitstring `(unused_bits, data)` pair.
pub(super) fn take_bitstring(
    items: &[PropertyValue],
    idx: &mut usize,
) -> Result<(u8, Vec<u8>), Error> {
    match items.get(*idx) {
        Some(PropertyValue::BitString { unused_bits, data }) => {
            *idx += 1;
            Ok((*unused_bits, data.clone()))
        }
        _ => Err(Error::decoding(*idx, "expected BitString")),
    }
}

/// Read a `SEQUENCE OF BIT STRING` as a list of `(unused_bits, data)` pairs.
pub(super) fn take_bitstring_list(
    items: &[PropertyValue],
    idx: &mut usize,
) -> Result<Vec<(u8, Vec<u8>)>, Error> {
    match items.get(*idx) {
        Some(PropertyValue::List(inner)) => {
            *idx += 1;
            inner
                .iter()
                .map(|v| take_bitstring(core::slice::from_ref(v), &mut 0))
                .collect()
        }
        _ => Err(Error::decoding(*idx, "expected list of bitstrings")),
    }
}

/// Read a [`BACnetDeviceObjectPropertyReference`].
pub(super) fn take_dopr(
    items: &[PropertyValue],
    idx: &mut usize,
) -> Result<BACnetDeviceObjectPropertyReference, Error> {
    let Some(PropertyValue::List(inner)) = items.get(*idx) else {
        return Err(Error::decoding(
            *idx,
            "expected device object property reference",
        ));
    };
    *idx += 1;
    if inner.len() < 4 {
        return Err(Error::decoding(
            *idx,
            "device object property reference too short",
        ));
    }
    let object_identifier = match &inner[0] {
        PropertyValue::ObjectIdentifier(oid) => *oid,
        _ => return Err(Error::decoding(0, "reference object id missing")),
    };
    let property_identifier = match &inner[1] {
        PropertyValue::Unsigned(v) => *v as u32,
        _ => return Err(Error::decoding(1, "reference property id missing")),
    };
    let property_array_index = match &inner[2] {
        PropertyValue::Unsigned(v) => Some(*v as u32),
        PropertyValue::Null => None,
        _ => return Err(Error::decoding(2, "reference array index invalid")),
    };
    let device_identifier = match &inner[3] {
        PropertyValue::ObjectIdentifier(dev) => Some(*dev),
        PropertyValue::Null => None,
        _ => return Err(Error::decoding(3, "reference device id invalid")),
    };
    Ok(BACnetDeviceObjectPropertyReference {
        object_identifier,
        property_identifier,
        property_array_index,
        device_identifier,
    })
}

/// Read a `SEQUENCE OF BACnetPropertyStates`.
pub(super) fn take_property_state_list(
    items: &[PropertyValue],
    idx: &mut usize,
) -> Result<Vec<BACnetPropertyStates>, Error> {
    let Some(PropertyValue::List(inner)) = items.get(*idx) else {
        return Err(Error::decoding(*idx, "expected list of property states"));
    };
    *idx += 1;
    inner.iter().map(property_state_from_pv).collect()
}

/// Reconstruct a [`BACnetPropertyStates`] from its flat-`List` encoding.
pub(super) fn property_state_from_pv(pv: &PropertyValue) -> Result<BACnetPropertyStates, Error> {
    let PropertyValue::List(inner) = pv else {
        return Err(Error::decoding(0, "property state is not a List"));
    };
    let Some((tag_pv, rest)) = inner.split_first() else {
        return Err(Error::decoding(0, "property state list empty"));
    };
    let PropertyValue::Unsigned(tag) = tag_pv else {
        return Err(Error::decoding(0, "property state tag not Unsigned"));
    };
    let data = match rest.first() {
        Some(PropertyValue::OctetString(b)) => b.clone(),
        _ => return Err(Error::decoding(1, "property state data not octets")),
    };
    let read_u32 = |data: &[u8]| -> Result<u32, Error> {
        data.try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| Error::decoding(1, "property state data wrong length"))
    };
    Ok(match *tag as u8 {
        0 => BACnetPropertyStates::BooleanValue(data.first().copied().unwrap_or(0) != 0),
        1 => BACnetPropertyStates::BinaryValue(read_u32(&data)?),
        2 => BACnetPropertyStates::EventType(read_u32(&data)?),
        3 => BACnetPropertyStates::Polarity(read_u32(&data)?),
        4 => BACnetPropertyStates::ProgramChange(read_u32(&data)?),
        5 => BACnetPropertyStates::ProgramState(read_u32(&data)?),
        6 => BACnetPropertyStates::ReasonForHalt(read_u32(&data)?),
        7 => BACnetPropertyStates::Reliability(read_u32(&data)?),
        8 => BACnetPropertyStates::State(read_u32(&data)?),
        9 => BACnetPropertyStates::SystemStatus(read_u32(&data)?),
        10 => BACnetPropertyStates::Units(read_u32(&data)?),
        11 => BACnetPropertyStates::UnsignedValue(read_u32(&data)?),
        12 => BACnetPropertyStates::LifeSafetyMode(read_u32(&data)?),
        13 => BACnetPropertyStates::LifeSafetyState(read_u32(&data)?),
        14 => BACnetPropertyStates::DoorAlarmState(read_u32(&data)?),
        15 => BACnetPropertyStates::Action(read_u32(&data)?),
        16 => BACnetPropertyStates::DoorSecuredStatus(read_u32(&data)?),
        17 => BACnetPropertyStates::DoorStatus(read_u32(&data)?),
        18 => BACnetPropertyStates::DoorValue(read_u32(&data)?),
        40 => BACnetPropertyStates::LiftCarDirection(read_u32(&data)?),
        42 => BACnetPropertyStates::LiftCarDoorCommand(read_u32(&data)?),
        38 => BACnetPropertyStates::TimerState(read_u32(&data)?),
        39 => BACnetPropertyStates::TimerTransition(read_u32(&data)?),
        other => BACnetPropertyStates::Other { tag: other, data },
    })
}

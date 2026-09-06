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
/// octet string. A trailing Boolean distinguishes current alternatives whose
/// tags had different legacy meanings and marks constructed proprietary values.
pub(super) fn property_state_pv(state: &BACnetPropertyStates) -> PropertyValue {
    let (tag, data) = property_state_parts(state);
    let mut items = vec![
        PropertyValue::Unsigned(tag as u64),
        PropertyValue::OctetString(data),
    ];
    match state {
        BACnetPropertyStates::Other(value) if value.is_constructed() => {
            items.push(PropertyValue::Boolean(true));
        }
        BACnetPropertyStates::Other(_) => {}
        _ if tag >= 14 => {
            items.push(PropertyValue::Boolean(false));
        }
        _ => {}
    }
    PropertyValue::List(items)
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
        BACnetPropertyStates::RestartReason(v) => (14, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorAlarmState(v) => (15, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Action(v) => (16, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorSecuredStatus(v) => (17, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorStatus(v) => (18, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::DoorValue(v) => (19, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::FileAccessMethod(v) => (20, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LockStatus(v) => (21, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LifeSafetyOperation(v) => (22, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::Maintenance(v) => (23, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::NodeType(v) => (24, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::NotifyType(v) => (25, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ShedState(v) => (27, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::SilencedState(v) => (28, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::AccessEvent(v) => (30, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ZoneOccupancyState(v) => (31, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::AccessCredentialDisableReason(v) => (32, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::AccessCredentialDisable(v) => (33, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::AuthenticationStatus(v) => (34, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::BackupState(v) => (36, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::WriteStatus(v) => (37, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LightingInProgress(v) => (38, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LightingOperation(v) => (39, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LightingTransition(v) => (40, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::IntegerValue(v) => (41, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::BinaryLightingValue(v) => (42, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::TimerState(v) => (43, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::TimerTransition(v) => (44, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::BacnetIpMode(v) => (45, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::NetworkPortCommand(v) => (46, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::NetworkType(v) => (47, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::NetworkNumberQuality(v) => (48, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::EscalatorOperationDirection(v) => (49, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::EscalatorFault(v) => (50, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::EscalatorMode(v) => (51, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarDirection(v) => (52, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarDoorCommand(v) => (53, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarDriveStatus(v) => (54, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftCarMode(v) => (55, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftGroupMode(v) => (56, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::LiftFault(v) => (57, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ProtocolLevel(v) => (58, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::AuditLevel(v) => (59, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::AuditOperation(v) => (60, v.to_le_bytes().to_vec()),
        BACnetPropertyStates::ExtendedValue(v) => (63, v.encoded().to_le_bytes().to_vec()),
        BACnetPropertyStates::Other(v) => (v.tag(), v.data().to_vec()),
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
    if inner.len() != 4 {
        return Err(Error::decoding(
            *idx,
            "device object property reference must contain exactly four values",
        ));
    }
    let object_identifier = match &inner[0] {
        PropertyValue::ObjectIdentifier(oid) => *oid,
        _ => return Err(Error::decoding(0, "reference object id missing")),
    };
    let property_identifier = match &inner[1] {
        PropertyValue::Unsigned(v) => u32::try_from(*v)
            .map_err(|_| Error::decoding(1, "reference property id exceeds u32"))?,
        _ => return Err(Error::decoding(1, "reference property id missing")),
    };
    let property_array_index = match &inner[2] {
        PropertyValue::Unsigned(v) => Some(
            u32::try_from(*v)
                .map_err(|_| Error::decoding(2, "reference array index exceeds u32"))?,
        ),
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
    let (data, marker) = match rest {
        [PropertyValue::OctetString(data)] => (data.clone(), None),
        [PropertyValue::OctetString(data), PropertyValue::Boolean(marker)] => {
            (data.clone(), Some(*marker))
        }
        _ => return Err(Error::decoding(1, "property state data not octets")),
    };
    let tag =
        u8::try_from(*tag).map_err(|_| Error::decoding(0, "property state tag exceeds u8"))?;
    let constructed = marker == Some(true);
    let typed_marker = marker == Some(false);
    let old_typed_tag = matches!(tag, 0..=18 | 38..=40 | 42);
    if typed_marker && !(14..64).contains(&tag) {
        return Err(Error::decoding(1, "unexpected typed property state marker"));
    }
    if constructed && !(64..=254).contains(&tag) {
        return Err(Error::decoding(
            0,
            "constructed property state requires a proprietary tag",
        ));
    }
    let legacy_wire = marker.is_none() && !old_typed_tag && tag < 64;
    let read_u32 = |data: &[u8]| -> Result<u32, Error> {
        if legacy_wire {
            if data.is_empty() || data.len() > 4 {
                return Err(Error::decoding(1, "property state data wrong length"));
            }
            return Ok(data
                .iter()
                .fold(0u32, |value, octet| (value << 8) | u32::from(*octet)));
        }
        data.try_into()
            .map(u32::from_le_bytes)
            .map_err(|_| Error::decoding(1, "property state data wrong length"))
    };
    let read_i32 = |data: &[u8]| -> Result<i32, Error> {
        if legacy_wire {
            if data.is_empty()
                || data.len() > 4
                || (data.len() > 1
                    && ((data[0] == 0 && data[1] & 0x80 == 0)
                        || (data[0] == 0xFF && data[1] & 0x80 != 0)))
            {
                return Err(Error::decoding(
                    1,
                    "property state signed data is not canonical",
                ));
            }
            let mut bytes = [if data[0] & 0x80 == 0 { 0 } else { 0xFF }; 4];
            bytes[4 - data.len()..].copy_from_slice(data);
            return Ok(i32::from_be_bytes(bytes));
        }
        data.try_into()
            .map(i32::from_le_bytes)
            .map_err(|_| Error::decoding(1, "property state data wrong length"))
    };
    if marker.is_none() {
        let legacy = match tag {
            14 => Some(BACnetPropertyStates::DoorAlarmState(read_u32(&data)?)),
            15 => Some(BACnetPropertyStates::Action(read_u32(&data)?)),
            16 => Some(BACnetPropertyStates::DoorSecuredStatus(read_u32(&data)?)),
            17 => Some(BACnetPropertyStates::DoorStatus(read_u32(&data)?)),
            18 => Some(BACnetPropertyStates::DoorValue(read_u32(&data)?)),
            38 => Some(BACnetPropertyStates::TimerState(read_u32(&data)?)),
            39 => Some(BACnetPropertyStates::TimerTransition(read_u32(&data)?)),
            40 => Some(BACnetPropertyStates::LiftCarDirection(read_u32(&data)?)),
            42 => Some(BACnetPropertyStates::LiftCarDoorCommand(read_u32(&data)?)),
            _ => None,
        };
        if let Some(state) = legacy {
            return Ok(state);
        }
    }
    Ok(match tag {
        0 => match data.as_slice() {
            [0] => BACnetPropertyStates::BooleanValue(false),
            [1] => BACnetPropertyStates::BooleanValue(true),
            _ => return Err(Error::decoding(1, "property state Boolean must be 0 or 1")),
        },
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
        14 => BACnetPropertyStates::RestartReason(read_u32(&data)?),
        15 => BACnetPropertyStates::DoorAlarmState(read_u32(&data)?),
        16 => BACnetPropertyStates::Action(read_u32(&data)?),
        17 => BACnetPropertyStates::DoorSecuredStatus(read_u32(&data)?),
        18 => BACnetPropertyStates::DoorStatus(read_u32(&data)?),
        19 => BACnetPropertyStates::DoorValue(read_u32(&data)?),
        20 => BACnetPropertyStates::FileAccessMethod(read_u32(&data)?),
        21 => BACnetPropertyStates::LockStatus(read_u32(&data)?),
        22 => BACnetPropertyStates::LifeSafetyOperation(read_u32(&data)?),
        23 => BACnetPropertyStates::Maintenance(read_u32(&data)?),
        24 => BACnetPropertyStates::NodeType(read_u32(&data)?),
        25 => BACnetPropertyStates::NotifyType(read_u32(&data)?),
        27 => BACnetPropertyStates::ShedState(read_u32(&data)?),
        28 => BACnetPropertyStates::SilencedState(read_u32(&data)?),
        30 => BACnetPropertyStates::AccessEvent(read_u32(&data)?),
        31 => BACnetPropertyStates::ZoneOccupancyState(read_u32(&data)?),
        32 => BACnetPropertyStates::AccessCredentialDisableReason(read_u32(&data)?),
        33 => BACnetPropertyStates::AccessCredentialDisable(read_u32(&data)?),
        34 => BACnetPropertyStates::AuthenticationStatus(read_u32(&data)?),
        36 => BACnetPropertyStates::BackupState(read_u32(&data)?),
        37 => BACnetPropertyStates::WriteStatus(read_u32(&data)?),
        38 => BACnetPropertyStates::LightingInProgress(read_u32(&data)?),
        39 => BACnetPropertyStates::LightingOperation(read_u32(&data)?),
        40 => BACnetPropertyStates::LightingTransition(read_u32(&data)?),
        41 => BACnetPropertyStates::IntegerValue(read_i32(&data)?),
        42 => BACnetPropertyStates::BinaryLightingValue(read_u32(&data)?),
        43 => BACnetPropertyStates::TimerState(read_u32(&data)?),
        44 => BACnetPropertyStates::TimerTransition(read_u32(&data)?),
        45 => BACnetPropertyStates::BacnetIpMode(read_u32(&data)?),
        46 => BACnetPropertyStates::NetworkPortCommand(read_u32(&data)?),
        47 => BACnetPropertyStates::NetworkType(read_u32(&data)?),
        48 => BACnetPropertyStates::NetworkNumberQuality(read_u32(&data)?),
        49 => BACnetPropertyStates::EscalatorOperationDirection(read_u32(&data)?),
        50 => BACnetPropertyStates::EscalatorFault(read_u32(&data)?),
        51 => BACnetPropertyStates::EscalatorMode(read_u32(&data)?),
        52 => BACnetPropertyStates::LiftCarDirection(read_u32(&data)?),
        53 => BACnetPropertyStates::LiftCarDoorCommand(read_u32(&data)?),
        54 => BACnetPropertyStates::LiftCarDriveStatus(read_u32(&data)?),
        55 => BACnetPropertyStates::LiftCarMode(read_u32(&data)?),
        56 => BACnetPropertyStates::LiftGroupMode(read_u32(&data)?),
        57 => BACnetPropertyStates::LiftFault(read_u32(&data)?),
        58 => BACnetPropertyStates::ProtocolLevel(read_u32(&data)?),
        59 => BACnetPropertyStates::AuditLevel(read_u32(&data)?),
        60 => BACnetPropertyStates::AuditOperation(read_u32(&data)?),
        63 => BACnetPropertyStates::ExtendedValue(BACnetExtendedPropertyState::from_encoded(
            read_u32(&data)?,
        )?),
        other @ 64..=254 if constructed => {
            BACnetPropertyStates::Other(BACnetProprietaryPropertyState::constructed(other, data)?)
        }
        other @ 64..=254 => {
            BACnetPropertyStates::Other(BACnetProprietaryPropertyState::primitive(other, data)?)
        }
        reserved => {
            return Err(Error::decoding(
                0,
                format!("property state tag {reserved} is reserved"),
            ));
        }
    })
}

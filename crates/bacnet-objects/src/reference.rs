//! Shared write-arm decode of the `BACnetObjectPropertyReference`-typed
//! properties (Clause 21 production): Loop `Controlled_Variable_Reference`,
//! `Manipulated_Variable_Reference`, `Setpoint_Reference` (Clause 12.17, the
//! last as the optional member of `BACnetSetpointReference`) and Pulse
//! Converter `Input_Reference` (Clause 12.23).
//!
//! Three input shapes are accepted, and nothing else:
//!
//! 1. **`Null`** — clears the reference.
//! 2. **Legacy local `List`** — `[ObjectIdentifier, Enumerated]` plus an
//!    optional third `Unsigned` carrying the array index. This is the
//!    in-process form (object read arms and the pre-#182 local writes), and
//!    it is exactly what the service decode now produces when a peer writes
//!    the flattened application-tagged form this stack emits on reads. The
//!    shape is exact: extra or wrong-typed members are refused, never
//!    silently ignored.
//! 3. **Framed network form** — the reference's primitive context-tagged
//!    members [0]/[1]/[2] verbatim: one or more `ApplicationData` elements
//!    (the service decode splits at context-tag boundaries, one element per
//!    member; a `Setpoint_Reference` write wrapped in the
//!    `BACnetSetpointReference` opening/closing tag 0 arrives as a single
//!    element). Concatenation is strictly decoded by the Clause 21 codec in
//!    `bacnet-encoding`, which rejects a device-qualifying member [3] (not
//!    part of this production — these references are local-device only) and
//!    any unknown trailing context tag.
//!
//! Error pairings follow Clause 15.9.1.3 and the object's existing arms: a
//! value of the wrong BACnet datatype is PROPERTY / INVALID_DATA_TYPE, and a
//! framed form whose encoding is not valid for the production is PROPERTY /
//! INVALID_DATA_ENCODING.

use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;

/// Which wire frame a reference-typed property accepts on top of the bare
/// member sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceFrame {
    /// Bare members only (Loop Controlled_/Manipulated_Variable_Reference,
    /// Pulse Converter Input_Reference — each declared
    /// `BACnetObjectPropertyReference`).
    Bare,
    /// Also accept the `BACnetSetpointReference` opening/closing tag 0 frame
    /// (Loop Setpoint_Reference's Clause 21 production).
    Setpoint,
}

/// Build the local (flat `List`) read form of a reference property: object
/// id and property as `[ObjectIdentifier, Enumerated]`, plus a third
/// `Unsigned` when the reference is indexed; an absent reference reads back
/// as `Null`.
pub(crate) fn reference_read_value(
    reference: &Option<BACnetObjectPropertyReference>,
) -> PropertyValue {
    match reference {
        Some(r) => {
            let mut items = vec![
                PropertyValue::ObjectIdentifier(r.object_identifier),
                PropertyValue::Enumerated(r.property_identifier),
            ];
            if let Some(index) = r.property_array_index {
                items.push(PropertyValue::Unsigned(index as u64));
            }
            PropertyValue::List(items)
        }
        None => PropertyValue::Null,
    }
}

/// Decode a `BACnetObjectPropertyReference`-typed write value into
/// `Some(reference)`; `Null` decodes to `None` (clear).
///
/// Both the legacy local `List` form and the framed network form are
/// accepted — see the module documentation for the exact shapes.
pub(crate) fn decode_reference_write(
    value: &PropertyValue,
    frame: ReferenceFrame,
) -> Result<Option<BACnetObjectPropertyReference>, Error> {
    match value {
        PropertyValue::Null => Ok(None),
        PropertyValue::ApplicationData(bytes) => decode_framed(bytes, frame),
        PropertyValue::List(items) => match items.first() {
            Some(PropertyValue::ObjectIdentifier(_)) => decode_legacy_list(items).map(Some),
            Some(PropertyValue::ApplicationData(_)) => {
                // Mixed framed/flat element lists are a framing-level
                // nonsensical mixture: refuse, never pick one interpretation.
                if !items
                    .iter()
                    .all(|item| matches!(item, PropertyValue::ApplicationData(_)))
                {
                    return Err(common::invalid_data_encoding_error());
                }
                let mut bytes = Vec::new();
                for item in items {
                    if let PropertyValue::ApplicationData(part) = item {
                        bytes.extend_from_slice(part);
                    }
                }
                decode_framed(&bytes, frame)
            }
            _ => Err(common::invalid_data_type_error()),
        },
        _ => Err(common::invalid_data_type_error()),
    }
}

/// The legacy local form: `[ObjectIdentifier, property-id]` with an
/// optional third `Unsigned` array index — exactly two or three members.
///
/// Member typing: the property member travels as `Enumerated` in the
/// Loop/Pulse Converter flat form and as `Unsigned` in the Averaging flat
/// form; both are accepted on write for cross-object compatibility (this
/// mirrors Clause 21 where `BACnetPropertyIdentifier` is itself an
/// ENUMERATED production, but the two flat conventions predate the framed
/// codec and each object's read keeps its historical emission). Values past
/// u32 (a >4-octet wire member, or an overflowing `Unsigned`) are refused.
fn decode_legacy_list(items: &[PropertyValue]) -> Result<BACnetObjectPropertyReference, Error> {
    let Some(PropertyValue::ObjectIdentifier(object_identifier)) = items.first() else {
        return Err(common::invalid_data_type_error());
    };
    let property_identifier = match items.get(1) {
        Some(PropertyValue::Enumerated(property)) => *property,
        Some(PropertyValue::Unsigned(property)) => {
            u32::try_from(*property).map_err(|_| common::invalid_data_type_error())?
        }
        _ => return Err(common::invalid_data_type_error()),
    };
    let property_array_index = match items.get(2) {
        None => None,
        Some(PropertyValue::Unsigned(index)) => {
            Some(u32::try_from(*index).map_err(|_| common::invalid_data_type_error())?)
        }
        Some(_) => return Err(common::invalid_data_type_error()),
    };
    if items.len() > 3 {
        return Err(common::invalid_data_type_error());
    }
    Ok(BACnetObjectPropertyReference {
        object_identifier: *object_identifier,
        property_identifier,
        property_array_index,
    })
}

/// Strict framed decode; every codec failure is INVALID_DATA_ENCODING.
///
/// `Setpoint` accepts the `BACnetSetpointReference` [0]-framed production as
/// well as the bare member sequence: the two are unambiguous (a bare
/// reference always opens with *primitive* context tag [0], the frame with
/// *opening* tag [0]), and the bare form is what a peer handling the
/// reference generically — and this stack's own test tooling — may send. The
/// [0] frame with NO inner members is the production's absent-alternative
/// (the member is OPTIONAL; Clause 12.17 Setpoint_Reference: "The absence of
/// a reference indicates that the setpoint for this control loop is fixed
/// and is contained in the Setpoint property") and clears, exactly like a
/// `Null` write.
fn decode_framed(
    bytes: &[u8],
    frame: ReferenceFrame,
) -> Result<Option<BACnetObjectPropertyReference>, Error> {
    let bare = bacnet_encoding::constructed::decode_object_property_reference(bytes);
    match (bare, frame) {
        (Ok(reference), _) => Ok(Some(reference)),
        (Err(_), ReferenceFrame::Setpoint) => {
            match bacnet_encoding::constructed::decode_setpoint_reference(bytes) {
                Ok(reference) => Ok(reference),
                Err(_) => Err(common::invalid_data_encoding_error()),
            }
        }
        (Err(_), ReferenceFrame::Bare) => Err(common::invalid_data_encoding_error()),
    }
}

#[cfg(test)]
mod tests;

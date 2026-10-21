//! `BACnetPropertyReference` and `ReadAccessSpecification` (Clause 21): the
//! object and properties a ReadPropertyMultiple request names, and the
//! element of a Group object's List_Of_Group_Members (Clause 12.14.5).

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::AuditPropertyReference;
use crate::enums::PropertyIdentifier;
use crate::error::Error;
use crate::primitives::ObjectIdentifier;

/// One property of an object, whole or one array element
/// (`BACnetPropertyReference`, Clause 21).
///
/// On the wire the property identifier is context tag `[0]` and the optional
/// Unsigned array index `[1]`. The codec is
/// `bacnet_encoding::constructed::{encode_property_reference,
/// decode_property_reference}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyReference {
    /// Property being referred to.
    pub property_identifier: PropertyIdentifier,
    /// Array element index; `None` refers to the whole property.
    pub property_array_index: Option<u32>,
}

/// An object and the properties of it to read (`ReadAccessSpecification`,
/// Clause 21).
///
/// On the wire the object identifier is context tag `[0]`, followed by the
/// property references back to back inside an opening/closing tag pair
/// `[1]`. The codec is
/// `bacnet_encoding::constructed::{encode_read_access_specification,
/// decode_read_access_specification}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAccessSpecification {
    /// Object to read from.
    pub object_identifier: ObjectIdentifier,
    /// Properties to read from that object; may name the special ALL, REQUIRED or OPTIONAL
    /// selectors.
    pub list_of_property_references: Vec<PropertyReference>,
}

/// [`PropertyReference`] narrows the optional array index to `u32`, while
/// Clause 21 leaves it an unconstrained Unsigned, so the Audit form keeps
/// every value the primitive layer supports and converting back can fail.
impl From<PropertyReference> for AuditPropertyReference {
    fn from(value: PropertyReference) -> Self {
        Self {
            property_identifier: value.property_identifier,
            property_array_index: value.property_array_index.map(u64::from),
        }
    }
}

impl TryFrom<AuditPropertyReference> for PropertyReference {
    type Error = Error;

    fn try_from(value: AuditPropertyReference) -> Result<Self, Self::Error> {
        Ok(Self {
            property_identifier: value.property_identifier,
            property_array_index: value
                .property_array_index
                .map(u32::try_from)
                .transpose()
                .map_err(|_| {
                    Error::range(
                        "Audit property-array-index exceeds shared PropertyReference u32 limit",
                    )
                })?,
        })
    }
}

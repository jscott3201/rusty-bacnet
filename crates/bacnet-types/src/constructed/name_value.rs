//! `BACnetNameValue` (Clause 21): one tag of an object's Tags array (#1553).

#[cfg(not(feature = "std"))]
use alloc::string::String;

use crate::primitives::{Date, PropertyValue, Time};

/// `BACnetNameValue` (Clause 21): a tag's name and, for a value tag, its
/// value. A semantic tag has a name alone (Annex Y.1.4).
///
/// On the wire it is a SEQUENCE with no frame of its own: the name as a
/// CharacterString under primitive context tag `[0]`, then the value, when
/// there is one, application-tagged as its own datatype. The `bacnet-encoding`
/// crate owns the codec.
///
/// ```
/// use bacnet_types::constructed::{BACnetNameValue, TagValue};
/// use bacnet_types::primitives::PropertyValue;
///
/// let semantic = BACnetNameValue::semantic("exhaust");
/// let valued = BACnetNameValue::valued("floor", TagValue::Primitive(PropertyValue::Unsigned(3)));
/// assert!(semantic.value.is_none() && valued.value.is_some());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetNameValue {
    /// Context tag 0: the tag's name.
    pub name: String,
    /// The tag's value; `None` for a semantic tag.
    pub value: Option<TagValue>,
}

impl BACnetNameValue {
    /// A semantic tag: a name with no value.
    pub fn semantic(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: None,
        }
    }

    /// A value tag: a name and its value.
    pub fn valued(name: impl Into<String>, value: TagValue) -> Self {
        Self {
            name: name.into(),
            value: Some(value),
        }
    }
}

/// The value of a [`BACnetNameValue`]: Clause 21 limits it to a primitive
/// datatype or a BACnetDateTime.
#[derive(Debug, Clone, PartialEq)]
pub enum TagValue {
    /// A value of a primitive datatype: a [`PropertyValue`] whose
    /// [`is_primitive`](PropertyValue::is_primitive) holds. The codec refuses
    /// any other.
    Primitive(PropertyValue),
    /// A BACnetDateTime: an application Date, then an application Time.
    DateTime {
        /// The calendar date.
        date: Date,
        /// The time of day.
        time: Time,
    },
}

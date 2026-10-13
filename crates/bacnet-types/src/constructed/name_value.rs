//! `BACnetNameValue` (Clause 21): one tag of an object's Tags array (#1553).

#[cfg(not(feature = "std"))]
use alloc::string::String;

use crate::primitives::PropertyValue;

/// `BACnetNameValue` (Clause 21): a tag's name and, for a value tag, its
/// value. A semantic tag has a name alone (Annex Y.1.4).
///
/// On the wire it is a SEQUENCE with no frame of its own: the name as a
/// CharacterString under primitive context tag `[0]`, then the value, when
/// there is one, application-tagged as its own datatype. The `bacnet-encoding`
/// crate owns the codec. The 2024-04-29 errata, item 37, restricts the
/// optional value to one primitive; Date and Time are allowed separately.
///
/// ```
/// use bacnet_types::constructed::BACnetNameValue;
/// use bacnet_types::primitives::PropertyValue;
///
/// let semantic = BACnetNameValue::semantic("exhaust");
/// let valued = BACnetNameValue::valued("floor", PropertyValue::Unsigned(3));
/// assert!(semantic.value.is_none() && valued.value.is_some());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetNameValue {
    /// Context tag 0: the tag's name.
    pub name: String,
    /// One primitive value; `None` for a semantic tag. The codec and object
    /// profile validator reject `List` and `ApplicationData` values.
    pub value: Option<PropertyValue>,
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
    pub fn valued(name: impl Into<String>, value: PropertyValue) -> Self {
        Self {
            name: name.into(),
            value: Some(value),
        }
    }
}

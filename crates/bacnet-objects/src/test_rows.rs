//! Property rows for the custom objects tests build (#2181).
//!
//! `BACnetObject::property_metadata` has no default, so each test object
//! names what it serves. The rows here hold the universal properties
//! (Object_Identifier, Object_Name, Object_Type and Property_List) plus the
//! ones a test object lists, each required. bacnet-server keeps the same
//! helper for its own tests.

use std::borrow::Cow;

use crate::property_metadata::{PropertyConformance, PropertyMetadata, PropertyWriteCapability};
use bacnet_types::enums::PropertyIdentifier as P;

const UNIVERSAL: [P; 3] = [P::OBJECT_IDENTIFIER, P::OBJECT_NAME, P::OBJECT_TYPE];

/// The universal rows, then `served` in order, then Property_List. A
/// property in `writable` gets a write route; every other row is read-only.
pub(crate) fn rows(served: &[P], writable: &[P]) -> Cow<'static, [PropertyMetadata]> {
    debug_assert!(
        writable
            .iter()
            .all(|property| served.contains(property) || UNIVERSAL.contains(property)),
        "a writable row must be served: {writable:?} against {served:?}"
    );
    let row = |property: P| {
        let (conformance, write) = if writable.contains(&property) {
            (
                PropertyConformance::RequiredWrite,
                PropertyWriteCapability::Always,
            )
        } else {
            (
                PropertyConformance::RequiredRead,
                PropertyWriteCapability::ReadOnly,
            )
        };
        PropertyMetadata::new(property, conformance, None, write)
    };
    let mut properties: Vec<P> = UNIVERSAL.to_vec();
    for &property in served {
        if property != P::PROPERTY_LIST && !properties.contains(&property) {
            properties.push(property);
        }
    }
    properties.push(P::PROPERTY_LIST);
    Cow::Owned(properties.into_iter().map(row).collect())
}

/// [`rows`] with no write route.
pub(crate) fn read_only(served: &[P]) -> Cow<'static, [PropertyMetadata]> {
    rows(served, &[])
}

/// The Property_List `rows` serve, collected for an assertion.
pub(crate) fn listed(rows: &[PropertyMetadata]) -> Vec<P> {
    crate::property_metadata::property_list(rows).collect()
}

/// The required properties `rows` make, collected for an assertion.
pub(crate) fn required(rows: &[PropertyMetadata]) -> Vec<P> {
    crate::property_metadata::required_properties(rows).collect()
}

/// `properties` without the four Property_List leaves out, for comparing an
/// expected set that names the identity properties with [`listed`].
pub(crate) fn listed_from(properties: &[P]) -> Vec<P> {
    properties
        .iter()
        .copied()
        .filter(|&property| crate::property_metadata::in_property_list(property))
        .collect()
}

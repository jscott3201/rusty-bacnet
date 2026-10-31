//! Property rows for the custom objects tests build (#2181).
//!
//! `BACnetObject::property_metadata` has no default, so each test object
//! names what it serves. The rows here hold the universal properties
//! (Object_Identifier, Object_Name, Object_Type and Property_List) plus the
//! ones a test object lists, each required. bacnet-endpoint's tests include
//! this file too, so it names its crates in full; bacnet-objects keeps the
//! same helper for its own tests.

use std::borrow::Cow;

use bacnet_objects::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyWriteCapability,
};
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

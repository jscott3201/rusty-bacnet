//! `ResolvedEnum::enumeration`: the names a property's enumeration offers.

use super::super::*;

#[test]
fn a_resolved_enum_lists_its_enumeration() {
    let (name, values) = ResolvedEnum::from_property(PropertyIdentifier::FILE_ACCESS_METHOD, 7)
        .enumeration()
        .unwrap();
    assert_eq!(name, "FileAccessMethod");
    assert_eq!(values, [("RECORD_ACCESS", 0), ("STREAM_ACCESS", 1)]);
    // The one u8-backed enumeration widens its values.
    let (name, values) = ResolvedEnum::from_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, 0)
        .enumeration()
        .unwrap();
    assert_eq!(name, "Segmentation");
    assert!(values.contains(&("NONE", 3)));
    assert_eq!(ResolvedEnum::Unknown(4).enumeration(), None);
}

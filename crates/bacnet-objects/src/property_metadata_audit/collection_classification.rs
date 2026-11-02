//! Every array the built-in objects serve answers index 0 with its size.
//! Their array and list classification is the standard value type table's,
//! which `value_type_equivalence` holds to the pre-#2177 classification.

use super::*;
use crate::property_metadata::standard_value_type;
use bacnet_types::error::RemoteError;

fn standard_array_property(object_type: ObjectType, property: PropertyIdentifier) -> bool {
    standard_value_type(object_type, property).is_some_and(|v| v.is_array())
}

#[test]
fn every_served_array_reads_its_size_at_index_zero() {
    let mut objects = supported_representatives();
    objects.push(Box::new(
        NetworkPortObject::new_bip(2, "NP-BIP", Default::default()).unwrap(),
    ));
    objects.push(Box::new(
        NetworkPortObject::new_bip6(3, "NP-BIP6", Default::default()).unwrap(),
    ));
    let mut failures = Vec::new();
    for object in &objects {
        let object_type = object.object_identifier().object_type();
        for &property in crate::test_rows::listed(&object.property_metadata()).iter() {
            if !standard_array_property(object_type, property) {
                continue;
            }
            match object.read_property(property, Some(0)) {
                Ok(PropertyValue::Unsigned(_)) => {}
                // A row the representative serves only once configured.
                Err(Error::Remote(RemoteError::Error { .. }))
                    if object.read_property(property, None).is_err() => {}
                other => failures.push(format!("{object_type} {property}[0]: {other:?}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

//! Every built-in object answers the array and list queries from the shared
//! classification table, so a client that knows only the object type and
//! property (`standard_array_property`, `standard_list_property`) shapes a
//! value the way the object serves it (#1296).

use super::*;
use crate::traits::{standard_array_property, standard_list_property};
use bacnet_types::error::RemoteError;

#[test]
fn every_served_property_follows_the_shared_collection_table() {
    let mut objects = supported_representatives();
    audit_object_type_coverage(&objects);
    // The representatives build a non-B/IP Network Port; a B/IP one serves
    // IP_DNS_SERVER as well, and a B/IPv6 one IPV6_DNS_SERVER.
    objects.push(Box::new(
        NetworkPortObject::new_bip(2, "NP-BIP", Default::default()).unwrap(),
    ));
    objects.push(Box::new(
        NetworkPortObject::new_bip6(3, "NP-BIP6", Default::default()).unwrap(),
    ));
    let mut failures = Vec::new();
    for object in &objects {
        let object_type = object.object_identifier().object_type();
        for &property in object.property_list().iter() {
            let array = standard_array_property(object_type, property);
            let list = standard_list_property(object_type, property);
            if object.is_array_property(property) != array
                || object.is_list_property(property) != list
            {
                failures.push(format!(
                    "{object_type} {property}: object array={} list={}, table array={array} list={list}",
                    object.is_array_property(property),
                    object.is_list_property(property),
                ));
            }
            assert!(!(array && list), "{object_type} {property} is both");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
        for &property in object.property_list().iter() {
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

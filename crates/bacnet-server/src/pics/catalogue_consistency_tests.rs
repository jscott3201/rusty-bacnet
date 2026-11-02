//! One PICS over the whole catalogue, as a running server generates it
//! (#2175).
//!
//! The PICS lists exactly the types the Device advertises, one per
//! catalogue type, and each type's row says what the object and the
//! server's type registry do: every property row, read-only or writable
//! as its write capability says, required or optional as its conformance
//! says, written through the property its capability names; createable,
//! deleteable and its creation-only properties as the registry answers.
//! That each write capability matches the object's write arm is
//! `bacnet_objects`' `write_capability_tests`.

use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject, SUPPORTED_OBJECT_TYPES};

const DEVICE_INSTANCE: u32 = 2175;

/// The catalogue, its Device configured as the server's own.
fn catalogue_database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(DEVICE_INSTANCE, 0)
                .unwrap()
                .with_name("Catalogue device"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    for object in bacnet_objects::catalogue::representatives() {
        if object.object_identifier().object_type() != ObjectType::DEVICE {
            db.add(object).unwrap();
        }
    }
    db
}

/// The PICS row `object`'s metadata rows give, in property order.
fn expected_properties(object: &dyn BACnetObject) -> Vec<String> {
    let mut rows: Vec<_> = object.property_metadata().iter().copied().collect();
    rows.sort_by_key(|row| row.property_identifier.to_raw());
    rows.iter()
        .map(|row| {
            format!(
                "{:?} R{} {} {:?}",
                row.property_identifier,
                if row.write_capability.is_writable() {
                    "W"
                } else {
                    ""
                },
                if row.is_required() {
                    "required"
                } else {
                    "optional"
                },
                row.write_capability.written_through(),
            )
        })
        .collect()
}

fn listed_properties(support: &ObjectTypeSupport) -> Vec<String> {
    support
        .supported_properties
        .iter()
        .map(|property| {
            assert!(property.access.readable, "{:?}", property.property_id);
            format!(
                "{:?} R{} {} {:?}",
                property.property_id,
                if property.access.writable { "W" } else { "" },
                if property.access.optional {
                    "optional"
                } else {
                    "required"
                },
                property.written_through,
            )
        })
        .collect()
}

#[test]
fn the_catalogue_pics_follows_each_types_rows_and_the_registry() {
    let db = catalogue_database();
    let server_config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::new(DEVICE_INSTANCE, 0).unwrap(),
        ..ServerConfig::default()
    };
    let pics_config = PicsConfig::default();
    let pics = PicsGenerator::new(&db, &server_config, &pics_config)
        .for_server()
        .generate();
    let registry = &server_config.object_types;

    let types: Vec<ObjectType> = pics
        .supported_object_types
        .iter()
        .map(|support| support.object_type)
        .collect();
    assert_eq!(types, SUPPORTED_OBJECT_TYPES, "one row per advertised type");

    let mut failures = Vec::new();
    for support in &pics.supported_object_types {
        let kind = support.object_type;
        let instances = db.find_by_type(kind);
        assert_eq!(instances.len(), 1, "{kind:?}: one catalogue object");
        let object = db.get(&instances[0]).unwrap();
        if listed_properties(support) != expected_properties(object) {
            failures.push(format!(
                "{kind:?}: PICS {:?}\n  rows {:?}",
                listed_properties(support),
                expected_properties(object)
            ));
        }
        if support.createable != registry.creates(kind) {
            failures.push(format!("{kind:?}: createable {}", support.createable));
        }
        if support.deleteable != registry.deletes(kind) {
            failures.push(format!("{kind:?}: deleteable {}", support.deleteable));
        }
        // Creation-only properties: the registry's, but a property written
        // through another, for a createable type; none otherwise. Each is a
        // property the object has.
        let rows = object.property_metadata();
        let expected: Vec<PropertyIdentifier> = if support.createable {
            registry
                .creation_only(kind)
                .properties()
                .iter()
                .copied()
                .filter(|property| {
                    !rows.iter().any(|row| {
                        row.property_identifier == *property
                            && row.write_capability.written_through().is_some()
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        if support.creation_only_properties != expected {
            failures.push(format!(
                "{kind:?}: creation-only {:?}, expected {expected:?}",
                support.creation_only_properties
            ));
        }
        for property in &support.creation_only_properties {
            if !bacnet_objects::property_metadata::has_property(&rows, *property) {
                failures.push(format!("{kind:?}: creation-only {property:?} has no row"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let createable = pics
        .supported_object_types
        .iter()
        .filter(|support| support.createable)
        .count();
    assert_eq!(createable, 27, "the built-in table's createable types");
}

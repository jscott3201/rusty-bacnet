//! Registering CreateObject constructors (#2173): which types a registry
//! takes, and the PICS reporting a registered type as createable.

use super::*;
use crate::pics::{PicsConfig, generate_pics};
use bacnet_objects::property_metadata::PropertyMetadata;
use std::borrow::Cow;

/// The proprietary type the tests register.
pub(crate) const VENDOR_TYPE: u32 = 600;

pub(crate) fn vendor_type() -> ObjectType {
    ObjectType::from_raw(VENDOR_TYPE)
}

/// An application's own object of a proprietary type, with a writable
/// Unsigned Present_Value.
pub(crate) struct Vendor {
    pub oid: ObjectIdentifier,
    pub name: String,
    pub value: u64,
}

impl Vendor {
    pub(crate) fn boxed(oid: ObjectIdentifier, name: &str) -> Box<dyn BACnetObject> {
        Box::new(Self {
            oid,
            name: name.to_owned(),
            value: 0,
        })
    }
}

impl BACnetObject for Vendor {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(VENDOR_TYPE)),
            PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::Unsigned(self.value)),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _: Option<u32>,
        value: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        match (property, value) {
            (PropertyIdentifier::OBJECT_NAME, PropertyValue::CharacterString(name)) => {
                self.name = name;
                Ok(())
            }
            (PropertyIdentifier::PRESENT_VALUE, PropertyValue::Unsigned(value)) => {
                self.value = value;
                Ok(())
            }
            (PropertyIdentifier::OBJECT_NAME | PropertyIdentifier::PRESENT_VALUE, _) => Err(
                Error::protocol(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE),
            ),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::rows(
            &[PropertyIdentifier::PRESENT_VALUE],
            &[
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::PRESENT_VALUE,
            ],
        )
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        bacnet_objects::property_metadata::property_list_from_metadata(&self.property_metadata())
    }
}

fn vendor(oid: ObjectIdentifier, name: &str) -> Result<Box<dyn BACnetObject>, Error> {
    Ok(Vendor::boxed(oid, name))
}

#[test]
fn a_type_registers_once() {
    let mut constructors = ObjectConstructors::new();
    constructors.register(vendor_type(), vendor).unwrap();
    assert!(constructors.contains(vendor_type()));
    let again = constructors.register(vendor_type(), |_, _| {
        panic!("a refused registration is never called")
    });
    assert!(matches!(again, Err(Error::Encoding(_))), "{again:?}");
    // A built-in type registers once too, and its built-in constructor
    // doesn't count as a registration.
    constructors
        .register(ObjectType::ANALOG_VALUE, vendor)
        .unwrap();
    assert!(
        constructors
            .register(ObjectType::ANALOG_VALUE, vendor)
            .is_err()
    );
    assert_eq!(
        constructors.object_types().collect::<Vec<_>>(),
        [ObjectType::ANALOG_VALUE, vendor_type()]
    );
}

#[test]
fn only_listed_and_proprietary_types_register() {
    let mut constructors = ObjectConstructors::new();
    // Network Security is deleted from the standard, 65 to 127 are reserved
    // standard types the Device doesn't list, and past 1023 no identifier
    // can hold the type.
    for raw in [38, 65, 127, 1024, 5000] {
        assert!(
            constructors
                .register(ObjectType::from_raw(raw), vendor)
                .is_err(),
            "{raw}"
        );
    }
    assert_eq!(constructors.object_types().count(), 0);
    // Every listed type but Device and Network Port, built-in table or not,
    // and the proprietary range.
    for &kind in bacnet_objects::device::SUPPORTED_OBJECT_TYPES {
        if !matches!(kind, ObjectType::DEVICE | ObjectType::NETWORK_PORT) {
            constructors.register(kind, vendor).unwrap();
        }
    }
    for raw in [128, 1023] {
        constructors
            .register(ObjectType::from_raw(raw), vendor)
            .unwrap();
    }
}

#[test]
fn the_builders_refuse_what_the_registry_refuses() {
    let builder = BACnetServer::bip_builder()
        .object_constructor(vendor_type(), vendor)
        .unwrap();
    assert!(builder.object_constructor(vendor_type(), vendor).is_err());
    assert!(
        BACnetServer::<test_transport::TestTransport>::generic_builder()
            .object_constructor(ObjectType::from_raw(64 + 1), vendor)
            .is_err()
    );
}

#[test]
fn device_and_network_port_never_register() {
    let mut constructors = ObjectConstructors::new();
    for kind in [ObjectType::DEVICE, ObjectType::NETWORK_PORT] {
        // Both are listed types; the node itself is what refuses them.
        assert!(bacnet_objects::device::SUPPORTED_OBJECT_TYPES.contains(&kind));
        let refused = constructors.register(kind, |_, _| {
            panic!("a refused registration is never called")
        });
        assert!(matches!(refused, Err(Error::Encoding(_))), "{refused:?}");
        assert!(!constructors.creates(kind), "{kind:?}");
    }
    assert_eq!(constructors.object_types().count(), 0);
}

#[test]
fn creates_answers_registry_or_built_in_table() {
    let mut constructors = ObjectConstructors::new();
    assert!(constructors.creates(ObjectType::ANALOG_VALUE));
    assert!(!constructors.creates(ObjectType::ELEVATOR_GROUP));
    assert!(!constructors.creates(vendor_type()));
    constructors.register(vendor_type(), vendor).unwrap();
    constructors
        .register(ObjectType::ELEVATOR_GROUP, vendor)
        .unwrap();
    assert!(constructors.creates(ObjectType::ELEVATOR_GROUP));
    assert!(constructors.creates(vendor_type()));
}

fn type_rows(db: &ObjectDatabase, config: &ServerConfig) -> Vec<crate::pics::ObjectTypeSupport> {
    generate_pics(db, config, &PicsConfig::default()).supported_object_types
}

fn row(rows: &[crate::pics::ObjectTypeSupport], kind: ObjectType) -> Option<(bool, usize)> {
    rows.iter()
        .find(|support| support.object_type == kind)
        .map(|support| (support.createable, support.supported_properties.len()))
}

#[test]
fn the_pics_lists_a_registered_type_with_no_instance_without_building_one() {
    let db = ObjectDatabase::new();
    let unregistered = type_rows(&db, &ServerConfig::default());
    assert_eq!(row(&unregistered, vendor_type()), None);
    assert_eq!(row(&unregistered, ObjectType::ELEVATOR_GROUP), None);

    let mut registered = ServerConfig::default();
    for kind in [vendor_type(), ObjectType::ELEVATOR_GROUP] {
        registered
            .object_constructors
            .register(kind, |_, _| panic!("the PICS never calls a constructor"))
            .unwrap();
    }
    let rows = type_rows(&db, &registered);
    // Listed, createable, and with no property detail.
    for kind in [vendor_type(), ObjectType::ELEVATOR_GROUP] {
        let support = rows.iter().find(|s| s.object_type == kind).unwrap();
        assert!(support.createable, "{kind:?}");
        assert!(support.supported_properties.is_empty(), "{kind:?}");
        assert!(support.creation_only_properties.is_empty(), "{kind:?}");
    }
}

#[test]
fn the_pics_reports_a_registered_type_with_an_instance_as_createable() {
    let mut db = ObjectDatabase::new();
    db.add(Vendor::boxed(
        ObjectIdentifier::new(vendor_type(), 1).unwrap(),
        "vendor",
    ))
    .unwrap();
    // Without a registration the instance's type is listed, not createable.
    assert_eq!(
        row(&type_rows(&db, &ServerConfig::default()), vendor_type()),
        Some((false, 5))
    );
    let mut registered = ServerConfig::default();
    registered
        .object_constructors
        .register(vendor_type(), vendor)
        .unwrap();
    // The instance still describes the properties.
    assert_eq!(
        row(&type_rows(&db, &registered), vendor_type()),
        Some((true, 5))
    );
}

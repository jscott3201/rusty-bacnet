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
    // Every listed type, built-in table or not, and the proprietary range.
    for &kind in bacnet_objects::device::SUPPORTED_OBJECT_TYPES {
        constructors.register(kind, vendor).unwrap();
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
fn the_pics_reports_a_registered_type_as_createable() {
    let mut db = ObjectDatabase::new();
    db.add(Vendor::boxed(
        ObjectIdentifier::new(vendor_type(), 1).unwrap(),
        "vendor",
    ))
    .unwrap();
    // Calendar has no built-in constructor; registered, it is createable.
    db.add(Box::new(
        bacnet_objects::schedule::CalendarObject::new(1, "cal").unwrap(),
    ))
    .unwrap();
    let createable = |config: &ServerConfig| {
        generate_pics(&db, config, &PicsConfig::default())
            .supported_object_types
            .iter()
            .map(|support| (support.object_type, support.createable))
            .collect::<Vec<_>>()
    };
    let unregistered = ServerConfig::default();
    assert_eq!(
        createable(&unregistered),
        [(ObjectType::CALENDAR, false), (vendor_type(), false)]
    );
    let mut registered = ServerConfig::default();
    registered
        .object_constructors
        .register(vendor_type(), vendor)
        .unwrap();
    registered
        .object_constructors
        .register(ObjectType::CALENDAR, |oid, name| {
            Ok(Box::new(bacnet_objects::schedule::CalendarObject::new(
                oid.instance_number(),
                name,
            )?))
        })
        .unwrap();
    assert_eq!(
        createable(&registered),
        [(ObjectType::CALENDAR, true), (vendor_type(), true)]
    );
    // The custom object answers is_createable with the trait default.
    assert!(
        !db.get(&ObjectIdentifier::new(vendor_type(), 1).unwrap())
            .unwrap()
            .is_createable()
    );
}

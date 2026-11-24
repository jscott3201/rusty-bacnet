//! `check_addable` gives every refusal `add` gives, without taking the
//! object (#2752): a caller that can't rebuild an object checks it first.

use std::borrow::Cow;

use super::*;
use crate::network_port::{BipPortConfig, NetworkPortObject};
use crate::property_metadata::PropertyMetadata;
use bacnet_types::primitives::PropertyValue;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// An object with any identifier and name.
struct Named(ObjectIdentifier, &'static str);

impl BACnetObject for Named {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0
    }
    fn object_name(&self) -> &str {
        self.1
    }
    fn read_property(
        &self,
        _property: bacnet_types::enums::PropertyIdentifier,
        _index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ))
    }
    fn write_property(
        &mut self,
        _property: bacnet_types::enums::PropertyIdentifier,
        _index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
}

/// A database with a value named "Value", a registered Network Port whose
/// owner lives, and an Audit owner protecting Device 10 and Reporter 1.
fn guarded() -> (ObjectDatabase, Arc<()>, Arc<AuditOwnership>) {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(Named(oid(ObjectType::BINARY_VALUE, 1), "Value")))
        .unwrap();
    let port = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        udp_port: 0,
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "Port", port).unwrap(),
    ))
    .unwrap();
    let (_, lease) = db
        .reserve_bip_port_internal(oid(ObjectType::NETWORK_PORT, 1), [127, 0, 0, 1], 0)
        .unwrap();
    let owner = AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 10),
        oid(ObjectType::AUDIT_REPORTER, 1),
    );
    db.protect_audit_internal(&owner).unwrap();
    (db, lease, owner)
}

#[test]
fn the_check_and_add_give_the_same_answer() {
    let cases = [
        (
            "the reserved instance",
            oid(ObjectType::BINARY_VALUE, 4_194_303),
            "Reserved",
            false,
        ),
        (
            "a name another object has",
            oid(ObjectType::BINARY_VALUE, 2),
            "Value",
            false,
        ),
        (
            "the registered port",
            oid(ObjectType::NETWORK_PORT, 1),
            "Another port",
            false,
        ),
        (
            "a Device beside the audited one",
            oid(ObjectType::DEVICE, 11),
            "Device",
            false,
        ),
        (
            "the audited Reporter",
            oid(ObjectType::AUDIT_REPORTER, 1),
            "Reporter",
            false,
        ),
        (
            "a new object",
            oid(ObjectType::BINARY_VALUE, 3),
            "Fresh",
            true,
        ),
        (
            "the value under its own name",
            oid(ObjectType::BINARY_VALUE, 1),
            "Value",
            true,
        ),
    ];
    for (case, identifier, name, takes) in cases {
        let (mut db, _lease, _owner) = guarded();
        let object = Named(identifier, name);
        let checked = db.check_addable(&object).map_err(|e| format!("{e:?}"));
        let added = db.add(Box::new(object)).map_err(|e| format!("{e:?}"));
        assert_eq!(checked, added, "{case}");
        assert_eq!(added.is_ok(), takes, "{case}");
    }
}

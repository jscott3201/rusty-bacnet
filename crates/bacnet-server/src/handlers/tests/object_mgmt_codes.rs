//! CreateObject and DeleteObject refusal codes (#2171): a type the Device
//! lists but the server doesn't create against one it doesn't support
//! (Clause 15.3.1.3), the reserved instance 4194303 (Clause 12.1.1), and
//! DeleteObject asking the object itself (Clause 15.4.1.3).

use super::*;
use bacnet_objects::device::SUPPORTED_OBJECT_TYPES;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
use std::borrow::Cow;

fn create(db: &mut ObjectDatabase, object_specifier: ObjectSpecifier) -> Result<(), Error> {
    let mut request = BytesMut::new();
    CreateObjectRequest {
        object_specifier,
        list_of_initial_values: vec![],
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    let result = handle_create_object(db, &request, &mut response);
    if result.is_err() {
        assert!(response.is_empty());
    }
    result
}

fn delete(db: &mut ObjectDatabase, oid: ObjectIdentifier) -> Result<(), Error> {
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut request);
    handle_delete_object(db, &request).map(|_removed| ())
}

fn refusal(result: Result<(), Error>) -> (ErrorClass, ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => (class, code),
        other => panic!("expected a protocol refusal, got {other:?}"),
    }
}

fn object(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

/// The types the factory builds; the rest of the listed types aren't
/// creatable (#2172).
const CREATABLE: [ObjectType; 8] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
];

#[test]
fn a_listed_type_the_server_does_not_create_is_dynamic_creation_not_supported() {
    for &kind in SUPPORTED_OBJECT_TYPES {
        if CREATABLE.contains(&kind) {
            continue;
        }
        for specifier in [
            ObjectSpecifier::Type(kind),
            ObjectSpecifier::Identifier(object(kind, 5)),
        ] {
            let mut db = make_db_with_device_and_ai();
            assert_eq!(
                refusal(create(&mut db, specifier)),
                (
                    ErrorClass::OBJECT,
                    ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED
                ),
                "{kind:?}"
            );
            assert_eq!(db.len(), 2, "{kind:?}");
        }
    }
}

#[test]
fn an_unlisted_type_is_unsupported_object_type() {
    // Network Security (38) is deleted from the standard and not listed;
    // types from 65 to 1023 fit the identifier but none is implemented here,
    // and 1024 doesn't fit the identifier at all.
    for raw in [38, 65, 127, 1023, 1024] {
        let kind = ObjectType::from_raw(raw);
        assert!(!SUPPORTED_OBJECT_TYPES.contains(&kind));
        let mut db = make_db_with_device_and_ai();
        assert_eq!(
            refusal(create(&mut db, ObjectSpecifier::Type(kind))),
            (ErrorClass::OBJECT, ErrorCode::UNSUPPORTED_OBJECT_TYPE),
            "{raw}"
        );
        assert_eq!(db.len(), 2);
    }
}

#[test]
fn an_unlisted_type_is_unsupported_whatever_its_instance() {
    // The type refusal comes first, so the reserved instance doesn't change
    // it (#2171).
    for instance in [5, ObjectIdentifier::WILDCARD_INSTANCE] {
        let mut db = make_db_with_device_and_ai();
        let unlisted = object(ObjectType::from_raw(600), instance);
        assert_eq!(
            refusal(create(&mut db, ObjectSpecifier::Identifier(unlisted))),
            (ErrorClass::OBJECT, ErrorCode::UNSUPPORTED_OBJECT_TYPE),
            "{instance}"
        );
        assert_eq!(db.len(), 2);
    }
}

#[test]
fn the_reserved_instance_is_refused_and_nothing_is_added() {
    for kind in [ObjectType::BINARY_VALUE, ObjectType::ANALOG_VALUE] {
        let mut db = make_db_with_device_and_ai();
        let reserved = object(kind, ObjectIdentifier::WILDCARD_INSTANCE);
        assert_eq!(
            refusal(create(&mut db, ObjectSpecifier::Identifier(reserved))),
            (
                ErrorClass::OBJECT,
                ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED
            ),
            "{kind:?}"
        );
        assert!(db.get(&reserved).is_none());
        assert!(db.find_by_type(kind).is_empty());
        assert_eq!(db.len(), 2);
    }
    // The last addressable instance is still creatable.
    let mut db = make_db_with_device_and_ai();
    let last = object(
        ObjectType::BINARY_VALUE,
        ObjectIdentifier::MAX_ADDRESSABLE_INSTANCE,
    );
    create(&mut db, ObjectSpecifier::Identifier(last)).unwrap();
    assert!(db.get(&last).is_some());
}

/// An application's own object of a vendor type the Device doesn't list.
struct Vendor(ObjectIdentifier);

impl BACnetObject for Vendor {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0
    }

    fn object_name(&self) -> &str {
        "vendor-object"
    }

    fn read_property(&self, _: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ))
    }

    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        bacnet_objects::property_metadata::property_list_from_metadata(&self.property_metadata())
    }
}

#[test]
fn an_identifier_in_use_is_refused_before_its_type_is_judged() {
    // Clause 15.3.1.3 looks for the named object first (#2171), so an
    // application's object of an unlisted vendor type is already there,
    // not an unsupported type.
    let mut db = make_db_with_device_and_ai();
    let vendor = object(ObjectType::from_raw(600), 7);
    assert!(!SUPPORTED_OBJECT_TYPES.contains(&vendor.object_type()));
    db.add(Box::new(Vendor(vendor))).unwrap();
    assert_eq!(
        refusal(create(&mut db, ObjectSpecifier::Identifier(vendor))),
        (
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS
        )
    );
    // A free identifier of the same type is still an unsupported type.
    assert_eq!(
        refusal(create(
            &mut db,
            ObjectSpecifier::Identifier(object(ObjectType::from_raw(600), 8))
        )),
        (ErrorClass::OBJECT, ErrorCode::UNSUPPORTED_OBJECT_TYPE)
    );
    assert_eq!(db.len(), 3);
    assert!(db.get(&vendor).is_some());
}

/// A Binary Input that its application keeps from being deleted.
struct Pinned(bacnet_objects::binary::BinaryInputObject);

impl BACnetObject for Pinned {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.0.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.0.read_property(property, array_index)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0
            .write_property(property, array_index, value, priority)
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.0.property_metadata()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.0.property_list()
    }

    fn is_deleteable(&self) -> bool {
        false
    }
}

#[test]
fn delete_object_asks_the_object_whether_it_may_go() {
    let mut db = make_db_with_device_and_ai();
    let pinned = object(ObjectType::BINARY_INPUT, 3);
    db.add(Box::new(Pinned(
        bacnet_objects::binary::BinaryInputObject::new(3, "BI-3").unwrap(),
    )))
    .unwrap();
    assert_eq!(
        refusal(delete(&mut db, pinned)),
        (ErrorClass::OBJECT, ErrorCode::OBJECT_DELETION_NOT_PERMITTED)
    );
    assert!(db.get(&pinned).is_some());
    // An object that allows it goes, whatever its type.
    let ai = object(ObjectType::ANALOG_INPUT, 1);
    delete(&mut db, ai).unwrap();
    assert!(db.get(&ai).is_none());
}

#[test]
fn delete_object_for_an_absent_object_is_unknown_object_whatever_its_type() {
    let mut db = make_db_with_device_and_ai();
    let device = db.selected_device().unwrap();
    for absent in [
        object(ObjectType::DEVICE, device.instance_number() + 1),
        object(ObjectType::NETWORK_PORT, 9),
        object(ObjectType::ANALOG_INPUT, 9),
    ] {
        assert_eq!(
            refusal(delete(&mut db, absent)),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
            "{absent:?}"
        );
    }
    // The Device itself is present and refuses.
    assert_eq!(
        refusal(delete(&mut db, device)),
        (ErrorClass::OBJECT, ErrorCode::OBJECT_DELETION_NOT_PERMITTED)
    );
    assert_eq!(db.len(), 2);
}

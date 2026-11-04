//! CreateObject through an application's registered constructors (#2173),
//! on the wire: a proprietary type, an override of a built-in type, and
//! the constructor answers the server refuses.

use super::mutation_tests::{Fixture, oid, value};
use super::*;
use crate::mutation::MutationPolicy;
use crate::server::object_type_registry::tests::{VENDOR_TYPE, Vendor, vendor_type};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::{CreateObjectAck, CreateObjectRequest, ObjectSpecifier};
use bacnet_types::enums::EngineeringUnits;
use std::sync::atomic::Ordering;

const CREATE: ConfirmedServiceChoice = ConfirmedServiceChoice::CREATE_OBJECT;

fn request(specifier: ObjectSpecifier, values: Vec<(PropertyIdentifier, PropertyValue)>) -> Bytes {
    let mut bytes = BytesMut::new();
    CreateObjectRequest {
        object_specifier: specifier,
        list_of_initial_values: values
            .into_iter()
            .map(|(property_identifier, initial)| BACnetPropertyValue {
                property_identifier,
                property_array_index: None,
                value: value(initial),
                priority: None,
            })
            .collect(),
    }
    .encode(&mut bytes)
    .unwrap();
    bytes.freeze()
}

async fn send(fixture: &Fixture, request: Bytes) -> Vec<u8> {
    let npdu = fixture
        .dispatch(CREATE, request, 5)
        .await
        .expect("response");
    bacnet_encoding::npdu::decode_npdu(npdu)
        .unwrap()
        .payload
        .to_vec()
}

fn created(response: &[u8]) -> ObjectIdentifier {
    assert_eq!(&response[..3], &[0x30, 5, CREATE.to_raw()], "{response:?}");
    CreateObjectAck::decode(&response[3..])
        .unwrap()
        .object_identifier
}

/// The CreateObject-Error for invoke ID 5.
fn refused(class: ErrorClass, code: ErrorCode, element: u8) -> Vec<u8> {
    vec![
        0x50,
        5,
        CREATE.to_raw(),
        0x0E,
        0x91,
        class.to_raw() as u8,
        0x91,
        code.to_raw() as u8,
        0x0F,
        0x19,
        element,
    ]
}

fn with_vendor(fixture: &mut Fixture) {
    fixture
        .config
        .object_types
        .register(vendor_type(), |oid, name| Ok(Vendor::new(oid, name)))
        .unwrap();
}

async fn read(
    fixture: &Fixture,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    fixture
        .db
        .read()
        .await
        .get(&object)
        .expect("created")
        .read_property(property, None)
        .unwrap()
}

#[tokio::test]
async fn a_registered_proprietary_type_is_created_with_its_initial_values() {
    let mut fixture = Fixture::new(None);
    // Unregistered, the type is one the device doesn't support.
    assert_eq!(
        send(
            &fixture,
            request(ObjectSpecifier::Type(vendor_type()), vec![])
        )
        .await,
        refused(ErrorClass::OBJECT, ErrorCode::UNSUPPORTED_OBJECT_TYPE, 0)
    );
    with_vendor(&mut fixture);
    let by_type = created(
        &send(
            &fixture,
            request(
                ObjectSpecifier::Type(vendor_type()),
                vec![
                    (
                        PropertyIdentifier::OBJECT_NAME,
                        PropertyValue::CharacterString("pump".into()),
                    ),
                    (
                        PropertyIdentifier::PRESENT_VALUE,
                        PropertyValue::Unsigned(42),
                    ),
                ],
            ),
        )
        .await,
    );
    assert_eq!(by_type, oid(vendor_type(), 1));
    assert_eq!(
        read(&fixture, by_type, PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Unsigned(42)
    );
    assert!(fixture.db.read().await.find_by_name("pump").is_some());

    // By identifier, with the default name.
    let wanted = oid(vendor_type(), 9);
    assert_eq!(
        created(
            &send(
                &fixture,
                request(ObjectSpecifier::Identifier(wanted), vec![])
            )
            .await
        ),
        wanted
    );
    assert_eq!(
        read(&fixture, wanted, PropertyIdentifier::OBJECT_NAME).await,
        PropertyValue::CharacterString(format!("{}-9", vendor_type()))
    );
    // An identifier in use and the reserved instance are refused as for a
    // built-in type.
    for (identifier, code) in [
        (wanted, ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS),
        (
            oid(vendor_type(), ObjectIdentifier::WILDCARD_INSTANCE),
            ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
        ),
    ] {
        assert_eq!(
            send(
                &fixture,
                request(ObjectSpecifier::Identifier(identifier), vec![])
            )
            .await,
            refused(ErrorClass::OBJECT, code, 0)
        );
    }
}

#[tokio::test]
async fn a_refused_initial_value_of_a_registered_type_is_named_and_dropped() {
    let mut fixture = Fixture::new(None);
    with_vendor(&mut fixture);
    let before = fixture.db.read().await.len();
    assert_eq!(
        send(
            &fixture,
            request(
                ObjectSpecifier::Type(vendor_type()),
                vec![
                    (
                        PropertyIdentifier::PRESENT_VALUE,
                        PropertyValue::Unsigned(1)
                    ),
                    (
                        PropertyIdentifier::PRESENT_VALUE,
                        PropertyValue::Boolean(true)
                    ),
                ],
            ),
        )
        .await,
        refused(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2)
    );
    assert_eq!(fixture.db.read().await.len(), before);
}

#[tokio::test]
async fn a_registered_analog_value_constructor_replaces_the_built_in() {
    let mut fixture = Fixture::new(None);
    fixture
        .config
        .object_types
        .register(ObjectType::ANALOG_VALUE, |oid, name| {
            let mut object = AnalogValueObject::new(
                oid.instance_number(),
                name,
                EngineeringUnits::DEGREES_CELSIUS,
            )?;
            object.set_description("site default");
            Ok(object)
        })
        .unwrap();
    let object = created(
        &send(
            &fixture,
            request(ObjectSpecifier::Type(ObjectType::ANALOG_VALUE), vec![]),
        )
        .await,
    );
    assert_eq!(
        read(&fixture, object, PropertyIdentifier::UNITS).await,
        PropertyValue::Enumerated(EngineeringUnits::DEGREES_CELSIUS.to_raw())
    );
    assert_eq!(
        read(&fixture, object, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("site default".into())
    );
    // The object's creation-only Units still takes an initial value.
    let hertz = EngineeringUnits::HERTZ.to_raw();
    let second = created(
        &send(
            &fixture,
            request(
                ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
                vec![(PropertyIdentifier::UNITS, PropertyValue::Enumerated(hertz))],
            ),
        )
        .await,
    );
    assert_eq!(
        read(&fixture, second, PropertyIdentifier::UNITS).await,
        PropertyValue::Enumerated(hertz)
    );
    // The constructor didn't provision the audit rows, so an Audit_Level
    // initial value is refused at its position, as a write to the absent
    // row would be, and nothing is added.
    let before = fixture.db.read().await.len();
    let response = send(
        &fixture,
        request(
            ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
            vec![(
                PropertyIdentifier::AUDIT_LEVEL,
                PropertyValue::Enumerated(bacnet_types::enums::AuditLevel::DEFAULT.to_raw()),
            )],
        ),
    )
    .await;
    assert_eq!(
        response,
        refused(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY, 1)
    );
    assert_eq!(fixture.db.read().await.len(), before);
}

#[tokio::test]
async fn a_constructor_that_builds_another_object_is_refused() {
    let mut fixture = Fixture::new(None);
    fixture
        .config
        .object_types
        .register(vendor_type(), |oid, name| {
            let other = ObjectIdentifier::new(oid.object_type(), oid.instance_number() + 1)?;
            Ok(Vendor::new(other, name))
        })
        .unwrap();
    let before = fixture.db.read().await.len();
    for specifier in [
        ObjectSpecifier::Type(vendor_type()),
        ObjectSpecifier::Identifier(oid(vendor_type(), 3)),
    ] {
        assert_eq!(
            send(&fixture, request(specifier, vec![])).await,
            refused(
                ErrorClass::OBJECT,
                ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
                0
            )
        );
    }
    let db = fixture.db.read().await;
    assert_eq!(db.len(), before);
    assert!(db.find_by_type(vendor_type()).is_empty());
}

#[tokio::test]
async fn a_constructor_refusal_reaches_the_client_and_a_panic_creates_nothing() {
    let mut fixture = Fixture::new(None);
    let raw = |offset| ObjectType::from_raw(VENDOR_TYPE + offset);
    let constructors = &mut fixture.config.object_types;
    constructors
        .register::<Vendor, _>(raw(1), |_, _| {
            Err(Error::protocol(
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_FOR_OBJECT,
            ))
        })
        .unwrap();
    constructors
        .register::<Vendor, _>(raw(2), |_, _| Err(Error::Encoding("not now".into())))
        .unwrap();
    constructors
        .register::<Vendor, _>(raw(3), |_, _| panic!("constructor bug"))
        .unwrap();
    let before = fixture.db.read().await.len();
    for (kind, class, code) in [
        (
            raw(1),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_FOR_OBJECT,
        ),
        (
            raw(2),
            ErrorClass::OBJECT,
            ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
        ),
        (
            raw(3),
            ErrorClass::OBJECT,
            ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
        ),
    ] {
        assert_eq!(
            send(&fixture, request(ObjectSpecifier::Type(kind), vec![])).await,
            refused(class, code, 0),
            "{kind:?}"
        );
    }
    assert_eq!(fixture.db.read().await.len(), before);
}

#[tokio::test]
async fn the_policy_decides_a_registered_type_before_its_constructor_runs() {
    let mut fixture = Fixture::new(None);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    fixture
        .config
        .object_types
        .register(vendor_type(), move |oid, name| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(Vendor::new(oid, name))
        })
        .unwrap();
    fixture.config.mutation_policy = MutationPolicy::DenyAll;
    assert_eq!(
        send(
            &fixture,
            request(ObjectSpecifier::Type(vendor_type()), vec![])
        )
        .await,
        refused(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED, 0)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    fixture.config.mutation_policy = MutationPolicy::Permissive;
    created(
        &send(
            &fixture,
            request(ObjectSpecifier::Type(vendor_type()), vec![]),
        )
        .await,
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_registered_type_refuses_a_taken_identifier_and_the_reserved_instance() {
    let mut fixture = Fixture::new(None);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    fixture
        .config
        .object_types
        .register(vendor_type(), move |oid, name| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(Vendor::new(oid, name))
        })
        .unwrap();
    let taken = oid(vendor_type(), 3);
    assert_eq!(
        created(
            &send(
                &fixture,
                request(ObjectSpecifier::Identifier(taken), vec![])
            )
            .await
        ),
        taken
    );
    let before = fixture.db.read().await.len();
    // #2171's order: the identifier in use is refused first, ahead of an
    // initial value that would fail, and the constructor never runs.
    assert_eq!(
        send(
            &fixture,
            request(
                ObjectSpecifier::Identifier(taken),
                vec![(
                    PropertyIdentifier::PRESENT_VALUE,
                    PropertyValue::Boolean(true)
                )],
            ),
        )
        .await,
        refused(
            ErrorClass::OBJECT,
            ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
            0
        )
    );
    // The reserved instance: the type is supported (registered), so the
    // instance check answers, again before the constructor.
    let reserved = oid(vendor_type(), ObjectIdentifier::WILDCARD_INSTANCE);
    assert_eq!(
        send(
            &fixture,
            request(ObjectSpecifier::Identifier(reserved), vec![])
        )
        .await,
        refused(
            ErrorClass::OBJECT,
            ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED,
            0
        )
    );
    // An unregistered proprietary type at the reserved instance fails the
    // type check, which comes before the instance check.
    let unregistered = oid(
        ObjectType::from_raw(VENDOR_TYPE + 1),
        ObjectIdentifier::WILDCARD_INSTANCE,
    );
    assert_eq!(
        send(
            &fixture,
            request(ObjectSpecifier::Identifier(unregistered), vec![])
        )
        .await,
        refused(ErrorClass::OBJECT, ErrorCode::UNSUPPORTED_OBJECT_TYPE, 0)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.db.read().await.len(), before);
}

#[tokio::test]
async fn creates_answers_exactly_what_create_object_builds() {
    // The registry's type-level answer, which the PICS reports, against
    // what CreateObject does: every type the Device lists, plus a
    // registered and an unregistered proprietary one, so the two can't
    // drift apart.
    let mut fixture = Fixture::new(None);
    with_vendor(&mut fixture);
    let kinds = bacnet_objects::device::SUPPORTED_OBJECT_TYPES
        .iter()
        .copied()
        .chain([vendor_type(), ObjectType::from_raw(VENDOR_TYPE + 1)]);
    for kind in kinds {
        let response = send(&fixture, request(ObjectSpecifier::Type(kind), vec![])).await;
        let built = response[0] == 0x30;
        if built {
            created(&response);
        }
        assert_eq!(
            built,
            fixture.config.object_types.creates(kind),
            "{kind:?}: {response:?}"
        );
    }
}

#[tokio::test]
async fn a_type_the_builders_keep_is_refused_by_delete_object_and_the_pics_says_so() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;
    const DELETE: ConfirmedServiceChoice = ConfirmedServiceChoice::DELETE_OBJECT;
    let kept = ObjectType::BINARY_VALUE;
    // Every builder takes it; the running server reads the generic one's.
    let bip = BACnetServer::bip_builder().keep_from_deletion(kept);
    assert!(!bip.config.object_types.deletes(kept));
    let builder =
        BACnetServer::<test_transport::TestTransport>::generic_builder().keep_from_deletion(kept);
    let mut fixture = Fixture::new(None);
    fixture.config.object_types = builder.config.object_types.clone();
    let delete = |instance| {
        let mut bytes = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid(kept, instance),
        }
        .encode(&mut bytes);
        bytes.freeze()
    };
    let payload = |npdu| {
        bacnet_encoding::npdu::decode_npdu(npdu)
            .unwrap()
            .payload
            .to_vec()
    };
    let answer = payload(
        fixture
            .dispatch(DELETE, delete(1), 5)
            .await
            .expect("response"),
    );
    assert_eq!(
        answer,
        [
            0x50,
            5,
            DELETE.to_raw(),
            0x91,
            ErrorClass::OBJECT.to_raw() as u8,
            0x91,
            ErrorCode::OBJECT_DELETION_NOT_PERMITTED.to_raw() as u8,
        ]
    );
    assert!(fixture.db.read().await.get(&oid(kept, 1)).is_some());
    let row = |config: &ServerConfig, db: &ObjectDatabase| {
        crate::pics::generate_pics(db, config, &crate::pics::PicsConfig::default())
            .supported_object_types
            .into_iter()
            .find(|support| support.object_type == kept)
            .unwrap()
            .deleteable
    };
    assert!(!row(&fixture.config, &*fixture.db.read().await));

    // Without the registration the same request removes it.
    fixture.config.object_types = Default::default();
    assert!(row(&fixture.config, &*fixture.db.read().await));
    let answer = payload(
        fixture
            .dispatch(DELETE, delete(1), 5)
            .await
            .expect("response"),
    );
    assert_eq!(answer, [0x20, 5, DELETE.to_raw()]);
    assert!(fixture.db.read().await.get(&oid(kept, 1)).is_none());
}

//! The types CreateObject builds since #2172 (Analog Value and the twelve
//! value types), each created over the wire with and without an initial
//! value, and each refusing a bad initial value with a CreateObject-Error
//! that names it (Clause 15.3.1.3).

use super::mutation_tests::{Fixture, oid, value};
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::{CreateObjectAck, CreateObjectRequest, ObjectSpecifier};
use bacnet_types::primitives::{Date, Time};

const CREATE: ConfirmedServiceChoice = ConfirmedServiceChoice::CREATE_OBJECT;

pub(super) fn request(
    specifier: ObjectSpecifier,
    values: Vec<(PropertyIdentifier, Vec<u8>)>,
) -> Bytes {
    let mut bytes = BytesMut::new();
    CreateObjectRequest {
        object_specifier: specifier,
        list_of_initial_values: values
            .into_iter()
            .map(|(property_identifier, value)| BACnetPropertyValue {
                property_identifier,
                property_array_index: None,
                value,
                priority: None,
            })
            .collect(),
    }
    .encode(&mut bytes)
    .unwrap();
    bytes.freeze()
}

/// The response APDU exactly as the server encoded it.
pub(super) async fn send(fixture: &Fixture, request: Bytes) -> Vec<u8> {
    let npdu = fixture
        .dispatch(CREATE, request, 5)
        .await
        .expect("response");
    bacnet_encoding::npdu::decode_npdu(npdu)
        .unwrap()
        .payload
        .to_vec()
}

/// The identifier a CreateObject ComplexACK for invoke ID 5 carries.
pub(super) fn created(response: &[u8]) -> ObjectIdentifier {
    assert_eq!(&response[..3], &[0x30, 5, CREATE.to_raw()], "{response:?}");
    CreateObjectAck::decode(&response[3..])
        .unwrap()
        .object_identifier
}

/// Each new type with a Present_Value it takes as an initial value.
fn new_types() -> Vec<(ObjectType, PropertyValue)> {
    let date = Date {
        year: 126,
        month: 10,
        day: 31,
        day_of_week: 6,
    };
    let time = Time {
        hour: 12,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    let any_day = Date {
        day: 0xFF,
        day_of_week: 0xFF,
        ..date
    };
    let any_hour = Time { hour: 0xFF, ..time };
    vec![
        (ObjectType::ANALOG_VALUE, PropertyValue::Real(21.5)),
        (ObjectType::INTEGER_VALUE, PropertyValue::Signed(-5)),
        (
            ObjectType::POSITIVE_INTEGER_VALUE,
            PropertyValue::Unsigned(7),
        ),
        (ObjectType::LARGE_ANALOG_VALUE, PropertyValue::Double(1.25)),
        (
            ObjectType::CHARACTERSTRING_VALUE,
            PropertyValue::CharacterString("hello".into()),
        ),
        (
            ObjectType::OCTETSTRING_VALUE,
            PropertyValue::OctetString(vec![1, 2, 3]),
        ),
        (
            ObjectType::BITSTRING_VALUE,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0xA0],
            },
        ),
        (ObjectType::DATE_VALUE, PropertyValue::Date(date)),
        (ObjectType::TIME_VALUE, PropertyValue::Time(time)),
        (
            ObjectType::DATETIME_VALUE,
            PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]),
        ),
        (ObjectType::DATEPATTERN_VALUE, PropertyValue::Date(any_day)),
        (ObjectType::TIMEPATTERN_VALUE, PropertyValue::Time(any_hour)),
        (
            ObjectType::DATETIMEPATTERN_VALUE,
            PropertyValue::List(vec![
                PropertyValue::Date(any_day),
                PropertyValue::Time(any_hour),
            ]),
        ),
    ]
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
async fn each_new_type_is_created_without_initial_values() {
    let fixture = Fixture::new(None);
    for (kind, _) in new_types() {
        let before = fixture.db.read().await.len();
        let object = created(&send(&fixture, request(ObjectSpecifier::Type(kind), vec![])).await);
        assert_eq!(object, oid(kind, 1), "{kind:?}");
        assert_eq!(fixture.db.read().await.len(), before + 1);
        assert_eq!(
            read(&fixture, object, PropertyIdentifier::OBJECT_NAME).await,
            PropertyValue::CharacterString(format!("{kind}-1")),
        );
        assert_eq!(
            read(&fixture, object, PropertyIdentifier::OBJECT_TYPE).await,
            PropertyValue::Enumerated(kind.to_raw()),
        );
    }
}

#[tokio::test]
async fn each_new_type_takes_its_initial_values_as_a_write_would() {
    let fixture = Fixture::new(None);
    for (kind, present_value) in new_types() {
        let wanted = oid(kind, 40);
        let response = send(
            &fixture,
            request(
                ObjectSpecifier::Identifier(wanted),
                vec![
                    (
                        PropertyIdentifier::OBJECT_NAME,
                        value(PropertyValue::CharacterString(format!("new {kind}"))),
                    ),
                    (
                        PropertyIdentifier::PRESENT_VALUE,
                        value(present_value.clone()),
                    ),
                ],
            ),
        )
        .await;
        assert_eq!(created(&response), wanted, "{kind:?}");
        assert_eq!(
            read(&fixture, wanted, PropertyIdentifier::PRESENT_VALUE).await,
            present_value,
            "{kind:?}"
        );
        assert_eq!(
            read(&fixture, wanted, PropertyIdentifier::OBJECT_NAME).await,
            PropertyValue::CharacterString(format!("new {kind}")),
        );
        // The value came in as a priority-16 command, as a WriteProperty
        // without a priority does.
        let slot = fixture
            .db
            .read()
            .await
            .get(&wanted)
            .unwrap()
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(16))
            .unwrap();
        assert_ne!(slot, PropertyValue::Null, "{kind:?}");
    }
}

#[tokio::test]
async fn analog_value_takes_units_only_at_creation() {
    let fixture = Fixture::new(None);
    let celsius = bacnet_types::enums::EngineeringUnits::DEGREES_CELSIUS.to_raw();
    let object = created(
        &send(
            &fixture,
            request(
                ObjectSpecifier::Type(ObjectType::ANALOG_VALUE),
                vec![(
                    PropertyIdentifier::UNITS,
                    value(PropertyValue::Enumerated(celsius)),
                )],
            ),
        )
        .await,
    );
    assert_eq!(
        read(&fixture, object, PropertyIdentifier::UNITS).await,
        PropertyValue::Enumerated(celsius)
    );
}

#[tokio::test]
async fn a_refused_initial_value_is_named_and_nothing_is_created() {
    let fixture = Fixture::new(None);
    let before = fixture.db.read().await.len();
    for (kind, _) in new_types() {
        let response = send(
            &fixture,
            request(
                ObjectSpecifier::Type(kind),
                vec![
                    (
                        PropertyIdentifier::DESCRIPTION,
                        value(PropertyValue::CharacterString("d".into())),
                    ),
                    (
                        PropertyIdentifier::PRESENT_VALUE,
                        value(PropertyValue::Boolean(true)),
                    ),
                ],
            ),
        )
        .await;
        // CreateObject-Error: the class and code in tag 0, then element 2.
        assert_eq!(
            response,
            vec![
                0x50,
                5,
                CREATE.to_raw(),
                0x0E,
                0x91,
                ErrorClass::PROPERTY.to_raw() as u8,
                0x91,
                ErrorCode::INVALID_DATA_TYPE.to_raw() as u8,
                0x0F,
                0x19,
                2,
            ],
            "{kind:?}"
        );
        let db = fixture.db.read().await;
        assert_eq!(db.len(), before, "{kind:?}");
        assert!(db.find_by_type(kind).is_empty(), "{kind:?}");
    }
}

#[tokio::test]
async fn a_freshly_created_object_of_each_type_reads_whole_with_rpm_all() {
    use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
    use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
    let rpm = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
    let fixture = Fixture::new(None);
    // Every type the table builds, the new ones (#2172) among them.
    let kinds: Vec<ObjectType> = bacnet_objects::creation::builtin_createable_types().collect();
    for (kind, _) in new_types() {
        assert!(kinds.contains(&kind), "{kind:?}");
    }
    for kind in kinds {
        let object = created(&send(&fixture, request(ObjectSpecifier::Type(kind), vec![])).await);
        let mut bytes = BytesMut::new();
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: object,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::ALL,
                    property_array_index: None,
                }],
            }],
        }
        .encode(&mut bytes)
        .unwrap();
        let npdu = fixture
            .dispatch(rpm, bytes.freeze(), 6)
            .await
            .expect("response");
        let response = bacnet_encoding::npdu::decode_npdu(npdu)
            .unwrap()
            .payload
            .to_vec();
        assert_eq!(&response[..3], &[0x30, 6, rpm.to_raw()], "{kind:?}");
        let ack = ReadPropertyMultipleACK::decode(&response[3..]).unwrap();
        let [result] = ack.list_of_read_access_results.as_slice() else {
            panic!("{kind:?}: one object asked for");
        };
        assert_eq!(result.object_identifier, object);
        // Every property the object lists reads without an error.
        let listed = fixture
            .db
            .read()
            .await
            .get(&object)
            .unwrap()
            .property_metadata()
            .into_owned();
        for property in listed
            .iter()
            .map(|row| &row.property_identifier)
            .filter(|&&p| {
                p != PropertyIdentifier::PROPERTY_LIST && p != PropertyIdentifier::LOG_BUFFER
            })
        {
            let element = result
                .list_of_results
                .iter()
                .find(|element| element.property_identifier == *property)
                .unwrap_or_else(|| panic!("{kind:?}: {property:?} missing"));
            assert!(element.property_value.is_some(), "{kind:?}: {element:?}");
        }
        // Except a log's buffer, which only ReadRange reads.
        for element in &result.list_of_results {
            if element.property_identifier == PropertyIdentifier::LOG_BUFFER {
                continue;
            }
            assert_eq!(element.error, None, "{kind:?}: {element:?}");
        }
    }
}

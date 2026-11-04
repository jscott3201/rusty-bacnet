//! An array written whole with no octets reaches its object over the wire
//! (#2305, Clause 15.9.1.1.3), by WriteProperty, WritePropertyMultiple and
//! `write_local_encoded` alike, and the object answers by its own rules.
//!
//! CMD-1's Action is read-only. CH-2 holds two members, and its member list
//! can shrink to none, its delays with it; CH-1 has no members, so its
//! delays take an empty array, and one control group, the floor. AI-1's
//! Event_Message_Texts_Config always holds three elements, and MSV-1's
//! State_Text one per state, at least one. Zero octets for one value, or
//! for one element of an array, stay undecodable.
//!
//! CreateObject's initial values take the same decoding, so an empty array
//! among them reaches the new object too.
use super::channel_wire_tests::{ch, channel, member, write_wire};
use super::command_action_wire_tests::read_wire;
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::{
    CreateObjectAck, CreateObjectError, CreateObjectRequest, ObjectSpecifier,
};
use bacnet_services::wpm::{
    WriteAccessSpecification, WritePropertyMultipleError, WritePropertyMultipleRequest,
};
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::error::RemoteError;

type P = PropertyIdentifier;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn objects(db: &mut ObjectDatabase) {
    db.add(Box::new(CommandObject::new(1, "CMD-1").unwrap()))
        .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()))
        .unwrap();
    let mut empty = channel(1, 11, vec![]);
    empty.set_control_groups(vec![7]).unwrap();
    db.add(Box::new(empty)).unwrap();
    db.add(Box::new(channel(
        2,
        12,
        vec![(member(av1(), PV), 0), (member(av1(), PV), 100)],
    )))
    .unwrap();
}

/// What an empty write answers, or `None` for success.
type Answer = Option<(ErrorClass, ErrorCode)>;

const ACCEPTED: Answer = None;
const ACCESS_DENIED: Answer = Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
const OUT_OF_RANGE: Answer = Some((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE));
const UNDECODABLE: Answer = Some((ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING));

/// Each empty write and its answer: one per behaviour.
fn cases() -> Vec<(ObjectIdentifier, P, Option<u32>, Answer)> {
    let ai1 = oid(ObjectType::ANALOG_INPUT, 1);
    vec![
        // A read-only array: refused for access, as a non-empty write is.
        (oid(ObjectType::COMMAND, 1), P::ACTION, None, ACCESS_DENIED),
        // Delays sized by two members, then the member list shrunk to
        // nothing, and delays for no members.
        (ch(2), P::EXECUTION_DELAY, None, OUT_OF_RANGE),
        (ch(2), P::LIST_OF_OBJECT_PROPERTY_REFERENCES, None, ACCEPTED),
        (ch(1), P::EXECUTION_DELAY, None, ACCEPTED),
        // A fixed size, and two sizes with a floor of one.
        (ai1, P::EVENT_MESSAGE_TEXTS_CONFIG, None, OUT_OF_RANGE),
        (ch(1), P::CONTROL_GROUPS, None, OUT_OF_RANGE),
        (
            oid(ObjectType::MULTI_STATE_VALUE, 1),
            P::STATE_TEXT,
            None,
            OUT_OF_RANGE,
        ),
        // One value, and one element of an array.
        (ai1, P::DESCRIPTION, None, UNDECODABLE),
        (ch(1), P::CONTROL_GROUPS, Some(1), UNDECODABLE),
    ]
}

/// The sizes each case leaves: CH-2's two arrays emptied, the rest as they
/// were.
async fn assert_sizes(h: &mut Harness) {
    let sizes = [
        (ch(2), P::LIST_OF_OBJECT_PROPERTY_REFERENCES, 0),
        (ch(2), P::EXECUTION_DELAY, 0),
        (ch(1), P::EXECUTION_DELAY, 0),
        (ch(1), P::CONTROL_GROUPS, 1),
        (
            oid(ObjectType::ANALOG_INPUT, 1),
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            3,
        ),
        (oid(ObjectType::MULTI_STATE_VALUE, 1), P::STATE_TEXT, 3),
    ];
    for (object, property, size) in sizes {
        assert_eq!(
            read_wire(h, object, property, Some(0)).await,
            Ok(vec![0x21, size]),
            "{object:?} {property:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn write_property_of_an_empty_array_reaches_the_object_over_the_wire() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    for (object, property, index, expected) in cases() {
        let answer = write_wire(&mut h, object, property, index, Vec::new(), None)
            .await
            .err()
            .map(|error| (error.error_class, error.error_code));
        assert_eq!(answer, expected, "{object:?} {property:?} {index:?}");
    }
    assert_sizes(&mut h).await;
}

#[tokio::test(start_paused = true)]
async fn write_property_multiple_of_an_empty_array_reaches_the_object_over_the_wire() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    for (object, property, index, expected) in cases() {
        let mut body = BytesMut::new();
        WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![WriteAccessSpecification {
                object_identifier: object,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: index,
                    value: Vec::new(),
                    priority: None,
                }],
            }],
        }
        .encode(&mut body)
        .unwrap();
        h.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
            .await;
        let answer = match error_response(&h).await {
            Ok(()) => None,
            Err(error) => {
                let refused = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
                let failed = refused.first_failed_write_attempt;
                assert_eq!(
                    (
                        failed.object_identifier,
                        failed.property_identifier,
                        failed.property_array_index
                    ),
                    (object, property.to_raw(), index)
                );
                Some((refused.error_class, refused.error_code))
            }
        };
        assert_eq!(answer, expected, "{object:?} {property:?} {index:?}");
    }
    assert_sizes(&mut h).await;
}

#[tokio::test(start_paused = true)]
async fn a_local_encoded_write_of_an_empty_array_reaches_the_object() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    // The same answers as over the wire: CMD-1 refuses its Action itself.
    for (object, property, index, expected) in cases() {
        let answer = h
            .server
            .write_local_encoded(
                &object,
                property,
                index,
                &[],
                None,
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .err()
            .map(|error| match error.remote() {
                Some(RemoteError::Error { class, code }) => (class, code),
                other => panic!("{other:?}"),
            });
        assert_eq!(answer, expected, "{object:?} {property:?} {index:?}");
    }
    assert_sizes(&mut h).await;
}

/// CreateObject of `object_type` with `values` as its initial values, over
/// the wire: the object created, or the CreateObject-Error.
async fn create(
    h: &mut Harness,
    object_type: ObjectType,
    values: Vec<(P, Vec<u8>)>,
) -> Result<ObjectIdentifier, CreateObjectError> {
    let mut body = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(object_type),
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
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::CREATE_OBJECT, body).await;
    let invoke_id = h.invoke_id;
    let reply = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let reply = {
                let mut frames = h.frames.lock().unwrap();
                let at = frames.iter().position(|apdu| match apdu {
                    Apdu::ComplexAck(ack) => ack.invoke_id == invoke_id,
                    Apdu::Error(error) => error.invoke_id == invoke_id,
                    _ => false,
                });
                at.map(|at| frames.remove(at))
            };
            match reply {
                Some(reply) => return reply,
                None => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
    })
    .await
    .expect("a reply to the CreateObject");
    match reply {
        Apdu::ComplexAck(ack) => Ok(CreateObjectAck::decode(&ack.service_ack)
            .unwrap()
            .object_identifier),
        Apdu::Error(error) => Err(CreateObjectError::try_from(&error as &ErrorPdu).unwrap()),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn create_object_takes_an_empty_array_as_an_initial_value() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    let name =
        |name: &str| channel_wire_tests::encoded(&PropertyValue::CharacterString(name.into()));
    // A Notification Class's Priority always holds three elements: refused
    // as out of range, naming the second initial value, and nothing created.
    let refused = create(
        &mut h,
        ObjectType::NOTIFICATION_CLASS,
        vec![(P::OBJECT_NAME, name("NC")), (P::PRIORITY, Vec::new())],
    )
    .await
    .unwrap_err();
    assert_eq!(
        (
            refused.error_class,
            refused.error_code,
            refused.first_failed_element_number
        ),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2)
    );
    assert!(
        h.server
            .database()
            .read()
            .await
            .find_by_type(ObjectType::NOTIFICATION_CLASS)
            .is_empty()
    );
    // A Structured View's Subordinate_Annotations can be empty.
    let view = create(
        &mut h,
        ObjectType::STRUCTURED_VIEW,
        vec![(P::SUBORDINATE_ANNOTATIONS, Vec::new())],
    )
    .await
    .unwrap();
    assert_eq!(
        read_wire(&mut h, view, P::SUBORDINATE_ANNOTATIONS, Some(0)).await,
        Ok(vec![0x21, 0])
    );
}

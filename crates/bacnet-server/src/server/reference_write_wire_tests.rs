//! The single-reference properties answer the same WriteProperty octets the
//! same way on a running server (#1395): the Loop and Pulse Converter
//! `BACnetObjectPropertyReference` properties, the Loop's
//! `BACnetSetpointReference`, and the Averaging Object_Property_Reference,
//! whose device-qualified encoding without a Device member is the same
//! octets. The server hands each value to its object whole, and both
//! reference modules decode it with one single-reference decoder.
//!
//! Each row writes a valid reference first, so a refusal has something to
//! leave alone; ReadProperty then serves exactly the octets it served before,
//! and a taken value reads back as written.
use super::command_action_wire_tests::read_wire;
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::loop_obj::LoopObject;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType};

type P = PropertyIdentifier;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// One single-reference property, and whether it holds the setpoint frame.
struct User {
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    setpoint: bool,
}

fn users() -> [User; 5] {
    let user = |object_type, property, setpoint| User {
        object: oid(object_type, 1),
        property,
        setpoint,
    };
    [
        user(ObjectType::LOOP, P::CONTROLLED_VARIABLE_REFERENCE, false),
        user(ObjectType::LOOP, P::MANIPULATED_VARIABLE_REFERENCE, false),
        user(ObjectType::LOOP, P::SETPOINT_REFERENCE, true),
        user(ObjectType::PULSE_CONVERTER, P::INPUT_REFERENCE, false),
        user(ObjectType::AVERAGING, P::OBJECT_PROPERTY_REFERENCE, false),
    ]
}

/// The members of a reference to AV-`instance`'s Present_Value.
fn members(instance: u32) -> Vec<u8> {
    let reference = BACnetObjectPropertyReference::new(
        oid(ObjectType::ANALOG_VALUE, instance),
        P::PRESENT_VALUE.to_raw(),
    );
    let mut encoded = BytesMut::new();
    encode_object_property_reference(&mut encoded, &reference);
    encoded.to_vec()
}

/// `members` as `user` takes them: as they are, or in the setpoint frame
/// (opening and closing context tag 0).
fn framed(user: &User, members: &[u8]) -> Vec<u8> {
    if user.setpoint {
        [&[0x0E][..], members, &[0x0F]].concat()
    } else {
        members.to_vec()
    }
}

/// A reference to AV-`instance`'s Present_Value as `user` takes it.
fn reference(user: &User, instance: u32) -> Vec<u8> {
    framed(user, &members(instance))
}

/// A reference to AV-2, then `trailing`.
fn followed_by(user: &User, trailing: &[u8]) -> Vec<u8> {
    [reference(user, 2), trailing.to_vec()].concat()
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
        db.add(Box::new(
            PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
        db.add(Box::new(AveragingObject::new(1, "AVG-1").unwrap()))
            .unwrap();
    })
    .await
}

async fn write(h: &mut Harness, user: &User, value: Vec<u8>) -> Result<(), ErrorPdu> {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: user.object,
        property_identifier: user.property,
        property_array_index: None,
        property_value: value,
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    error_response(h).await
}

type Octets = fn(&User) -> Vec<u8>;

/// What every user answers a row. The one exception is no octets, which
/// Setpoint_Reference takes as its value without a reference.
#[derive(Clone, Copy)]
enum Answer {
    /// Taken, and read back as written.
    Stored,
    Refused(ErrorCode),
}

fn rows() -> Vec<(&'static str, Octets, Answer)> {
    use Answer::{Refused, Stored};
    use ErrorCode as E;
    vec![
        ("a reference", |u| reference(u, 3), Stored),
        (
            "a context tag [4] after the reference",
            |u| followed_by(u, &[0x49, 0x01]),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        (
            "an application Unsigned after the reference",
            |u| followed_by(u, &[0x21, 0x01]),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        (
            "a context tag [0] after the reference",
            |u| followed_by(u, &[0x09, 0x01]),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        (
            "a second reference",
            |u| [reference(u, 2), reference(u, 3)].concat(),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        // The object identifier alone: well-formed tags, but no whole
        // reference. Octets cut inside a tag would break the request's own
        // framing instead.
        (
            "an incomplete reference",
            |u| framed(u, &members(2)[..5]),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        (
            "no octets",
            |_| Vec::new(),
            Refused(E::INVALID_DATA_ENCODING),
        ),
        (
            "an application-tagged object identifier",
            |_| vec![0xC4, 0x00, 0x80, 0x00, 0x02],
            Refused(E::INVALID_DATA_TYPE),
        ),
        (
            "a REAL",
            |_| vec![0x44, 0x3F, 0x80, 0x00, 0x00],
            Refused(E::INVALID_DATA_TYPE),
        ),
    ]
}

#[tokio::test(start_paused = true)]
async fn single_references_answer_the_same_octets_the_same_way() {
    let mut h = start().await;
    for user in users() {
        for (what, octets, answer) in rows() {
            let context = format!("{:?} {:?}: {what}", user.object, user.property);
            write(&mut h, &user, reference(&user, 1))
                .await
                .unwrap_or_else(|error| panic!("{context}: setting up: {error:?}"));
            let before = read_wire(&mut h, user.object, user.property, None)
                .await
                .unwrap();
            assert_eq!(before, reference(&user, 1), "{context}: read as written");
            let value = octets(&user);
            let answer = match answer {
                _ if user.setpoint && value.is_empty() => Answer::Stored,
                answer => answer,
            };
            let result = write(&mut h, &user, value.clone()).await;
            let read = read_wire(&mut h, user.object, user.property, None)
                .await
                .unwrap();
            match answer {
                Answer::Stored => {
                    result.unwrap_or_else(|error| panic!("{context}: {error:?}"));
                    assert_eq!(read, value, "{context}: read back as written");
                }
                Answer::Refused(code) => {
                    let error = result.expect_err(&context);
                    assert_eq!(
                        (error.error_class, error.error_code),
                        (ErrorClass::PROPERTY, code),
                        "{context}"
                    );
                    assert_eq!(read, before, "{context}: a refusal changes nothing");
                }
            }
        }
    }
    h.server.stop().await.unwrap();
}

//! The error map's rows that ReadProperty, Who-Is and Who-Has reach, through
//! the `Responder`, with the reply each draws.
use super::*;
use crate::read::NoOverlay;
use bacnet_encoding::apdu::encode_apdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::group::GroupObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::who_has::{IHaveRequest, WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::{DeviceInstanceRange, IAmRequest, WhoIsRequest};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P, RejectReason,
};
use bacnet_types::primitives::PropertyValue;
use std::borrow::Cow;

const DEVICE: u32 = 1234;
const INVOKE: u8 = 7;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A custom object that answers Present_Value whatever index it is given,
/// so only the core's own array-index gate refuses an index on it.
struct Lenient;

impl BACnetObject for Lenient {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(ObjectType::ANALOG_VALUE, 9)
    }
    fn object_name(&self) -> &str {
        "lenient"
    }
    fn read_property(&self, property: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        match property {
            P::PRESENT_VALUE => Ok(PropertyValue::Real(1.0)),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_list(&self) -> Cow<'static, [P]> {
        Cow::Borrowed(&[P::PRESENT_VALUE])
    }
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_name("core-device")
        .with_services(SERVICES)
}

/// A Device, AI-1 (21.5), BV-1, Group-1 of both, and [`Lenient`].
fn database() -> ObjectDatabase {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(21.5);
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let mut group = GroupObject::new(1, "Group-1").unwrap();
    for member in [
        oid(ObjectType::ANALOG_INPUT, 1),
        oid(ObjectType::BINARY_VALUE, 1),
    ] {
        group
            .add_member(ReadAccessSpecification {
                object_identifier: member,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: P::PRESENT_VALUE,
                    property_array_index: None,
                }],
            })
            .unwrap();
    }
    build_database_with_extra(
        &identity(),
        vec![
            Box::new(ai),
            Box::new(bv),
            Box::new(group),
            Box::new(Lenient),
        ],
    )
    .unwrap()
}

fn responder() -> Responder {
    Responder::new(identity(), Limits::default())
}

fn confirmed(service: ConfirmedServiceChoice, body: &[u8], segmented: bool) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: INVOKE,
            sequence_number: segmented.then_some(0),
            proposed_window_size: segmented.then_some(1),
            service_choice: service,
            service_request: Bytes::copy_from_slice(body),
        }),
    )
    .unwrap();
    apdu.freeze()
}

fn read_property(object: ObjectIdentifier, property: P, index: Option<u32>) -> Bytes {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut body);
    confirmed(ConfirmedServiceChoice::READ_PROPERTY, &body, false)
}

fn unconfirmed(service: UnconfirmedServiceChoice, body: &[u8]) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: service,
            service_request: Bytes::copy_from_slice(body),
        }),
    )
    .unwrap();
    apdu.freeze()
}

/// `apdu` through [`Responder::handle`], with what it wrote.
fn handle_with(responder: &Responder, apdu: Bytes, max_reply: u16) -> (Outcome, Option<Apdu>) {
    let mut db = database();
    let mut out = BytesMut::new();
    let outcome = responder.handle(&mut db, Inbound { apdu, max_reply }, &mut out);
    let reply = (!out.is_empty()).then(|| decode_apdu(out.freeze()).unwrap());
    (outcome, reply)
}

fn handle(apdu: Bytes) -> (Outcome, Option<Apdu>) {
    handle_with(&responder(), apdu, 1476)
}

fn error_of(reply: Option<Apdu>) -> (ErrorClass, ErrorCode) {
    match reply {
        Some(Apdu::Error(error)) => {
            assert_eq!(error.invoke_id, INVOKE);
            assert_eq!(error.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
            (error.error_class, error.error_code)
        }
        other => panic!("expected an Error PDU, got {other:?}"),
    }
}

fn reject_of(reply: Option<Apdu>) -> RejectReason {
    match reply {
        Some(Apdu::Reject(reject)) if reject.invoke_id == INVOKE => reject.reject_reason,
        other => panic!("expected a Reject, got {other:?}"),
    }
}

fn abort_of(reply: Option<Apdu>) -> AbortReason {
    match reply {
        Some(Apdu::Abort(abort)) if abort.invoke_id == INVOKE && abort.sent_by_server => {
            abort.abort_reason
        }
        other => panic!("expected a server Abort, got {other:?}"),
    }
}

#[test]
fn read_property_answers_with_the_value() {
    let (outcome, reply) = handle(read_property(
        oid(ObjectType::ANALOG_INPUT, 1),
        P::PRESENT_VALUE,
        None,
    ));
    assert_eq!(outcome, Outcome::Reply);
    let Some(Apdu::ComplexAck(ack)) = reply else {
        panic!("expected a ComplexACK, got {reply:?}");
    };
    assert_eq!(
        (ack.invoke_id, ack.service_choice),
        (INVOKE, ConfirmedServiceChoice::READ_PROPERTY)
    );
    let ack = bacnet_services::read_property::ReadPropertyACK::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    // Application-tagged REAL 21.5.
    assert_eq!(ack.property_value, [0x44, 0x41, 0xAC, 0x00, 0x00]);
}

#[test]
fn the_device_wildcard_reads_the_frozen_device() {
    let (_, reply) = handle(read_property(
        oid(ObjectType::DEVICE, 4_194_303),
        P::OBJECT_NAME,
        None,
    ));
    let Some(Apdu::ComplexAck(ack)) = reply else {
        panic!("expected a ComplexACK, got {reply:?}");
    };
    let ack = bacnet_services::read_property::ReadPropertyACK::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.object_identifier, oid(ObjectType::DEVICE, DEVICE));
}

/// Service parameters that don't decode: a Reject with the decoder's
/// reason (Clauses 18.9 and 20.1.8).
#[test]
fn malformed_parameters_are_rejected() {
    // The object identifier, then nothing: no property identifier.
    let body = [0x0C, 0x00, 0x00, 0x00, 0x01];
    let (outcome, reply) = handle(confirmed(
        ConfirmedServiceChoice::READ_PROPERTY,
        &body,
        false,
    ));
    assert_eq!(outcome, Outcome::Reply);
    assert_eq!(reject_of(reply), RejectReason::MISSING_REQUIRED_PARAMETER);
}

/// A refused object, property or index: an Error PDU (Clause 20.1.7).
#[test]
fn refused_reads_are_error_pdus() {
    let cases = [
        (
            oid(ObjectType::ANALOG_INPUT, 99),
            P::PRESENT_VALUE,
            None,
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            oid(ObjectType::ANALOG_INPUT, 1),
            P::from_raw(9999),
            None,
            (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
        ),
        (
            oid(ObjectType::ANALOG_INPUT, 1),
            P::PRESENT_VALUE,
            Some(1),
            (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        // Only the core's gate refuses this one: the object would answer.
        (
            oid(ObjectType::ANALOG_VALUE, 9),
            P::PRESENT_VALUE,
            Some(1),
            (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            oid(ObjectType::DEVICE, DEVICE),
            P::OBJECT_LIST,
            Some(99),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX),
        ),
    ];
    for (object, property, index, expected) in cases {
        let (outcome, reply) = handle(read_property(object, property, index));
        assert_eq!(outcome, Outcome::Reply);
        assert_eq!(
            error_of(reply),
            expected,
            "{object:?} {property:?} {index:?}"
        );
    }
}

/// A Group's Present_Value past the work limit: an Abort with
/// OUT_OF_RESOURCES, before any member is read (Clause 18.10).
#[test]
fn a_read_past_the_work_limit_is_aborted() {
    let group = read_property(oid(ObjectType::GROUP, 1), P::PRESENT_VALUE, None);
    let tight = Limits {
        read_work: 2,
        ..Limits::default()
    };
    let (outcome, reply) = handle_with(&Responder::new(identity(), tight), group.clone(), 1476);
    assert_eq!(outcome, Outcome::Reply);
    assert_eq!(abort_of(reply), AbortReason::OUT_OF_RESOURCES);
    // Its own row and two member rows fit a limit of three.
    let enough = Limits {
        read_work: 3,
        ..Limits::default()
    };
    let (_, reply) = handle_with(&Responder::new(identity(), enough), group, 1476);
    assert!(matches!(reply, Some(Apdu::ComplexAck(_))), "{reply:?}");
}

/// A segmented request: an Abort with SEGMENTATION_NOT_SUPPORTED, whatever
/// its service (Clause 18.10).
#[test]
fn a_segmented_request_is_aborted() {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut body);
    for service in [
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY,
    ] {
        let apdu = confirmed(service, &body, true);
        assert_eq!(Responder::classify(&apdu), Access::None);
        let (outcome, reply) = handle(apdu);
        assert_eq!(outcome, Outcome::Reply);
        assert_eq!(abort_of(reply), AbortReason::SEGMENTATION_NOT_SUPPORTED);
    }
}

/// A ComplexACK longer than the requester takes: an Abort with
/// SEGMENTATION_NOT_SUPPORTED (Clause 5.4.5.3).
#[test]
fn a_reply_over_max_reply_is_aborted() {
    let apdu = read_property(oid(ObjectType::DEVICE, DEVICE), P::OBJECT_LIST, None);
    // The ACK takes 37 octets.
    let (_, reply) = handle_with(&responder(), apdu.clone(), 36);
    assert_eq!(abort_of(reply), AbortReason::SEGMENTATION_NOT_SUPPORTED);
    let (_, reply) = handle_with(&responder(), apdu, 37);
    assert!(matches!(reply, Some(Apdu::ComplexAck(_))), "{reply:?}");
}

/// A confirmed service the core doesn't run: `read` leaves it to the host,
/// `handle` rejects it with UNRECOGNIZED_SERVICE (Clause 18.9).
#[test]
fn a_service_the_core_doesnt_run_is_rejected_by_handle_only() {
    let apdu = confirmed(ConfirmedServiceChoice::WRITE_PROPERTY, &[0x0C], false);
    assert_eq!(Responder::classify(&apdu), Access::None);
    let mut out = BytesMut::new();
    let request = Inbound {
        apdu: apdu.clone(),
        max_reply: 1476,
    };
    let outcome = responder().read(&database(), None::<&NoOverlay>, request, &mut out);
    assert_eq!((outcome, out.is_empty()), (Outcome::Unhandled, true));
    let (outcome, reply) = handle(apdu);
    assert_eq!(outcome, Outcome::Reply);
    assert_eq!(reject_of(reply), RejectReason::UNRECOGNIZED_SERVICE);
}

fn who_is(range: Option<(u32, u32)>) -> Bytes {
    let mut body = BytesMut::new();
    WhoIsRequest {
        range: range.map(|(low, high)| DeviceInstanceRange::new(low, high).unwrap()),
    }
    .encode(&mut body);
    unconfirmed(UnconfirmedServiceChoice::WHO_IS, &body)
}

fn who_has(object: WhoHasObject) -> Bytes {
    let mut body = BytesMut::new();
    WhoHasRequest {
        range: None,
        object,
    }
    .encode(&mut body)
    .unwrap();
    unconfirmed(UnconfirmedServiceChoice::WHO_HAS, &body)
}

#[test]
fn who_is_in_range_announces_the_frozen_identity() {
    for range in [None, Some((1000, 2000)), Some((DEVICE, DEVICE))] {
        let apdu = who_is(range);
        assert_eq!(Responder::classify(&apdu), Access::Read);
        let (outcome, reply) = handle(apdu);
        assert_eq!(outcome, Outcome::Announce);
        let Some(Apdu::UnconfirmedRequest(i_am)) = reply else {
            panic!("expected an I-Am, got {reply:?}");
        };
        assert_eq!(i_am.service_choice, UnconfirmedServiceChoice::I_AM);
        let i_am = IAmRequest::decode(&i_am.service_request).unwrap();
        assert_eq!(i_am.object_identifier, oid(ObjectType::DEVICE, DEVICE));
        assert_eq!(i_am.vendor_id, 0x105);
    }
}

/// An unconfirmed request is never refused: out of range, malformed, or
/// naming nothing here, it draws no answer (Clause 18.9).
#[test]
fn unconfirmed_requests_out_of_range_or_malformed_are_silent() {
    let silent = [
        who_is(Some((1, 1000))),
        // A low limit with no high one.
        unconfirmed(UnconfirmedServiceChoice::WHO_IS, &[0x09, 0x01]),
        who_has(WhoHasObject::Identifier(oid(ObjectType::ANALOG_INPUT, 99))),
        who_has(WhoHasObject::Name("nowhere".into())),
        unconfirmed(UnconfirmedServiceChoice::WHO_HAS, &[0xFF]),
        // An unconfirmed service the core doesn't run.
        unconfirmed(UnconfirmedServiceChoice::TIME_SYNCHRONIZATION, &[]),
    ];
    for apdu in silent {
        assert_eq!(handle(apdu.clone()), (Outcome::Silent, None), "{apdu:02X?}");
    }
}

#[test]
fn who_has_by_identifier_and_by_name_names_the_selected_device() {
    for object in [
        WhoHasObject::Identifier(oid(ObjectType::BINARY_VALUE, 1)),
        WhoHasObject::Name("BV-1".into()),
    ] {
        let (outcome, reply) = handle(who_has(object));
        assert_eq!(outcome, Outcome::Announce);
        let Some(Apdu::UnconfirmedRequest(i_have)) = reply else {
            panic!("expected an I-Have, got {reply:?}");
        };
        assert_eq!(i_have.service_choice, UnconfirmedServiceChoice::I_HAVE);
        let i_have = IHaveRequest::decode(&i_have.service_request).unwrap();
        assert_eq!(i_have.device_identifier, oid(ObjectType::DEVICE, DEVICE));
        assert_eq!(i_have.object_identifier, oid(ObjectType::BINARY_VALUE, 1));
        assert_eq!(i_have.object_name, "BV-1");
    }
}

#[test]
fn classify_reads_the_header_only() {
    let read = read_property(oid(ObjectType::ANALOG_INPUT, 1), P::PRESENT_VALUE, None);
    assert_eq!(Responder::classify(&read), Access::Read);
    // The header alone is enough; the parameters are never looked at.
    assert_eq!(Responder::classify(&read[..4]), Access::Read);
    for short in [&[][..], &[0x00], &[0x00, 0x05, INVOKE]] {
        assert_eq!(Responder::classify(short), Access::None);
    }
    // A SimpleACK is no request.
    assert_eq!(Responder::classify(&[0x20, INVOKE, 0x0F]), Access::None);
}

#[test]
fn refuse_and_fit_build_the_refusals() {
    let mut out = BytesMut::new();
    assert_eq!(reply::refuse(&[0x00, 0x05], &mut out), Outcome::Silent);
    assert_eq!(reply::refuse(&[0x10, 0x08], &mut out), Outcome::Silent);
    assert!(out.is_empty());
    let ack = |length| {
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: INVOKE,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::from(vec![0; length]),
        })
    };
    // `fit` measures the ACK as the encoder writes it.
    for length in [46, 47, 48] {
        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &ack(length)).unwrap();
        let fits = encoded.len() <= 50;
        assert_eq!(
            matches!(reply::fit(ack(length), 50), Apdu::ComplexAck(_)),
            fits
        );
    }
    let error = reply::error_apdu(
        INVOKE,
        ConfirmedServiceChoice::READ_PROPERTY,
        &Error::protocol(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
    );
    assert!(matches!(reply::fit(error, 1), Apdu::Error(_)));
}

//! The error map's rows that ReadPropertyMultiple and WritePropertyMultiple
//! reach, through the `Responder`, with the reply each draws and what the
//! writes changed.
use super::*;
use crate::read::NoOverlay;
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::command_source::{CommandDeviceBinding, CommandOrigin};
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::{BACnetAddress, PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::primitives::PropertyValue;

const DEVICE: u32 = 1234;
const INVOKE: u8 = 11;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn ai1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

fn bv(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, instance)
}

/// AI-1, and BV-1 and BV-2, whose Present_Values are writable.
fn database() -> ObjectDatabase {
    let identity = DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_services(SERVICES);
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(21.5);
    let mut extra: Vec<Box<dyn BACnetObject>> = vec![Box::new(ai)];
    for (instance, name) in [(1, "BV-1"), (2, "BV-2")] {
        let bv = BinaryValueObject::with_access(instance, name, PresentValueAccess::Writable);
        extra.push(Box::new(bv.unwrap()));
    }
    build_database_with_extra(&identity, extra).unwrap()
}

fn responder_with(limits: Limits) -> Responder {
    Responder::new(DeviceIdentity::new(DEVICE, 0x105).unwrap(), limits).unwrap()
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

fn rpm_body(specs: &[(ObjectIdentifier, &[P])]) -> BytesMut {
    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: specs
            .iter()
            .map(|(object, properties)| ReadAccessSpecification {
                object_identifier: *object,
                list_of_property_references: properties
                    .iter()
                    .map(|&property_identifier| PropertyReference {
                        property_identifier,
                        property_array_index: None,
                    })
                    .collect(),
            })
            .collect(),
    };
    let mut body = BytesMut::new();
    request.encode(&mut body).unwrap();
    body
}

fn rpm(specs: &[(ObjectIdentifier, &[P])]) -> Bytes {
    let body = rpm_body(specs);
    confirmed(ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, &body, false)
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_property_value(&mut octets, value).unwrap();
    octets.to_vec()
}

/// A WritePropertyMultiple body: each (object, property, value) a
/// specification of its own.
fn wpm_body(writes: &[(ObjectIdentifier, P, PropertyValue)]) -> BytesMut {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: writes
            .iter()
            .map(|(object, property, value)| WriteAccessSpecification {
                object_identifier: *object,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: *property,
                    property_array_index: None,
                    value: encoded(value),
                    priority: None,
                }],
            })
            .collect(),
    };
    let mut body = BytesMut::new();
    request.encode(&mut body).unwrap();
    body
}

fn wpm(body: &[u8]) -> Bytes {
    confirmed(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body, false)
}

/// `apdu` through `handle` on `db`: the outcome, the reply and the objects
/// a write changed.
fn handle(
    responder: &Responder,
    db: &mut ObjectDatabase,
    apdu: Bytes,
    max_reply: u16,
) -> (Outcome, Option<Apdu>, Vec<ObjectIdentifier>) {
    let mut out = BytesMut::new();
    let written = responder
        .clone()
        .handle(db, Inbound::new(apdu, max_reply), &mut out);
    let reply = (!out.is_empty()).then(|| decode_apdu(out.freeze()).unwrap());
    (written.outcome, reply, written.objects)
}

fn rpm_ack(reply: Option<Apdu>) -> ReadPropertyMultipleACK {
    match reply {
        Some(Apdu::ComplexAck(ack))
            if ack.invoke_id == INVOKE
                && ack.service_choice == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE =>
        {
            ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap()
        }
        other => panic!("expected an RPM ComplexACK, got {other:?}"),
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

/// The WritePropertyMultiple Error's class, code and the attempt it names.
fn wpm_error_of(reply: Option<Apdu>) -> (ErrorClass, ErrorCode, ObjectIdentifier, u32) {
    let Some(Apdu::Error(error)) = reply else {
        panic!("expected an Error PDU, got {reply:?}");
    };
    assert_eq!(error.invoke_id, INVOKE);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
    );
    let decoded = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
    let attempt = decoded.first_failed_write_attempt;
    (
        decoded.error_class,
        decoded.error_code,
        attempt.object_identifier,
        attempt.property_identifier,
    )
}

fn present_value(db: &ObjectDatabase, object: ObjectIdentifier) -> PropertyValue {
    db.get(&object)
        .unwrap()
        .read_property(P::PRESENT_VALUE, None)
        .unwrap()
}

#[test]
fn classify_puts_rpm_under_read_and_wpm_under_write() {
    let read = rpm(&[(ai1(), &[P::PRESENT_VALUE])]);
    assert_eq!(Responder::classify(&read), Access::Read);
    let write = wpm(&wpm_body(&[(
        bv(1),
        P::PRESENT_VALUE,
        PropertyValue::Enumerated(1),
    )]));
    assert_eq!(Responder::classify(&write), Access::Write);
}

/// `read` holds the database shared, so it never runs a WPM: the host must
/// take the write guard and call `write`.
#[test]
fn read_leaves_a_wpm_to_the_host() {
    let db = database();
    let write = wpm(&wpm_body(&[(
        bv(1),
        P::PRESENT_VALUE,
        PropertyValue::Enumerated(1),
    )]));
    let mut out = BytesMut::new();
    let responder = responder_with(Limits::default());
    let outcome = responder.read(&db, None::<&NoOverlay>, Inbound::new(write, 1476), &mut out);
    assert_eq!((outcome, out.is_empty()), (Outcome::Unhandled, true));
    assert_eq!(present_value(&db, bv(1)), PropertyValue::Enumerated(0));
}

/// ALL, REQUIRED and OPTIONAL expand against each object's metadata, and an
/// unknown object or property is an error row in the ACK, not a refusal.
#[test]
fn rpm_expands_and_reports_errors_inline() {
    let mut db = database();
    let responder = responder_with(Limits::default());
    let ai = db.get(&ai1()).unwrap();
    let metadata = ai.property_metadata();
    let all = metadata.len() - 1;
    let required = metadata
        .iter()
        .filter(|row| row.is_required() && row.property_identifier != P::PROPERTY_LIST)
        .count();
    let optional = metadata.iter().filter(|row| !row.is_required()).count();
    let unknown = oid(ObjectType::ANALOG_INPUT, 99);
    let apdu = rpm(&[
        (ai1(), &[P::ALL]),
        (ai1(), &[P::REQUIRED]),
        (ai1(), &[P::OPTIONAL]),
        (ai1(), &[P::PRESENT_VALUE, P::from_raw(9999)]),
        (unknown, &[P::PRESENT_VALUE]),
    ]);
    let (outcome, reply, written) = handle(&responder, &mut db, apdu, 1476);
    assert_eq!((outcome, written), (Outcome::Reply, Vec::new()));
    let results = rpm_ack(reply).list_of_read_access_results;
    let counts: Vec<_> = results.iter().map(|r| r.list_of_results.len()).collect();
    assert_eq!(counts, [all, required, optional, 2, 1]);
    let rows = &results[3].list_of_results;
    assert_eq!(
        rows[0].property_value,
        Some(encoded(&PropertyValue::Real(21.5)))
    );
    assert_eq!(
        rows[1].error,
        Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
    assert_eq!(results[4].object_identifier, unknown);
    assert_eq!(
        results[4].list_of_results[0].error,
        Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
    );
}

#[test]
fn rpm_malformed_parameters_are_rejected() {
    let mut db = database();
    let responder = responder_with(Limits::default());
    // An object identifier, then nothing: no list of references.
    let apdu = confirmed(
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &[0x0C, 0x00, 0x00, 0x00, 0x01],
        false,
    );
    let (outcome, reply, _) = handle(&responder, &mut db, apdu, 1476);
    assert_eq!(outcome, Outcome::Reply);
    assert!(
        matches!(reply, Some(Apdu::Reject(ref reject)) if reject.invoke_id == INVOKE),
        "{reply:?}"
    );
}

/// The work limit fails in planning: past it, OUT_OF_RESOURCES; at it, the
/// ACK.
#[test]
fn rpm_past_the_work_limit_is_aborted() {
    let mut db = database();
    let apdu = rpm(&[(ai1(), &[P::PRESENT_VALUE, P::OBJECT_NAME, P::UNITS])]);
    let rows = |max_result_elements| {
        Limits::default().with_rpm(ReadPropertyMultipleBudget {
            max_result_elements,
            ..ReadPropertyMultipleBudget::default()
        })
    };
    let (_, reply, _) = handle(&responder_with(rows(2)), &mut db, apdu.clone(), 1476);
    assert_eq!(abort_of(reply), AbortReason::OUT_OF_RESOURCES);
    let (_, reply, _) = handle(&responder_with(rows(3)), &mut db, apdu, 1476);
    assert_eq!(rpm_ack(reply).list_of_read_access_results.len(), 1);
}

/// The byte limit fails after the reads: an ACK one octet over it is
/// BUFFER_OVERFLOW, one that fits it exactly is sent.
#[test]
fn rpm_past_the_byte_limit_is_aborted() {
    let mut db = database();
    let apdu = rpm(&[(ai1(), &[P::ALL])]);
    let (_, reply, _) = handle(
        &responder_with(Limits::default()),
        &mut db,
        apdu.clone(),
        1476,
    );
    let Some(Apdu::ComplexAck(ack)) = reply else {
        panic!("expected a ComplexACK, got {reply:?}");
    };
    let octets = |max_service_ack_bytes| {
        Limits::default().with_rpm(ReadPropertyMultipleBudget {
            max_service_ack_bytes,
            ..ReadPropertyMultipleBudget::default()
        })
    };
    let exact = ack.service_ack.len();
    let (_, reply, _) = handle(
        &responder_with(octets(exact - 1)),
        &mut db,
        apdu.clone(),
        1476,
    );
    assert_eq!(abort_of(reply), AbortReason::BUFFER_OVERFLOW);
    let (_, reply, _) = handle(&responder_with(octets(exact)), &mut db, apdu, 1476);
    assert!(matches!(reply, Some(Apdu::ComplexAck(_))), "{reply:?}");
}

/// Segmented either way: a segmented request, and an ACK longer than the
/// requester takes, are aborted with SEGMENTATION_NOT_SUPPORTED.
#[test]
fn rpm_segmentation_is_refused() {
    let mut db = database();
    let responder = responder_with(Limits::default());
    let body = rpm_body(&[(ai1(), &[P::PRESENT_VALUE])]);
    let segmented = confirmed(ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, &body, true);
    let (_, reply, _) = handle(&responder, &mut db, segmented, 1476);
    assert_eq!(abort_of(reply), AbortReason::SEGMENTATION_NOT_SUPPORTED);
    let (_, reply, _) = handle(&responder, &mut db, rpm(&[(ai1(), &[P::ALL])]), 50);
    assert_eq!(abort_of(reply), AbortReason::SEGMENTATION_NOT_SUPPORTED);
}

#[test]
fn new_refuses_a_zero_rpm_budget() {
    let identity = || DeviceIdentity::new(DEVICE, 0x105).unwrap();
    for budget in [
        ReadPropertyMultipleBudget {
            max_result_elements: 0,
            ..ReadPropertyMultipleBudget::default()
        },
        ReadPropertyMultipleBudget {
            max_service_ack_bytes: 0,
            ..ReadPropertyMultipleBudget::default()
        },
    ] {
        assert!(Responder::new(identity(), Limits::default().with_rpm(budget)).is_err());
    }
}

/// Every attempt succeeds: a SimpleACK, and the objects changed.
#[test]
fn wpm_writes_every_attempt() {
    let mut db = database();
    let responder = responder_with(Limits::default());
    let active = PropertyValue::Enumerated(1);
    let apdu = wpm(&wpm_body(&[
        (bv(1), P::PRESENT_VALUE, active.clone()),
        (bv(2), P::PRESENT_VALUE, active.clone()),
    ]));
    let (outcome, reply, written) = handle(&responder, &mut db, apdu, 1476);
    assert_eq!((outcome, written), (Outcome::Reply, vec![bv(1), bv(2)]));
    assert!(
        matches!(reply, Some(Apdu::SimpleAck(ref ack))
            if ack.invoke_id == INVOKE
                && ack.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE),
        "{reply:?}"
    );
    assert_eq!(present_value(&db, bv(2)), active);
}

/// A commandable Present_Value through WritePropertyMultiple: refused with
/// no requester named, commanded (at priority 16, the request naming none)
/// for the request's source.
#[test]
fn wpm_commands_a_commandable_present_value_for_the_source() {
    let mut db = database();
    db.add(Box::new(BinaryValueObject::new(3, "BV-3").unwrap()))
        .unwrap();
    let mut responder = responder_with(Limits::default());
    let active = PropertyValue::Enumerated(1);
    let body = wpm_body(&[(bv(3), P::PRESENT_VALUE, active.clone())]);
    let (_, reply, written) = handle(&responder, &mut db, wpm(&body), 1476);
    assert!(written.is_empty());
    let refused = wpm_error_of(reply);
    assert_eq!(refused.1, ErrorCode::WRITE_ACCESS_DENIED);

    let source = CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number: 0,
            mac_address: [192, 168, 1, 7, 0xBA, 0xC0][..].into(),
        },
        binding: CommandDeviceBinding::Unknown,
    };
    let mut out = BytesMut::new();
    let request = Inbound::new(wpm(&body), 1476).with_source(source);
    let handled = responder.handle(&mut db, request, &mut out);
    assert_eq!(handled.objects, [bv(3)]);
    assert!(
        matches!(decode_apdu(out.freeze()), Ok(Apdu::SimpleAck(_))),
        "the commanded write is acknowledged"
    );
    assert_eq!(present_value(&db, bv(3)), active);
    let slot = db.get(&bv(3)).unwrap();
    let slot = slot.read_property(P::PRIORITY_ARRAY, Some(16)).unwrap();
    assert_eq!(slot, active);
}

/// A refused attempt after a successful one: the Error names the refused
/// attempt, and the one before it stays written.
#[test]
fn wpm_keeps_the_prefix_before_a_refused_attempt() {
    let mut db = database();
    let responder = responder_with(Limits::default());
    let active = PropertyValue::Enumerated(1);
    let apdu = wpm(&wpm_body(&[
        (bv(1), P::PRESENT_VALUE, active.clone()),
        (bv(9), P::PRESENT_VALUE, active.clone()),
    ]));
    let (_, reply, written) = handle(&responder, &mut db, apdu, 1476);
    assert_eq!(written, [bv(1)]);
    assert_eq!(
        wpm_error_of(reply),
        (
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
            bv(9),
            P::PRESENT_VALUE.to_raw()
        )
    );
    assert_eq!(present_value(&db, bv(1)), active);
}

/// A syntax fault before any attempt succeeded is a Reject; after one
/// succeeded, the request has changed the device, so it is an Error,
/// SERVICES / INVALID_TAG, naming the attempt it reached (Clauses 15.10 and
/// 20.1.8), and the prefix stays written.
#[test]
fn wpm_syntax_fault_rejects_until_an_attempt_succeeds() {
    let responder = responder_with(Limits::default());
    let active = PropertyValue::Enumerated(1);
    let good = wpm_body(&[(bv(1), P::PRESENT_VALUE, active.clone())]);
    let second = wpm_body(&[(bv(2), P::PRESENT_VALUE, active.clone())]);
    // The second specification, cut off inside its value.
    let mut torn = good.to_vec();
    torn.extend_from_slice(&second[..second.len() - 2]);

    let mut db = database();
    let (_, reply, written) = handle(&responder, &mut db, wpm(&second[..second.len() - 2]), 1476);
    assert!(written.is_empty());
    assert!(
        matches!(reply, Some(Apdu::Reject(ref reject)) if reject.invoke_id == INVOKE),
        "{reply:?}"
    );
    assert_eq!(present_value(&db, bv(2)), PropertyValue::Enumerated(0));

    let (_, reply, written) = handle(&responder, &mut db, wpm(&torn), 1476);
    assert_eq!(written, [bv(1)]);
    let (class, code, object, _) = wpm_error_of(reply);
    assert_eq!(
        (class, code, object),
        (ErrorClass::SERVICES, ErrorCode::INVALID_TAG, bv(2))
    );
    assert_eq!(present_value(&db, bv(1)), active);
    assert_eq!(present_value(&db, bv(2)), PropertyValue::Enumerated(0));
}

#[test]
fn wpm_segmented_request_is_aborted() {
    let mut db = database();
    let responder = responder_with(Limits::default());
    let body = wpm_body(&[(bv(1), P::PRESENT_VALUE, PropertyValue::Enumerated(1))]);
    let segmented = confirmed(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, &body, true);
    let (_, reply, written) = handle(&responder, &mut db, segmented, 1476);
    assert!(written.is_empty());
    assert_eq!(abort_of(reply), AbortReason::SEGMENTATION_NOT_SUPPORTED);
    assert_eq!(present_value(&db, bv(1)), PropertyValue::Enumerated(0));
}

//! The error map's rows that WriteProperty reaches, through the
//! `Responder`, with the reply each draws and what the write changed.
use super::*;
use crate::write::WriteTarget;
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyWriteCapability,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P, RejectReason,
};
use bacnet_types::primitives::PropertyValue;
use std::borrow::Cow;

const DEVICE: u32 = 1234;
const INVOKE: u8 = 9;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn bv1() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 1)
}

fn bv2() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 2)
}

fn cmd1() -> ObjectIdentifier {
    oid(ObjectType::COMMAND, 1)
}

/// A custom Device-typed object, instance 77, whose writer takes every
/// value, so only the core's executor-owned gate refuses a write to it.
struct Permissive;

impl BACnetObject for Permissive {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(ObjectType::DEVICE, 77)
    }
    fn object_name(&self) -> &str {
        "permissive"
    }
    fn read_property(&self, _: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        Ok(PropertyValue::Null)
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        const ROWS: &[PropertyMetadata] = &[PropertyMetadata::new(
            P::DESCRIPTION,
            PropertyConformance::Optional,
            None,
            PropertyWriteCapability::Always,
        )];
        Cow::Borrowed(ROWS)
    }
    fn property_list(&self) -> Cow<'static, [P]> {
        Cow::Borrowed(&[P::DESCRIPTION])
    }
}

/// A Device, AI-1, BV-1 (a writable Present_Value), BV-2 (a commandable
/// one), CMD-1, whose list 1 writes BV-1 active at priority 8, and
/// [`Permissive`].
fn database() -> ObjectDatabase {
    let identity = DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_services(SERVICES);
    let ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let bv = BinaryValueObject::with_access(1, "BV-1", PresentValueAccess::Writable).unwrap();
    let commanded = BinaryValueObject::new(2, "BV-2").unwrap();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![BACnetActionCommand {
                device_identifier: None,
                object_identifier: bv1(),
                property_identifier: P::PRESENT_VALUE,
                property_array_index: None,
                property_value: PropertyValue::Enumerated(1),
                priority: Some(8),
                post_delay: None,
                quit_on_failure: false,
                write_successful: true,
            }],
        }])
        .unwrap();
    build_database_with_extra(
        &identity,
        vec![
            Box::new(ai),
            Box::new(bv),
            Box::new(commanded),
            Box::new(command),
            Box::new(Permissive),
        ],
    )
    .unwrap()
}

fn responder() -> Responder {
    let identity = DeviceIdentity::new(DEVICE, 0x105).unwrap();
    Responder::new(identity, Limits::default()).unwrap()
}

fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut octets = BytesMut::new();
    encode_property_value(&mut octets, value).unwrap();
    octets.to_vec()
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

fn write_property(
    object: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: &[u8],
) -> Bytes {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: index,
        property_value: value.to_vec(),
        priority: Some(8),
    }
    .encode(&mut body)
    .unwrap();
    confirmed(ConfirmedServiceChoice::WRITE_PROPERTY, &body, false)
}

/// `apdu` through [`Responder::handle`] on `db`: the outcome, what it wrote
/// and the objects the write changed.
fn handle(db: &mut ObjectDatabase, apdu: Bytes) -> (Outcome, Option<Apdu>, Vec<ObjectIdentifier>) {
    let mut out = BytesMut::new();
    let written = responder().handle(db, Inbound::new(apdu, 1476), &mut out);
    let reply = (!out.is_empty()).then(|| decode_apdu(out.freeze()).unwrap());
    (written.outcome, reply, written.objects)
}

/// `property` of `object`, read through the responder.
fn read(db: &mut ObjectDatabase, object: ObjectIdentifier, property: P) -> PropertyValue {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut body);
    let apdu = confirmed(ConfirmedServiceChoice::READ_PROPERTY, &body, false);
    let (_, reply, _) = handle(db, apdu);
    let Some(Apdu::ComplexAck(ack)) = reply else {
        panic!("expected a ComplexACK, got {reply:?}");
    };
    let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    let (value, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    value
}

fn is_simple_ack(reply: &Option<Apdu>) -> bool {
    matches!(reply, Some(Apdu::SimpleAck(ack))
        if ack.invoke_id == INVOKE && ack.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY)
}

fn error_of(reply: Option<Apdu>) -> (ErrorClass, ErrorCode) {
    match reply {
        Some(Apdu::Error(error)) => {
            assert_eq!(error.invoke_id, INVOKE);
            assert_eq!(error.service_choice, ConfirmedServiceChoice::WRITE_PROPERTY);
            (error.error_class, error.error_code)
        }
        other => panic!("expected an Error PDU, got {other:?}"),
    }
}

#[test]
fn classify_names_a_write_property() {
    let apdu = write_property(bv1(), P::PRESENT_VALUE, None, &[0x91, 0x01]);
    assert_eq!(Responder::classify(&apdu), Access::Write);
    assert_eq!(Responder::classify(&apdu[..4]), Access::Write);
    let segmented = confirmed(ConfirmedServiceChoice::WRITE_PROPERTY, &apdu[4..], true);
    assert_eq!(Responder::classify(&segmented), Access::None);
}

#[test]
fn a_write_is_acknowledged_and_reads_back() {
    let mut db = database();
    let apdu = write_property(bv1(), P::PRESENT_VALUE, None, &[0x91, 0x01]);
    let (outcome, reply, objects) = handle(&mut db, apdu);
    assert_eq!(outcome, Outcome::Reply);
    assert!(is_simple_ack(&reply), "{reply:?}");
    assert_eq!(objects, [bv1()]);
    assert_eq!(
        read(&mut db, bv1(), P::PRESENT_VALUE),
        PropertyValue::Enumerated(1)
    );
}

/// Parameters that don't decode are rejected before anything runs (Clause
/// 20.1.8), and nothing is written.
#[test]
fn malformed_write_parameters_are_rejected() {
    let mut db = database();
    // The object and property, then no value.
    let body = [0x0C, 0x01, 0x40, 0x00, 0x01, 0x19, 0x55];
    let apdu = confirmed(ConfirmedServiceChoice::WRITE_PROPERTY, &body, false);
    let (outcome, reply, objects) = handle(&mut db, apdu);
    assert_eq!(outcome, Outcome::Reply);
    assert!(
        matches!(reply, Some(Apdu::Reject(ref reject))
            if reject.invoke_id == INVOKE
                && reject.reject_reason == RejectReason::MISSING_REQUIRED_PARAMETER),
        "{reply:?}"
    );
    assert!(objects.is_empty());
}

/// A refused object, property, index or value is an Error PDU, and nothing
/// is written.
#[test]
fn refused_writes_are_error_pdus() {
    let device = oid(ObjectType::DEVICE, DEVICE);
    let active = [0x91, 0x01];
    let rows: [(&str, Bytes, (ErrorClass, ErrorCode)); 7] = [
        (
            "unknown object",
            write_property(
                oid(ObjectType::BINARY_VALUE, 9),
                P::PRESENT_VALUE,
                None,
                &active,
            ),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            "executor-owned Device property",
            write_property(
                device,
                P::PROTOCOL_SERVICES_SUPPORTED,
                None,
                &encoded(&PropertyValue::BitString {
                    unused_bits: 7,
                    data: vec![0; 6],
                }),
            ),
            (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
        ),
        (
            "Device identity property",
            write_property(device, P::VENDOR_IDENTIFIER, None, &[0x21, 0x07]),
            (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
        ),
        (
            "index on a non-array",
            write_property(bv1(), P::PRESENT_VALUE, Some(1), &active),
            (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            "wrong datatype",
            write_property(bv1(), P::PRESENT_VALUE, None, &[0x44, 0, 0, 0, 0]),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE),
        ),
        (
            "value that doesn't decode",
            // Application tag 13, which no datatype has.
            write_property(bv1(), P::PRESENT_VALUE, None, &[0xD1, 0x00]),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_ENCODING),
        ),
        (
            "duplicate Object_Name",
            write_property(
                bv1(),
                P::OBJECT_NAME,
                None,
                &encoded(&PropertyValue::CharacterString("AI-1".into())),
            ),
            (ErrorClass::PROPERTY, ErrorCode::DUPLICATE_NAME),
        ),
    ];
    for (what, apdu, expected) in rows {
        let mut db = database();
        let (outcome, reply, objects) = handle(&mut db, apdu);
        assert_eq!(outcome, Outcome::Reply, "{what}");
        assert_eq!(error_of(reply), expected, "{what}");
        assert!(objects.is_empty(), "{what}");
    }
    let mut db = database();
    assert_eq!(
        read(&mut db, bv1(), P::OBJECT_NAME),
        PropertyValue::CharacterString("BV-1".into())
    );
}

/// A NULL to a property that isn't commandable and has no NULL in its
/// datatype succeeds and changes nothing (`write::relinquish`).
#[test]
fn a_null_left_unchanged_is_acknowledged_with_nothing_written() {
    let mut db = database();
    let before = read(&mut db, bv1(), P::OBJECT_NAME);
    let apdu = write_property(bv1(), P::OBJECT_NAME, None, &[0x00]);
    let (outcome, reply, objects) = handle(&mut db, apdu);
    assert_eq!(outcome, Outcome::Reply);
    assert!(is_simple_ack(&reply), "{reply:?}");
    assert!(objects.is_empty());
    assert_eq!(read(&mut db, bv1(), P::OBJECT_NAME), before);
}

/// The executor serves its own Device properties, so no write reaches a
/// Device's writer for them, even one that would take it; any other
/// property goes to the writer.
#[test]
fn executor_owned_device_properties_are_never_written() {
    let permissive = oid(ObjectType::DEVICE, 77);
    let unsigned = encoded(&PropertyValue::Unsigned(7));
    for property in [
        P::PROTOCOL_SERVICES_SUPPORTED,
        P::PROPERTY_LIST,
        P::ACTIVE_COV_SUBSCRIPTIONS,
        P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS,
        P::DEVICE_ADDRESS_BINDING,
        P::OBJECT_IDENTIFIER,
        P::VENDOR_IDENTIFIER,
        P::MAX_APDU_LENGTH_ACCEPTED,
        P::SEGMENTATION_SUPPORTED,
        P::APDU_SEGMENT_TIMEOUT,
        P::MAX_SEGMENTS_ACCEPTED,
    ] {
        let mut db = database();
        let apdu = write_property(permissive, property, None, &unsigned);
        let (_, reply, objects) = handle(&mut db, apdu);
        assert_eq!(
            error_of(reply),
            (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
            "{property:?}"
        );
        assert!(objects.is_empty());
    }
    let mut db = database();
    let text = encoded(&PropertyValue::CharacterString("taken".into()));
    let apdu = write_property(permissive, P::DESCRIPTION, None, &text);
    let (_, reply, objects) = handle(&mut db, apdu);
    assert!(is_simple_ack(&reply), "{reply:?}");
    assert_eq!(objects, [permissive]);
}

/// A commandable Present_Value records who commanded it (Clause 19.5), and
/// the core has no requester to name: `Inbound` carries no source address.
/// The object refuses an unsourced command, as it does the bare handler's.
#[test]
fn a_commandable_present_value_is_refused_without_a_command_origin() {
    let mut db = database();
    let apdu = write_property(bv2(), P::PRESENT_VALUE, None, &[0x91, 0x01]);
    let (outcome, reply, objects) = handle(&mut db, apdu);
    assert_eq!(outcome, Outcome::Reply);
    assert_eq!(
        error_of(reply),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert!(objects.is_empty());
}

/// The core has no task to make a Command's writes in, so the run a
/// Present_Value write starts ends at once, unmade (#1178).
#[test]
fn a_command_present_value_write_ends_its_run_unmade() {
    let mut db = database();
    let apdu = write_property(cmd1(), P::PRESENT_VALUE, None, &[0x21, 0x01]);
    let (outcome, reply, objects) = handle(&mut db, apdu);
    assert_eq!(outcome, Outcome::Reply);
    assert!(is_simple_ack(&reply), "{reply:?}");
    assert_eq!(objects, [cmd1()]);
    assert_eq!(
        read(&mut db, cmd1(), P::IN_PROCESS),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(&mut db, cmd1(), P::ALL_WRITES_SUCCESSFUL),
        PropertyValue::Boolean(false)
    );
    // BV-1 was never written.
    assert_eq!(
        read(&mut db, bv1(), P::PRESENT_VALUE),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn a_segmented_write_is_aborted() {
    let mut db = database();
    let body = write_property(bv1(), P::PRESENT_VALUE, None, &[0x91, 0x01]);
    let apdu = confirmed(ConfirmedServiceChoice::WRITE_PROPERTY, &body[4..], true);
    let mut out = BytesMut::new();
    let written = responder().write(&mut db, None, Inbound::new(apdu.clone(), 1476), &mut out);
    assert_eq!(written, Written::from(Outcome::Reply));
    let (outcome, reply, _) = handle(&mut db, apdu);
    assert_eq!(outcome, Outcome::Reply);
    assert!(
        matches!(reply, Some(Apdu::Abort(ref abort))
            if abort.invoke_id == INVOKE
                && abort.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED),
        "{reply:?}"
    );
    assert_eq!(
        read(&mut db, bv1(), P::PRESENT_VALUE),
        PropertyValue::Enumerated(0)
    );
}

/// `write` runs WriteProperty alone and `read` never runs it: each leaves
/// the other's services to the host.
#[test]
fn write_and_read_leave_each_others_services_to_the_host() {
    let mut db = database();
    let mut out = BytesMut::new();
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: bv1(),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut body);
    let read = confirmed(ConfirmedServiceChoice::READ_PROPERTY, &body, false);
    let written = responder().write(&mut db, None, Inbound::new(read, 1476), &mut out);
    assert_eq!(written, Written::from(Outcome::Unhandled));
    let write = write_property(bv1(), P::PRESENT_VALUE, None, &[0x91, 0x01]);
    let request = Inbound::new(write, 1476);
    let outcome = responder().read(&db, None::<&read::NoOverlay>, request, &mut out);
    assert_eq!(outcome, Outcome::Unhandled);
    assert!(out.is_empty());
}

/// What the observer saw, in order.
#[derive(Default)]
struct Seen(Vec<&'static str>);

impl WriteCommitObserver for Seen {
    fn before(&mut self, _: &ObjectDatabase, write: WriteTarget<'_>) {
        assert_eq!(write.oid, bv1());
        self.0.push("before");
    }
    fn committed(&mut self, _: &mut ObjectDatabase) {
        self.0.push("committed");
    }
    fn failed(&mut self, _: &mut ObjectDatabase, _: &Error) {
        self.0.push("failed");
    }
    fn applied(&mut self, _: &ObjectDatabase, oid: ObjectIdentifier) {
        assert_eq!(oid, bv1());
        self.0.push("applied");
    }
}

#[test]
fn the_observer_sees_each_attempt() {
    for (value, expected) in [
        (&[0x91, 0x01][..], &["before", "committed", "applied"][..]),
        (&[0x44, 0, 0, 0, 0][..], &["before", "failed"][..]),
    ] {
        let mut db = database();
        let mut seen = Seen::default();
        let mut out = BytesMut::new();
        let apdu = write_property(bv1(), P::PRESENT_VALUE, None, value);
        responder().write(&mut db, Some(&mut seen), Inbound::new(apdu, 1476), &mut out);
        assert_eq!(seen.0, expected);
    }
}

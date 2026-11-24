//! The inventory tests' lab: an in-process device over a loopback link
//! whose client side can refuse RPM, fail it, refuse the whole
//! Object_List, go silent after so many requests or raise a stop flag,
//! and whose sends are logged. The Standard 223 export's tests read
//! through it too.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_objects::value_types::IntegerValueObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SendMode, StartMode, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::Bytes;

use crate::client::Client;
use crate::limits::RequestLimits;

pub(crate) const LAB: u32 = 2436;

pub(crate) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// What the client's side of the lab link does to requests.
#[derive(Clone)]
pub(crate) struct Faults {
    /// Requests passed on before the link goes silent.
    pub(crate) pass: usize,
    /// Answer each ReadPropertyMultiple with a Reject, as a device
    /// without RPM does.
    pub(crate) reject_rpm: bool,
    /// Answer a ReadProperty of the whole Object_List with a Reject, so
    /// the list is read element by element.
    pub(crate) reject_whole_list: bool,
    /// Answer each ReadPropertyMultiple with an Error PDU (PROPERTY,
    /// UNKNOWN_PROPERTY), as a device does that fails the whole request
    /// over one property it lacks.
    pub(crate) error_rpm: bool,
    /// Set this flag as the given request goes out (1 for the first).
    pub(crate) cancel: Option<(usize, Arc<AtomicBool>)>,
    /// Answer each ReadProperty of this object with an Error PDU (OBJECT,
    /// UNKNOWN_OBJECT), as a device does for an object its list names and
    /// it no longer has.
    pub(crate) unknown_object: Option<ObjectIdentifier>,
    /// Answer each ReadPropertyMultiple whose service request is longer
    /// than this many octets with an Abort (BUFFER_OVERFLOW), as a device
    /// does whose answer would not fit.
    pub(crate) abort_longer: Option<usize>,
    /// Answer each ReadPropertyMultiple of several objects from a script
    /// instead of the device.
    pub(crate) scripted: Option<Scripted>,
    /// Answer a ReadProperty of the whole Object_List with these objects.
    pub(crate) listed: Option<Vec<ObjectIdentifier>>,
}

/// How a scripted ReadPropertyMultiple answer differs from the request:
/// each object's Object_Name is its label (`analog-input 1`), its
/// Present_Value a REAL of its instance number, its Units degrees Celsius
/// (62), and every other property an unknown-property error.
#[derive(Clone, Copy)]
pub(crate) enum Scripted {
    /// The objects in the reverse of the order asked.
    Reversed,
    /// The answer leaves this object out.
    Without(ObjectIdentifier),
    /// The first and the last object's results change places.
    EndsSwapped,
    /// A result for this object, which nobody asked for, comes first.
    Extra(ObjectIdentifier),
}

impl Default for Faults {
    fn default() -> Self {
        Self {
            pass: usize::MAX,
            reject_rpm: false,
            reject_whole_list: false,
            error_rpm: false,
            cancel: None,
            unknown_object: None,
            abort_longer: None,
            scripted: None,
            listed: None,
        }
    }
}

/// A local unsegmented confirmed request's invoke ID, service and service
/// request.
pub(crate) fn request(npdu: &[u8]) -> Option<(u8, u8, &[u8])> {
    match npdu {
        [0x01, control, apdu, _, invoke_id, service, rest @ ..]
            if control & 0x28 == 0 && apdu & 0xF8 == 0x00 =>
        {
            Some((*invoke_id, *service, rest))
        }
        _ => None,
    }
}

/// The device's answer the faults put in place of `npdu`'s, if any.
fn refusal(faults: &Faults, npdu: &[u8]) -> Option<Vec<u8>> {
    let (invoke_id, service, rest) = request(npdu)?;
    // RPM is service 14; ReadProperty 12, here of Object_List (76)
    // with no array index.
    if faults.error_rpm && service == 14 {
        // An Error PDU: class PROPERTY (2), code UNKNOWN_PROPERTY (32).
        return Some(vec![0x01, 0x00, 0x50, invoke_id, 14, 0x91, 2, 0x91, 32]);
    }
    if let Some(object) = faults.unknown_object
        && service == 12
        && rest.get(1..5) == Some(&object_octets(object)[..])
    {
        // An Error PDU: class OBJECT (1), code UNKNOWN_OBJECT (31).
        return Some(vec![0x01, 0x00, 0x50, invoke_id, 12, 0x91, 1, 0x91, 31]);
    }
    if service == 14 && faults.abort_longer.is_some_and(|most| rest.len() > most) {
        // An Abort PDU from the server: BUFFER_OVERFLOW (1).
        return Some(vec![0x01, 0x00, 0x71, invoke_id, 1]);
    }
    if let Some(script) = faults.scripted
        && service == 14
        && let Some(answer) = scripted(script, rest)
    {
        // A ComplexACK PDU.
        return Some([&[0x01, 0x00, 0x30, invoke_id, 14][..], &answer].concat());
    }
    if let Some(objects) = &faults.listed
        && service == 12
        && matches!(rest, [0x0C, _, _, _, _, 0x19, 0x4C])
    {
        // A ComplexACK PDU: the request's object and property, then the
        // list between the tags of the value.
        let mut answer = vec![0x01, 0x00, 0x30, invoke_id, 12];
        answer.extend_from_slice(rest);
        answer.push(0x3E);
        for object in objects {
            answer.push(0xC4);
            answer.extend(object_octets(*object));
        }
        answer.push(0x3F);
        return Some(answer);
    }
    let rpm = faults.reject_rpm && service == 14;
    let whole_list =
        faults.reject_whole_list && service == 12 && matches!(rest, [0x0C, _, _, _, _, 0x19, 0x4C]);
    // A Reject PDU for UNRECOGNIZED_SERVICE (9).
    (rpm || whole_list).then(|| vec![0x01, 0x00, 0x60, invoke_id, 9])
}

/// The scripted answer to the ReadPropertyMultiple request `rest`, when
/// it asks for several objects.
fn scripted(script: Scripted, rest: &[u8]) -> Option<Vec<u8>> {
    use bacnet_services::rpm::{
        ReadAccessResult, ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
    };
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let request = ReadPropertyMultipleRequest::decode(rest).ok()?;
    if request.list_of_read_access_specs.len() < 2 {
        return None;
    }
    let mut results: Vec<ReadAccessResult> = request
        .list_of_read_access_specs
        .into_iter()
        .filter(|spec| !matches!(script, Scripted::Without(gone) if gone == spec.object_identifier))
        .map(|spec| ReadAccessResult {
            object_identifier: spec.object_identifier,
            list_of_results: spec
                .list_of_property_references
                .into_iter()
                .map(|reference| {
                    let property = reference.property_identifier;
                    let object = spec.object_identifier;
                    let value = match property {
                        PropertyIdentifier::OBJECT_NAME => {
                            Some(text(&crate::label::object_label(object)))
                        }
                        PropertyIdentifier::PRESENT_VALUE => {
                            Some(PropertyValue::Real(object.instance_number() as f32))
                        }
                        PropertyIdentifier::UNITS => Some(PropertyValue::Enumerated(62)),
                        _ => None,
                    };
                    let unknown = (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY);
                    ReadResultElement {
                        property_identifier: property,
                        property_array_index: None,
                        error: value.is_none().then_some(unknown),
                        property_value: value.and_then(|value| row(property, value).1.ok()),
                    }
                })
                .collect(),
        })
        .collect();
    match script {
        Scripted::Reversed => results.reverse(),
        Scripted::EndsSwapped => {
            let last = results.len() - 1;
            results.swap(0, last);
        }
        Scripted::Extra(object) => {
            let pv = PropertyIdentifier::PRESENT_VALUE;
            let element = ReadResultElement {
                property_identifier: pv,
                property_array_index: None,
                property_value: row(pv, PropertyValue::Real(99.0)).1.ok(),
                error: None,
            };
            let unasked = ReadAccessResult {
                object_identifier: object,
                list_of_results: vec![element],
            };
            results.insert(0, unasked);
        }
        Scripted::Without(_) => {}
    }
    let mut out = bytes::BytesMut::new();
    let ack = ReadPropertyMultipleACK {
        list_of_read_access_results: results,
    };
    ack.encode(&mut out).ok()?;
    Some(out.to_vec())
}

/// `object` as an object identifier's four octets.
fn object_octets(object: ObjectIdentifier) -> [u8; 4] {
    ((object.object_type().to_raw() << 22) | object.instance_number()).to_be_bytes()
}

/// The client's side of the lab link: `inner`, relayed through a channel the
/// link feeds the Rejects and Errors of `faults` into.
fn link(inner: LoopbackTransport, faults: Faults) -> (TestTransport, SendLog) {
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .start(StartMode::Relay(256));
    let inbox = builder.handle();
    let log = builder.sent();
    let sent = Arc::new(AtomicUsize::new(0));
    let faults = Arc::new(faults);
    let port = builder
        .intercept(move |frame| {
            let (faults, inbox) = (Arc::clone(&faults), inbox.clone());
            let count = (!frame.broadcast).then(|| sent.fetch_add(1, Ordering::Relaxed) + 1);
            async move {
                let Some(sent) = count else {
                    return true;
                };
                if let Some((after, flag)) = &faults.cancel
                    && sent >= *after
                {
                    flag.store(true, Ordering::Relaxed);
                }
                if sent > faults.pass {
                    return false;
                }
                let Some(answer) = refusal(&faults, &frame.npdu) else {
                    return true;
                };
                let reject = Bytes::from(answer);
                if let Some(inbox) = inbox.inbound() {
                    let source = MacAddr::from_slice(&frame.mac);
                    let back = ReceivedNpdu::unverified(reject, source, false, Vec::new(), None);
                    let _ = inbox.send(back).await;
                }
                false
            }
        })
        .build();
    (port, log)
}

pub(crate) fn text(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

/// One object of each exported type (two analog values: one commandable,
/// one writable), a multi-state value with state texts, a writable integer
/// value in kilowatt-hours, and a trend log.
pub(crate) fn objects() -> Vec<Box<dyn BACnetObject>> {
    let mut ai =
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        text("Room 101"),
        None,
    )
    .unwrap();
    let mut msv = MultiStateValueObject::new(1, "Fan Mode", 3).unwrap();
    msv.write_property(
        PropertyIdentifier::STATE_TEXT,
        None,
        PropertyValue::List(vec![text("Off"), text("Low"), text("High")]),
        None,
    )
    .unwrap();
    let f = EngineeringUnits::DEGREES_FAHRENHEIT;
    let mut iv = IntegerValueObject::with_access(1, "Meter", PresentValueAccess::Writable).unwrap();
    iv.set_units(EngineeringUnits::KILOWATT_HOURS).unwrap();
    vec![
        Box::new(ai),
        Box::new(AnalogOutputObject::new(1, "Damper", EngineeringUnits::PERCENT).unwrap()),
        Box::new(AnalogValueObject::new(1, "Setpoint", f).unwrap()),
        Box::new(
            AnalogValueObject::with_access(
                2,
                "Total",
                EngineeringUnits::KILOWATT_HOURS,
                PresentValueAccess::Writable,
            )
            .unwrap(),
        ),
        Box::new(BinaryInputObject::new(1, "Filter").unwrap()),
        Box::new(BinaryOutputObject::new(1, "Fan").unwrap()),
        Box::new(BinaryValueObject::new(1, "Occupied").unwrap()),
        Box::new(MultiStateInputObject::new(1, "Season", 4).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "Stage", 2).unwrap()),
        Box::new(msv),
        Box::new(iv),
        Box::new(TrendLogObject::new(1, "Log", 10).unwrap()),
    ]
}

/// The lab device and a client over a low-level client paired with it,
/// through a link with `faults`.
pub(crate) async fn lab(
    faults: Faults,
) -> (Client<TestTransport>, BACnetServer<LoopbackTransport>) {
    let (client, server, _) = lab_logged(faults).await;
    (client, server)
}

/// [`lab`], with the log of what the client's side sent.
pub(crate) async fn lab_logged(
    faults: Faults,
) -> (
    Client<TestTransport>,
    BACnetServer<LoopbackTransport>,
    SendLog,
) {
    let identity = DeviceIdentity::new(LAB, 42)
        .unwrap()
        .with_name("Lab \"AHU\"");
    let database = build_database_with_extra(&identity, objects()).unwrap();
    serve_logged(faults, identity, database).await
}

/// A device serving `database` as `identity`, and a client over a low-level
/// client paired with it, through a link with `faults`.
pub(crate) async fn serve(
    faults: Faults,
    identity: DeviceIdentity,
    database: ObjectDatabase,
) -> (Client<TestTransport>, BACnetServer<LoopbackTransport>) {
    let (client, server, _) = serve_logged(faults, identity, database).await;
    (client, server)
}

/// [`serve`], with the log of what the client's side sent.
pub(crate) async fn serve_logged(
    faults: Faults,
    identity: DeviceIdentity,
    database: ObjectDatabase,
) -> (
    Client<TestTransport>,
    BACnetServer<LoopbackTransport>,
    SendLog,
) {
    let instance = identity.instance();
    let (client_port, server_port) = LoopbackTransport::pair(vec![1], vec![2]);
    let server = BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(server_port)
        .build()
        .await
        .unwrap();
    let config = ClientConfig {
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let (link, log) = link(client_port, faults);
    let low = BACnetClient::start(config, link).await.unwrap();
    low.add_device(instance, &[2]).await.unwrap();
    let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
    (client, server, log)
}

/// The confirmed requests in `log`, in the order sent, by service choice:
/// 12 for ReadProperty, 14 for ReadPropertyMultiple.
pub(crate) fn services(log: &SendLog) -> Vec<u8> {
    log.frames()
        .iter()
        .filter_map(|frame| request(&frame.npdu).map(|(_, service, _)| service))
        .collect()
}

/// How many of `service` the log holds.
pub(crate) fn count(log: &SendLog, service: u8) -> usize {
    services(log)
        .iter()
        .filter(|&&sent| sent == service)
        .count()
}

/// A link to a device that refuses RPM.
pub(crate) fn refusing_rpm() -> Faults {
    Faults {
        reject_rpm: true,
        ..Faults::default()
    }
}

pub(crate) fn passing(pass: usize) -> Faults {
    Faults {
        pass,
        ..Faults::default()
    }
}

/// The lab device's handle, following its instance.
pub(crate) async fn lab_device(client: &Client<TestTransport>) -> crate::Device<TestTransport> {
    client.device(LAB).await.unwrap()
}

/// Where the lab device answers.
pub(crate) fn lab_at() -> crate::DeviceAddress {
    crate::DeviceAddress::local(&[2]).unwrap()
}

pub(crate) async fn stop(
    client: Client<TestTransport>,
    mut server: BACnetServer<LoopbackTransport>,
) {
    let Some(low) = client.into_low_level() else {
        panic!("the test holds the only client");
    };
    let Ok(mut low) = Arc::try_unwrap(low) else {
        panic!("the client holds the only low-level client");
    };
    low.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// One property as a device gave it.
pub(crate) fn row(property: PropertyIdentifier, value: PropertyValue) -> crate::format::RawRow {
    let mut data = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut data, &value).unwrap();
    (property, Ok(data.to_vec()))
}

/// One property the device answered with an error: that it is unknown.
pub(crate) fn refused(property: PropertyIdentifier) -> crate::format::RawRow {
    let error = bacnet_types::error::RemoteError::Error {
        class: bacnet_types::enums::ErrorClass::PROPERTY,
        code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
    };
    (property, Err(crate::format::PropertyError::Remote(error)))
}

/// What [`Answering`] gives for one property: a value, or an error class
/// and code.
pub(crate) type Answer = Result<PropertyValue, (ErrorClass, ErrorCode)>;

/// An object that answers every read as the one it wraps does but those of
/// the properties `answers` names, which it answers as given there: a
/// value of another type than the property's, one out of its range, or an
/// error, as an older or non-conforming device might. A property answered
/// with an error is left out of its Property_List too.
pub(crate) struct Answering {
    object: Box<dyn BACnetObject>,
    answers: Vec<(PropertyIdentifier, Answer)>,
}

impl Answering {
    /// `object` behind an [`Answering`] that gives `answers`, ready for a
    /// database.
    pub(crate) fn wrap(
        object: Box<dyn BACnetObject>,
        answers: Vec<(PropertyIdentifier, Answer)>,
    ) -> Box<dyn BACnetObject> {
        Box::new(Self { object, answers })
    }
}

impl BACnetObject for Answering {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.object.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.object.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match self
            .answers
            .iter()
            .find(|(answered, _)| *answered == property)
        {
            Some((_, Ok(value))) => Ok(value.clone()),
            Some((_, Err((class, code)))) => Err(Error::protocol(*class, *code)),
            None => self.object.read_property(property, index),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.object.write_property(property, index, value, priority)
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        let refused = |property| {
            self.answers
                .iter()
                .any(|(answered, answer)| *answered == property && answer.is_err())
        };
        let rows = self.object.property_metadata();
        let kept = rows
            .iter()
            .filter(|row| !refused(row.property_identifier))
            .cloned()
            .collect();
        Cow::Owned(kept)
    }
}

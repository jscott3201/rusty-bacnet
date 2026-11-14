//! The inventory against an in-process device over a loopback link: what
//! it reads, what it costs (by RPM, and by ReadProperty when the device
//! refuses RPM), and what a silent device or a cancel leaves. The round
//! trip from a point list is in `round_trip_tests`, over the same link.

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
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_objects::value_types::IntegerValueObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendMode, StartMode, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use bytes::Bytes;

use super::tests::oid;
use super::*;
use crate::engine::{Client, Halt, Reader};
use crate::limits::RequestLimits;

pub(super) const LAB: u32 = 2436;

/// What the client's side of the lab link does to requests.
#[derive(Clone)]
pub(super) struct Faults {
    /// Requests passed on before the link goes silent.
    pass: usize,
    /// Answer each ReadPropertyMultiple with a Reject, as a device
    /// without RPM does.
    reject_rpm: bool,
    /// Answer a ReadProperty of the whole Object_List with a Reject, so
    /// the list is read element by element.
    reject_whole_list: bool,
    /// Answer each ReadPropertyMultiple with an Error PDU (PROPERTY,
    /// UNKNOWN_PROPERTY), as a device does that fails the whole request
    /// over one property it lacks.
    error_rpm: bool,
    /// Set this flag as the given request goes out (1 for the first).
    cancel: Option<(usize, Arc<AtomicBool>)>,
}

impl Default for Faults {
    fn default() -> Self {
        Self {
            pass: usize::MAX,
            reject_rpm: false,
            reject_whole_list: false,
            error_rpm: false,
            cancel: None,
        }
    }
}

/// A local unsegmented confirmed request's invoke ID, service and service
/// request.
fn request(npdu: &[u8]) -> Option<(u8, u8, &[u8])> {
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
    let rpm = faults.reject_rpm && service == 14;
    let whole_list =
        faults.reject_whole_list && service == 12 && matches!(rest, [0x0C, _, _, _, _, 0x19, 0x4C]);
    // A Reject PDU for UNRECOGNIZED_SERVICE (9).
    (rpm || whole_list).then(|| vec![0x01, 0x00, 0x60, invoke_id, 9])
}

/// The client's side of the lab link: `inner`, relayed through a channel the
/// link feeds the Rejects and Errors of `faults` into.
fn link(inner: LoopbackTransport, faults: Faults) -> TestTransport {
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .start(StartMode::Relay(256));
    let inbox = builder.handle();
    let sent = Arc::new(AtomicUsize::new(0));
    let faults = Arc::new(faults);
    builder
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
        .build()
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

/// One object of each exported type (two analog values: one commandable,
/// one writable), a multi-state value with state texts, a writable integer
/// value in kilowatt-hours, and a trend log.
fn objects() -> Vec<Box<dyn BACnetObject>> {
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

/// The lab device and an engine over a client paired with it, through a
/// link with `faults`.
async fn lab(faults: Faults) -> (Client<TestTransport>, BACnetServer<LoopbackTransport>) {
    let identity = DeviceIdentity::new(LAB, 42)
        .unwrap()
        .with_name("Lab \"AHU\"");
    let database = build_database_with_extra(&identity, objects()).unwrap();
    serve(faults, identity, database).await
}

/// A device serving `database` as `identity`, and an engine over a client
/// paired with it, through a link with `faults`.
pub(super) async fn serve(
    faults: Faults,
    identity: DeviceIdentity,
    database: ObjectDatabase,
) -> (Client<TestTransport>, BACnetServer<LoopbackTransport>) {
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
    let link = link(client_port, faults);
    let client = BACnetClient::start(config, link).await.unwrap();
    client.add_device(instance, &[2]).await.unwrap();
    let engine = Client::new(Arc::new(client), Arc::new(RequestLimits::new(0)));
    (engine, server)
}

/// A link to a device that refuses RPM.
pub(super) fn refusing_rpm() -> Faults {
    Faults {
        reject_rpm: true,
        ..Faults::default()
    }
}

fn passing(pass: usize) -> Faults {
    Faults {
        pass,
        ..Faults::default()
    }
}

/// The lab device's handle, following its instance.
pub(super) async fn lab_device(engine: &Client<TestTransport>) -> crate::Device<TestTransport> {
    engine.device(LAB).await.unwrap()
}

/// Where the lab device answers.
pub(super) fn lab_at() -> crate::DeviceAddress {
    crate::DeviceAddress::local(&[2]).unwrap()
}

pub(super) async fn stop(
    engine: Client<TestTransport>,
    mut server: BACnetServer<LoopbackTransport>,
) {
    let Some(client) = engine.into_low_level() else {
        panic!("the test holds the only client");
    };
    let Ok(mut client) = Arc::try_unwrap(client) else {
        panic!("the client holds the only low-level client");
    };
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Whether `point` reads as commandable.
fn commandable(point: &Point) -> bool {
    match &point.kind {
        PointKind::Analog(analog) => matches!(analog.access, Access::Commandable { .. }),
        PointKind::Binary(binary) => matches!(binary.access, Access::Commandable { .. }),
        PointKind::MultiState(states) => matches!(states.access, Access::Commandable { .. }),
        PointKind::Integer(integer) => matches!(integer.access, Access::Commandable { .. }),
        _ => false,
    }
}

/// Every exported object is read once, by RPM: the device's name, its
/// Object_List, then one request an object, N + 2 in all. The trend log
/// isn't read. The device's text comes back exactly, and a value without
/// a Priority_Array reads as read-only, the writable one included.
#[tokio::test(start_paused = true)]
async fn reads_each_exported_object_once() {
    let (engine, server) = lab(Faults::default()).await;
    let inventory = lab_device(&engine)
        .await
        .inventory(&InventoryOptions::default())
        .await
        .unwrap();
    assert_eq!(engine.limits().recent(), 11 + 2, "{inventory:?}");
    assert_eq!(
        inventory.device,
        DeviceInfo::new(LAB).with_name("Lab \"AHU\"")
    );
    assert_eq!(inventory.stopped, None);
    let points: Vec<&Point> = inventory.points().collect();
    let object = |object_type, instance| {
        *points
            .iter()
            .find(|o| o.object == oid(object_type, instance))
            .unwrap_or_else(|| panic!("{object_type} {instance}: {inventory:?}"))
    };
    assert_eq!(points.len(), 11, "none failed: {inventory:?}");
    assert_eq!(inventory.entries.len(), 11);
    let ai = object(ObjectType::ANALOG_INPUT, 1);
    assert_eq!(ai.name, "Zone Temp");
    assert_eq!(ai.description.as_deref(), Some("Room 101"));
    let PointKind::Analog(analog) = &ai.kind else {
        panic!("{ai:?}");
    };
    // Export reads none of the optional settings.
    let mut expected = Analog::new(EngineeringUnits::DEGREES_FAHRENHEIT, Access::ReadOnly);
    assert_eq!(analog, &expected);
    assert!(commandable(object(ObjectType::ANALOG_OUTPUT, 1)));
    assert!(commandable(object(ObjectType::ANALOG_VALUE, 1)));
    let total = object(ObjectType::ANALOG_VALUE, 2);
    expected.units = EngineeringUnits::KILOWATT_HOURS;
    assert_eq!(total.kind, PointKind::Analog(expected), "{total:?}");
    assert!(commandable(object(ObjectType::BINARY_VALUE, 1)));
    assert!(!commandable(object(ObjectType::BINARY_INPUT, 1)));
    let msv = object(ObjectType::MULTI_STATE_VALUE, 1);
    let commanded = Access::Commandable {
        relinquish_default: None,
    };
    let three = MultiState::new(3, commanded).with_state_texts(["Off", "Low", "High"]);
    assert_eq!(msv.kind, PointKind::MultiState(three));
    let PointKind::MultiState(msi) = &object(ObjectType::MULTI_STATE_INPUT, 1).kind else {
        panic!("a multi-state input");
    };
    assert_eq!((msi.number_of_states, msi.access), (4, Access::ReadOnly));
    // The integer value, writable, reads as read-only, as the analog does.
    let meter = object(ObjectType::INTEGER_VALUE, 1);
    let integer = Integer::new(EngineeringUnits::KILOWATT_HOURS, Access::ReadOnly);
    assert_eq!(meter.kind, PointKind::Integer(integer), "{meter:?}");
    assert!(!engine.refused_rpm(&lab_at()));

    let turtle = to_turtle(&inventory, &TurtleOptions::default());
    super::tests::check_structure(&turtle);
    assert!(
        turtle.contains("bacnet:device-name \"Lab \\\"AHU\\\"\" ;"),
        "{turtle}"
    );
    assert!(
        turtle.contains("d:integer-value-1.present-value a s223:QuantifiableProperty ;"),
        "{turtle}"
    );
    assert!(
        turtle.contains("# 11 objects exported, 0 skipped"),
        "{turtle}"
    );
    stop(engine, server).await;
}

/// `types` narrows the reading: the other objects cost nothing.
#[tokio::test(start_paused = true)]
async fn types_narrow_what_is_read() {
    let (engine, server) = lab(Faults::default()).await;
    let options = InventoryOptions::new(vec![ObjectType::MULTI_STATE_VALUE]).unwrap();
    let inventory = lab_device(&engine).await.inventory(&options).await.unwrap();
    assert_eq!(engine.limits().recent(), 1 + 2);
    assert_eq!(inventory.entries.len(), 1);
    assert_eq!(
        inventory.entries[0].object(),
        oid(ObjectType::MULTI_STATE_VALUE, 1)
    );
    stop(engine, server).await;
}

/// A device that refuses RPM costs one refused request, then a
/// ReadProperty a property, by type: 2 for a binary input or output, 3 for
/// an analog input or output or a binary value, 4 for an analog value, an
/// integer value or a multi-state input or output, 5 for a multi-state
/// value. Each type is
/// read alone, so the refusal comes first each time.
#[tokio::test(start_paused = true)]
async fn by_read_property_when_rpm_is_refused() {
    let costs = [
        (ObjectType::ANALOG_INPUT, 3),
        (ObjectType::ANALOG_OUTPUT, 3),
        (ObjectType::ANALOG_VALUE, 2 * 4),
        (ObjectType::BINARY_INPUT, 2),
        (ObjectType::BINARY_OUTPUT, 2),
        (ObjectType::BINARY_VALUE, 3),
        (ObjectType::MULTI_STATE_INPUT, 4),
        (ObjectType::MULTI_STATE_OUTPUT, 4),
        (ObjectType::MULTI_STATE_VALUE, 5),
        (ObjectType::INTEGER_VALUE, 4),
    ];
    for (object_type, cost) in costs {
        let refuses = Faults {
            reject_rpm: true,
            ..Faults::default()
        };
        let (engine, server) = lab(refuses).await;
        let options = InventoryOptions::new(vec![object_type]).unwrap();
        let inventory = lab_device(&engine).await.inventory(&options).await.unwrap();
        assert!(engine.refused_rpm(&lab_at()));
        // The name and the list, the refused RPM, then the ReadProperty.
        assert_eq!(engine.limits().recent(), 2 + 1 + cost, "{object_type}");
        assert!(
            inventory.entries.iter().all(|o| o.error().is_none()),
            "{inventory:?}"
        );
        if object_type == ObjectType::MULTI_STATE_VALUE {
            let Some(PointKind::MultiState(msv)) = inventory.points().next().map(|p| &p.kind)
            else {
                panic!("{inventory:?}");
            };
            assert_eq!(msv.state_texts, ["Off", "Low", "High"]);
        }
        stop(engine, server).await;
    }
}

/// A device that stops answering partway ends the reading: what was read
/// is kept, and the inventory says where it stopped. One that never
/// answers fails it once, at its name.
#[tokio::test(start_paused = true)]
async fn a_device_that_stops_answering() {
    let (engine, server) = lab(passing(4)).await;
    let inventory = lab_device(&engine)
        .await
        .inventory(&InventoryOptions::default())
        .await
        .unwrap();
    assert_eq!(inventory.entries.len(), 2, "{inventory:?}");
    let stopped = inventory.stopped.as_deref().unwrap();
    assert!(stopped.starts_with("analog-value 1: "), "{stopped}");
    assert_eq!(engine.limits().recent(), 5, "no request after the timeout");
    let turtle = to_turtle(&inventory, &TurtleOptions::default());
    assert!(
        turtle.contains("# Incomplete: the reading stopped early"),
        "{turtle}"
    );
    stop(engine, server).await;

    let (engine, server) = lab(passing(0)).await;
    let error = lab_device(&engine)
        .await
        .inventory(&InventoryOptions::default())
        .await
        .unwrap_err();
    assert!(crate::engine::unreachable(&error), "{error}");
    assert_eq!(engine.limits().recent(), 1);
    stop(engine, server).await;
}

/// An Object_List read element by element that stops early ends the
/// reading there: no object is asked for, and its reason is the one kept.
#[tokio::test(start_paused = true)]
async fn a_list_that_stops_early_ends_the_reading() {
    // The name, the refused whole list, element 0, elements 1 and 2; then
    // element 3 goes unanswered.
    let faults = Faults {
        pass: 5,
        reject_whole_list: true,
        ..Faults::default()
    };
    let (engine, server) = lab(faults).await;
    let inventory = lab_device(&engine)
        .await
        .inventory(&InventoryOptions::default())
        .await
        .unwrap();
    assert!(inventory.entries.is_empty(), "{inventory:?}");
    let stopped = inventory.stopped.as_deref().unwrap();
    assert!(
        stopped.starts_with("Object_List, element 3 of "),
        "{stopped}"
    );
    assert_eq!(engine.limits().recent(), 6, "no object asked for");
    stop(engine, server).await;
}

/// A caller that cancels once the Object_List is in keeps what was read,
/// with a reason that says so; one that cancels before has an error.
#[tokio::test(start_paused = true)]
async fn a_cancel_keeps_what_was_read() {
    let flag = Arc::new(AtomicBool::new(false));
    let faults = Faults {
        cancel: Some((4, Arc::clone(&flag))),
        ..Faults::default()
    };
    let (engine, server) = lab(faults).await;
    let mut reader = Reader {
        client: engine.low_level(),
        device: lab_at(),
        instance: Some(LAB),
        rate: engine.limits(),
        cancel: &flag,
        rpm: true,
    };
    // The name, the list, then two objects: the flag goes up as the second
    // one's request leaves, so the third isn't asked for.
    let inventory = read_inventory(&mut reader, &InventoryOptions::default())
        .await
        .unwrap();
    assert_eq!(inventory.points().count(), 2, "{inventory:?}");
    let stopped = inventory.stopped.as_deref().unwrap();
    assert!(stopped.starts_with("cancelled before "), "{stopped}");
    assert_eq!(engine.limits().recent(), 4);
    stop(engine, server).await;

    let flag = Arc::new(AtomicBool::new(false));
    let faults = Faults {
        cancel: Some((1, Arc::clone(&flag))),
        ..Faults::default()
    };
    let (engine, server) = lab(faults).await;
    let mut reader = Reader {
        client: engine.low_level(),
        device: lab_at(),
        instance: Some(LAB),
        rate: engine.limits(),
        cancel: &flag,
        rpm: true,
    };
    let early = read_inventory(&mut reader, &InventoryOptions::default()).await;
    assert!(matches!(early, Err(Halt::Cancelled)), "{early:?}");
    stop(engine, server).await;
}

/// The pieces a front end that pages reads give what the inventory reads,
/// at either detail: the device exactly, and each object as the inventory
/// has it, one request each; one the device doesn't have fails, naming
/// the first property it lacks. A refusal of RPM is remembered, and a
/// silent device fails either read.
#[tokio::test(start_paused = true)]
async fn the_pieces_read_as_the_inventory_does() {
    let (engine, server) = lab(Faults::default()).await;
    for detail in [Detail::Export, Detail::Full] {
        let options = InventoryOptions::default().with_detail(detail);
        let inventory = lab_device(&engine).await.inventory(&options).await.unwrap();
        let before = engine.limits().recent();
        let device = lab_device(&engine)
            .await
            .inventory_device(detail)
            .await
            .unwrap();
        assert_eq!(device, inventory.device);
        for expected in &inventory.entries {
            let read = lab_device(&engine)
                .await
                .inventory_object(expected.object(), detail)
                .await;
            assert_eq!(&read.unwrap(), expected);
        }
        assert_eq!(
            engine.limits().recent() - before,
            1 + inventory.entries.len()
        );
    }
    let missing = oid(ObjectType::ANALOG_INPUT, 99);
    let read = lab_device(&engine)
        .await
        .inventory_object(missing, Detail::Export)
        .await;
    // The device answers each of its properties with an error, so there is
    // no name: the object fails, as the inventory would have it.
    let Entry::Failed {
        object,
        property,
        error,
    } = read.unwrap()
    else {
        panic!("an object the device doesn't have is no point");
    };
    assert_eq!(
        (object, property),
        (missing, Some(PropertyIdentifier::OBJECT_NAME))
    );
    assert!(error.starts_with("object-name: "), "{error}");
    // A type the export doesn't map isn't asked for.
    let before = engine.limits().recent();
    let trend = lab_device(&engine)
        .await
        .inventory_object(oid(ObjectType::TREND_LOG, 1), Detail::Full)
        .await
        .unwrap();
    assert_eq!(trend.error(), Some("trend-log objects aren't exported"));
    assert_eq!(engine.limits().recent(), before);
    stop(engine, server).await;

    let refuses = Faults {
        reject_rpm: true,
        ..Faults::default()
    };
    let (engine, server) = lab(refuses).await;
    let ai = lab_device(&engine)
        .await
        .inventory_object(oid(ObjectType::ANALOG_INPUT, 1), Detail::Export)
        .await
        .unwrap();
    let Some(PointKind::Analog(analog)) = ai.point().map(|point| &point.kind) else {
        panic!("{ai:?}");
    };
    assert_eq!(analog.units, EngineeringUnits::DEGREES_FAHRENHEIT);
    assert!(engine.refused_rpm(&lab_at()));
    // The refused RPM, then a ReadProperty each for the three properties.
    assert_eq!(engine.limits().recent(), 1 + 3);
    stop(engine, server).await;

    let (engine, server) = lab(passing(0)).await;
    for detail in [Detail::Export, Detail::Full] {
        let error = lab_device(&engine)
            .await
            .inventory_device(detail)
            .await
            .unwrap_err();
        assert!(crate::engine::unreachable(&error), "{error}");
        let ai = oid(ObjectType::ANALOG_INPUT, 1);
        let error = lab_device(&engine)
            .await
            .inventory_object(ai, detail)
            .await
            .unwrap_err();
        assert!(crate::engine::unreachable(&error), "{error}");
    }
    stop(engine, server).await;
}

/// One property as a device gave it.
fn row(property: PropertyIdentifier, value: PropertyValue) -> crate::format::RawRow {
    let mut data = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut data, &value).unwrap();
    (property, Ok(data.to_vec()))
}

/// One property the device answered with an error.
fn refused(property: PropertyIdentifier) -> crate::format::RawRow {
    let error = bacnet_types::error::RemoteError::Error {
        class: bacnet_types::enums::ErrorClass::PROPERTY,
        code: bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY,
    };
    (property, Err(crate::format::RowError::Remote(error)))
}

/// A property every object of the type has fails the entry, naming the
/// property, whether the device refused it or gave something else; an
/// optional one the device refused is `None`. A value's relinquish
/// default counts only with a Priority_Array.
#[test]
fn a_missing_required_property_fails_the_entry() {
    use PropertyIdentifier as P;
    let name = || row(P::OBJECT_NAME, text("Zone"));
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let failed = |entry: Entry| entry.error().map(str::to_string);
    let no_units = super::inventory::entry_of(ai, &[name(), refused(P::UNITS)]);
    assert!(
        matches!(
            no_units,
            Entry::Failed {
                property: Some(P::UNITS),
                ..
            }
        ),
        "{no_units:?}"
    );
    assert_eq!(
        failed(no_units).as_deref(),
        Some("units: PROPERTY:UNKNOWN_PROPERTY")
    );
    let not_units = [name(), row(P::UNITS, PropertyValue::Real(1.0))];
    let not_units = super::inventory::entry_of(ai, &not_units);
    assert_eq!(
        failed(not_units).as_deref(),
        Some("units: not an engineering unit")
    );
    let no_name = [
        refused(P::OBJECT_NAME),
        row(P::UNITS, PropertyValue::Enumerated(62)),
    ];
    let no_name = super::inventory::entry_of(ai, &no_name);
    assert_eq!(
        failed(no_name).as_deref(),
        Some("object-name: PROPERTY:UNKNOWN_PROPERTY")
    );
    // An integer value needs its Units as an analog object does; its range
    // and relinquish default decode as INTEGER and its increment as an
    // Unsigned of any size, and anything else there is None.
    let iv = oid(ObjectType::INTEGER_VALUE, 1);
    let no_units = super::inventory::entry_of(iv, &[name(), refused(P::UNITS)]);
    assert_eq!(
        failed(no_units).as_deref(),
        Some("units: PROPERTY:UNKNOWN_PROPERTY")
    );
    let rows = [
        name(),
        row(P::UNITS, PropertyValue::Enumerated(19)),
        row(P::PRIORITY_ARRAY, PropertyValue::Null),
        row(P::RELINQUISH_DEFAULT, PropertyValue::Signed(-4)),
        row(P::COV_INCREMENT, PropertyValue::Unsigned(u64::MAX)),
        row(P::MIN_PRES_VALUE, PropertyValue::Signed(i32::MIN)),
        row(P::MAX_PRES_VALUE, PropertyValue::Real(5.0)),
    ];
    let mut expected = Integer::new(
        EngineeringUnits::KILOWATT_HOURS,
        Access::Commandable {
            relinquish_default: Some(-4),
        },
    )
    .with_cov_increment(u64::MAX)
    .with_min_pres_value(i32::MIN);
    expected.max_pres_value = None;
    assert_eq!(
        super::inventory::entry_of(iv, &rows),
        Entry::Point(Point::new(iv, "Zone", expected))
    );
    let msi = oid(ObjectType::MULTI_STATE_INPUT, 1);
    let states = [name(), row(P::NUMBER_OF_STATES, text("three"))];
    let states = super::inventory::entry_of(msi, &states);
    assert_eq!(
        failed(states).as_deref(),
        Some("number-of-states: not an Unsigned")
    );

    // An optional property refused is None; a binary value's relinquish
    // default without a Priority_Array is no access of its own.
    let bv = oid(ObjectType::BINARY_VALUE, 1);
    let rows = [
        name(),
        refused(P::DESCRIPTION),
        refused(P::PRIORITY_ARRAY),
        row(P::ACTIVE_TEXT, text("")),
        refused(P::INACTIVE_TEXT),
        row(P::RELINQUISH_DEFAULT, PropertyValue::Enumerated(1)),
    ];
    let read = super::inventory::entry_of(bv, &rows);
    let mut expected = Binary::new(Access::ReadOnly);
    expected.active_text = Some(String::new());
    assert_eq!(read, Entry::Point(Point::new(bv, "Zone", expected)));
    let bo = oid(ObjectType::BINARY_OUTPUT, 1);
    let read = super::inventory::entry_of(bo, &rows);
    let mut expected = Binary::new(Access::Commandable {
        relinquish_default: Some(bacnet_types::enums::BinaryPV::ACTIVE),
    });
    expected.active_text = Some(String::new());
    assert_eq!(read, Entry::Point(Point::new(bo, "Zone", expected)));
}

/// A device that fails a whole RPM with an error of its own, not a refusal
/// of the service: at Full the Device's name is then read by ReadProperty,
/// as Export reads it, so Full never knows less than Export. Only the
/// Description, which Export doesn't read, is lost.
#[tokio::test(start_paused = true)]
async fn full_reads_the_device_name_after_an_rpm_error() {
    let faults = Faults {
        error_rpm: true,
        ..Faults::default()
    };
    let (engine, server) = lab(faults).await;
    let export = lab_device(&engine)
        .await
        .inventory_device(Detail::Export)
        .await
        .unwrap();
    let before = engine.limits().recent();
    let full = lab_device(&engine)
        .await
        .inventory_device(Detail::Full)
        .await
        .unwrap();
    assert_eq!(full, export);
    assert_eq!(full.name.as_deref(), Some("Lab \"AHU\""));
    // The failed RPM, then the name by ReadProperty.
    assert_eq!(engine.limits().recent() - before, 2);
    assert!(
        !engine.refused_rpm(&lab_at()),
        "an error isn't a refusal of RPM"
    );
    stop(engine, server).await;
}

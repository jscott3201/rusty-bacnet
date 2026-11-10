//! The inventory against an in-process device over a loopback link: what
//! it reads, what it costs, and what a failure leaves.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::tests::oid;
use super::*;
use crate::engine::Engine;
use crate::limits::RequestLimits;

const LAB: u32 = 2436;

/// The client's side of the lab link, passing on its first `pass`
/// requests and dropping the rest, as a device that stops answering.
struct Muting {
    inner: LoopbackTransport,
    pass: AtomicUsize,
}

impl TransportPort for Muting {
    async fn start(&mut self) -> Result<tokio::sync::mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let left = self.pass.load(Ordering::Relaxed);
        if left == 0 {
            return Ok(());
        }
        self.pass.store(left - 1, Ordering::Relaxed);
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

/// A device with an analog input and output, a commandable and a writable
/// analog value, a binary value, a multi-state value with state texts and
/// a trend log; an engine over a client paired with it, whose link passes
/// on its first `pass` requests.
async fn lab(pass: usize) -> (Engine<Muting>, BACnetServer<LoopbackTransport>) {
    let identity = DeviceIdentity::new(LAB, 42)
        .unwrap()
        .with_name("Lab \"AHU\"");
    let mut ai =
        AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    ai.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        text("Room 101"),
        None,
    )
    .unwrap();
    let ao = AnalogOutputObject::new(1, "Damper", EngineeringUnits::PERCENT).unwrap();
    let av = AnalogValueObject::new(1, "Setpoint", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
    let total = AnalogValueObject::with_access(
        2,
        "Total",
        EngineeringUnits::KILOWATT_HOURS,
        PresentValueAccess::Writable,
    )
    .unwrap();
    let bv = BinaryValueObject::new(1, "Occupied").unwrap();
    let mut msv = MultiStateValueObject::new(1, "Fan Mode", 3).unwrap();
    msv.write_property(
        PropertyIdentifier::STATE_TEXT,
        None,
        PropertyValue::List(vec![text("Off"), text("Low"), text("High")]),
        None,
    )
    .unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(ai),
        Box::new(ao),
        Box::new(av),
        Box::new(total),
        Box::new(bv),
        Box::new(msv),
        Box::new(TrendLogObject::new(1, "Log", 10).unwrap()),
    ];
    let database = build_database_with_extra(&identity, objects).unwrap();
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
    let link = Muting {
        inner: client_port,
        pass: AtomicUsize::new(pass),
    };
    let client = BACnetClient::start(config, link).await.unwrap();
    client.add_device(LAB, &[2]).await.unwrap();
    let engine = Engine::new(Arc::new(client), Arc::new(RequestLimits::new(0)));
    (engine, server)
}

async fn stop(engine: Engine<Muting>, mut server: BACnetServer<LoopbackTransport>) {
    let Ok(mut client) = Arc::try_unwrap(engine.client) else {
        panic!("the engine holds the only client");
    };
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Every exported object is read once, by RPM: the device's name, its
/// Object_List, then one request an object, N + 2 in all. The trend log
/// isn't read. The device's text comes back exactly, and a value without
/// a Priority_Array says so.
#[tokio::test(start_paused = true)]
async fn reads_each_exported_object_once() {
    let (engine, server) = lab(usize::MAX).await;
    let inventory = engine
        .inventory(LAB, &InventoryOptions::default())
        .await
        .unwrap();
    assert_eq!(engine.limits().recent(), 6 + 2, "{inventory:?}");
    assert_eq!(inventory.device, LAB);
    assert_eq!(inventory.device_name.as_deref(), Some("Lab \"AHU\""));
    assert_eq!(inventory.stopped, None);
    let objects: Vec<_> = inventory.objects.iter().map(|o| o.object).collect();
    assert_eq!(
        objects,
        [
            oid(ObjectType::ANALOG_INPUT, 1),
            oid(ObjectType::ANALOG_OUTPUT, 1),
            oid(ObjectType::ANALOG_VALUE, 1),
            oid(ObjectType::ANALOG_VALUE, 2),
            oid(ObjectType::BINARY_VALUE, 1),
            oid(ObjectType::MULTI_STATE_VALUE, 1),
        ]
    );
    let [ai, ao, av, total, bv, msv] = &inventory.objects[..] else {
        unreachable!()
    };
    assert_eq!(ai.name.as_deref(), Some("Zone Temp"));
    assert_eq!(ai.description.as_deref(), Some("Room 101"));
    assert_eq!(ai.units, Some(EngineeringUnits::DEGREES_FAHRENHEIT));
    assert!(!ai.priority_array);
    assert!(ao.priority_array);
    assert!(av.priority_array);
    assert!(!total.priority_array, "{total:?}");
    assert_eq!(total.units, Some(EngineeringUnits::KILOWATT_HOURS));
    assert!(bv.priority_array);
    assert_eq!(msv.states, Some(3));
    assert_eq!(msv.state_texts, ["Off", "Low", "High"]);
    assert!(inventory.objects.iter().all(|o| o.error.is_none()));
    assert!(!engine.refused_rpm(LAB));

    let turtle = to_turtle(&inventory, &TurtleOptions::default());
    super::tests::check_structure(&turtle);
    assert!(
        turtle.contains("bacnet:device-name \"Lab \\\"AHU\\\"\" ;"),
        "{turtle}"
    );
    assert!(
        turtle.contains("# 6 objects exported, 0 skipped"),
        "{turtle}"
    );
    stop(engine, server).await;
}

/// `types` narrows the reading: the other objects cost nothing.
#[tokio::test(start_paused = true)]
async fn types_narrow_what_is_read() {
    let (engine, server) = lab(usize::MAX).await;
    let options = InventoryOptions::new(vec![ObjectType::MULTI_STATE_VALUE]).unwrap();
    let inventory = engine.inventory(LAB, &options).await.unwrap();
    assert_eq!(engine.limits().recent(), 1 + 2);
    assert_eq!(inventory.objects.len(), 1);
    assert_eq!(
        inventory.objects[0].object,
        oid(ObjectType::MULTI_STATE_VALUE, 1)
    );
    stop(engine, server).await;
}

/// A device that stops answering partway ends the reading: what was read
/// is kept, and the inventory says where it stopped. One that never
/// answers fails it once, at its name.
#[tokio::test(start_paused = true)]
async fn a_device_that_stops_answering() {
    let (engine, server) = lab(4).await;
    let inventory = engine
        .inventory(LAB, &InventoryOptions::default())
        .await
        .unwrap();
    assert_eq!(inventory.objects.len(), 2, "{inventory:?}");
    let stopped = inventory.stopped.as_deref().unwrap();
    assert!(stopped.starts_with("analog-value 1: "), "{stopped}");
    assert_eq!(engine.limits().recent(), 5, "no request after the timeout");
    let turtle = to_turtle(&inventory, &TurtleOptions::default());
    assert!(
        turtle.contains("# Incomplete: the reading stopped early"),
        "{turtle}"
    );
    stop(engine, server).await;

    let (engine, server) = lab(0).await;
    let error = engine
        .inventory(LAB, &InventoryOptions::default())
        .await
        .unwrap_err();
    assert!(crate::engine::unreachable(&error), "{error}");
    assert_eq!(engine.limits().recent(), 1);
    stop(engine, server).await;
}

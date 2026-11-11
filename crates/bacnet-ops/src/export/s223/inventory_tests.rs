//! The inventory against an in-process device over a loopback link: what
//! it reads, what it costs (by RPM, and by ReadProperty when the device
//! refuses RPM), and what a silent device or a cancel leaves.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::Bytes;
use tokio::sync::mpsc;

use super::tests::oid;
use super::*;
use crate::engine::{Engine, Halt, Reader};
use crate::limits::RequestLimits;

const LAB: u32 = 2436;

/// What the client's side of the lab link does to requests.
#[derive(Clone)]
struct Faults {
    /// Requests passed on before the link goes silent.
    pass: usize,
    /// Answer each ReadPropertyMultiple with a Reject, as a device
    /// without RPM does.
    reject_rpm: bool,
    /// Answer a ReadProperty of the whole Object_List with a Reject, so
    /// the list is read element by element.
    reject_whole_list: bool,
    /// Set this flag as the given request goes out (1 for the first).
    cancel: Option<(usize, Arc<AtomicBool>)>,
}

impl Default for Faults {
    fn default() -> Self {
        Self {
            pass: usize::MAX,
            reject_rpm: false,
            reject_whole_list: false,
            cancel: None,
        }
    }
}

struct Link {
    inner: LoopbackTransport,
    faults: Faults,
    sent: AtomicUsize,
    /// Feeds the client's receive channel, for the Rejects.
    inbox: Mutex<Option<mpsc::Sender<ReceivedNpdu>>>,
}

impl Link {
    /// A local unsegmented confirmed request's invoke ID, service and
    /// service request.
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

    fn refused(&self, npdu: &[u8]) -> Option<u8> {
        let (invoke_id, service, rest) = Self::request(npdu)?;
        // RPM is service 14; ReadProperty 12, here of Object_List (76)
        // with no array index.
        let rpm = self.faults.reject_rpm && service == 14;
        let whole_list = self.faults.reject_whole_list
            && service == 12
            && matches!(rest, [0x0C, _, _, _, _, 0x19, 0x4C]);
        (rpm || whole_list).then_some(invoke_id)
    }
}

impl TransportPort for Link {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let mut from_peer = self.inner.start().await?;
        let (tx, rx) = mpsc::channel(256);
        *self.inbox.lock().unwrap() = Some(tx.clone());
        tokio::spawn(async move {
            while let Some(npdu) = from_peer.recv().await {
                if tx.send(npdu).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inbox.lock().unwrap().take();
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let sent = self.sent.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some((after, flag)) = &self.faults.cancel
            && sent >= *after
        {
            flag.store(true, Ordering::Relaxed);
        }
        if sent > self.faults.pass {
            return Ok(());
        }
        let Some(invoke_id) = self.refused(npdu) else {
            return self.inner.send_unicast(npdu, mac).await;
        };
        // A Reject PDU for UNRECOGNIZED_SERVICE (9), from the device.
        let reject = Bytes::from(vec![0x01, 0x00, 0x60, invoke_id, 9]);
        let inbox = self.inbox.lock().unwrap().clone();
        if let Some(inbox) = inbox {
            let source = MacAddr::from_slice(mac);
            let back = ReceivedNpdu::unverified(reject, source, false, Vec::new(), None);
            let _ = inbox.send(back).await;
        }
        Ok(())
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

/// One object of each exported type (two analog values: one commandable,
/// one writable), a multi-state value with state texts, and a trend log.
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
        Box::new(TrendLogObject::new(1, "Log", 10).unwrap()),
    ]
}

/// The lab device and an engine over a client paired with it, through a
/// link with `faults`.
async fn lab(faults: Faults) -> (Engine<Link>, BACnetServer<LoopbackTransport>) {
    let identity = DeviceIdentity::new(LAB, 42)
        .unwrap()
        .with_name("Lab \"AHU\"");
    let database = build_database_with_extra(&identity, objects()).unwrap();
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
    let link = Link {
        inner: client_port,
        faults,
        sent: AtomicUsize::new(0),
        inbox: Mutex::new(None),
    };
    let client = BACnetClient::start(config, link).await.unwrap();
    client.add_device(LAB, &[2]).await.unwrap();
    let engine = Engine::new(Arc::new(client), Arc::new(RequestLimits::new(0)));
    (engine, server)
}

fn passing(pass: usize) -> Faults {
    Faults {
        pass,
        ..Faults::default()
    }
}

async fn stop(engine: Engine<Link>, mut server: BACnetServer<LoopbackTransport>) {
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
    let (engine, server) = lab(Faults::default()).await;
    let inventory = engine
        .inventory(LAB, &InventoryOptions::default(), &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(engine.limits().recent(), 10 + 2, "{inventory:?}");
    assert_eq!(inventory.device, LAB);
    assert_eq!(inventory.device_name.as_deref(), Some("Lab \"AHU\""));
    assert_eq!(inventory.stopped, None);
    let object = |object_type, instance| {
        inventory
            .objects
            .iter()
            .find(|o| o.object == oid(object_type, instance))
            .unwrap_or_else(|| panic!("{object_type} {instance}: {inventory:?}"))
    };
    assert_eq!(inventory.objects.len(), 10);
    let ai = object(ObjectType::ANALOG_INPUT, 1);
    assert_eq!(ai.name.as_deref(), Some("Zone Temp"));
    assert_eq!(ai.description.as_deref(), Some("Room 101"));
    assert_eq!(ai.units, Some(EngineeringUnits::DEGREES_FAHRENHEIT));
    assert!(!ai.priority_array);
    assert!(object(ObjectType::ANALOG_OUTPUT, 1).priority_array);
    assert!(object(ObjectType::ANALOG_VALUE, 1).priority_array);
    let total = object(ObjectType::ANALOG_VALUE, 2);
    assert!(!total.priority_array, "{total:?}");
    assert_eq!(total.units, Some(EngineeringUnits::KILOWATT_HOURS));
    assert!(object(ObjectType::BINARY_VALUE, 1).priority_array);
    assert!(!object(ObjectType::BINARY_INPUT, 1).priority_array);
    let msv = object(ObjectType::MULTI_STATE_VALUE, 1);
    assert_eq!(msv.states, Some(3));
    assert_eq!(msv.state_texts, ["Off", "Low", "High"]);
    assert_eq!(object(ObjectType::MULTI_STATE_INPUT, 1).states, Some(4));
    assert!(inventory.objects.iter().all(|o| o.error.is_none()));
    assert!(!engine.refused_rpm(LAB));

    let turtle = to_turtle(&inventory, &TurtleOptions::default());
    super::tests::check_structure(&turtle);
    assert!(
        turtle.contains("bacnet:device-name \"Lab \\\"AHU\\\"\" ;"),
        "{turtle}"
    );
    assert!(
        turtle.contains("# 10 objects exported, 0 skipped"),
        "{turtle}"
    );
    stop(engine, server).await;
}

/// `types` narrows the reading: the other objects cost nothing.
#[tokio::test(start_paused = true)]
async fn types_narrow_what_is_read() {
    let (engine, server) = lab(Faults::default()).await;
    let options = InventoryOptions::new(vec![ObjectType::MULTI_STATE_VALUE]).unwrap();
    let inventory = engine
        .inventory(LAB, &options, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(engine.limits().recent(), 1 + 2);
    assert_eq!(inventory.objects.len(), 1);
    assert_eq!(
        inventory.objects[0].object,
        oid(ObjectType::MULTI_STATE_VALUE, 1)
    );
    stop(engine, server).await;
}

/// A device that refuses RPM costs one refused request, then a
/// ReadProperty a property, by type: 2 for a binary input or output, 3 for
/// an analog input or output or a binary value, 4 for an analog value or a
/// multi-state input or output, 5 for a multi-state value. Each type is
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
    ];
    for (object_type, cost) in costs {
        let refuses = Faults {
            reject_rpm: true,
            ..Faults::default()
        };
        let (engine, server) = lab(refuses).await;
        let options = InventoryOptions::new(vec![object_type]).unwrap();
        let inventory = engine
            .inventory(LAB, &options, &AtomicBool::new(false))
            .await
            .unwrap();
        assert!(engine.refused_rpm(LAB));
        // The name and the list, the refused RPM, then the ReadProperty.
        assert_eq!(engine.limits().recent(), 2 + 1 + cost, "{object_type}");
        assert!(
            inventory
                .objects
                .iter()
                .all(|o| o.error.is_none() && o.name.is_some()),
            "{inventory:?}"
        );
        if object_type == ObjectType::MULTI_STATE_VALUE {
            assert_eq!(inventory.objects[0].state_texts, ["Off", "Low", "High"]);
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
    let inventory = engine
        .inventory(LAB, &InventoryOptions::default(), &AtomicBool::new(false))
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

    let (engine, server) = lab(passing(0)).await;
    let halt = engine
        .inventory(LAB, &InventoryOptions::default(), &AtomicBool::new(false))
        .await;
    let Err(Halt::Device(error)) = halt else {
        panic!("{halt:?}");
    };
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
    let inventory = engine
        .inventory(LAB, &InventoryOptions::default(), &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(inventory.objects.is_empty(), "{inventory:?}");
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
        client: &engine.client,
        device: LAB,
        rate: engine.limits(),
        cancel: &flag,
        rpm: true,
    };
    // The name, the list, then two objects: the flag goes up as the second
    // one's request leaves, so the third isn't asked for.
    let inventory = read_inventory(&mut reader, &InventoryOptions::default())
        .await
        .unwrap();
    assert_eq!(inventory.objects.len(), 2, "{inventory:?}");
    assert!(inventory.objects.iter().all(|o| o.name.is_some()));
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
        client: &engine.client,
        device: LAB,
        rate: engine.limits(),
        cancel: &flag,
        rpm: true,
    };
    let early = read_inventory(&mut reader, &InventoryOptions::default()).await;
    assert!(matches!(early, Err(Halt::Cancelled)), "{early:?}");
    stop(engine, server).await;
}

/// The engine's inventory stops at its caller's flag as a reader does:
/// set once the Object_List is in, what was read is kept; set before, it
/// is a cancel, not a panic. The device's refusal of RPM is still
/// remembered after a stop.
#[tokio::test(start_paused = true)]
async fn the_engine_stops_at_the_callers_flag() {
    let flag = Arc::new(AtomicBool::new(false));
    let faults = Faults {
        // The name, the list, the refused RPM, then three ReadProperty of
        // the analog input: the flag goes up as the third leaves.
        cancel: Some((6, Arc::clone(&flag))),
        reject_rpm: true,
        ..Faults::default()
    };
    let (engine, server) = lab(faults).await;
    let inventory = engine
        .inventory(LAB, &InventoryOptions::default(), &flag)
        .await
        .unwrap();
    assert_eq!(inventory.objects.len(), 1, "{inventory:?}");
    let stopped = inventory.stopped.as_deref().unwrap();
    assert!(stopped.starts_with("cancelled before "), "{stopped}");
    assert_eq!(engine.limits().recent(), 6);
    assert!(engine.refused_rpm(LAB));
    stop(engine, server).await;

    let flag = Arc::new(AtomicBool::new(false));
    let faults = Faults {
        cancel: Some((1, Arc::clone(&flag))),
        ..Faults::default()
    };
    let (engine, server) = lab(faults).await;
    let early = engine
        .inventory(LAB, &InventoryOptions::default(), &flag)
        .await;
    assert!(matches!(early, Err(Halt::Cancelled)), "{early:?}");
    assert_eq!(engine.limits().recent(), 1);
    stop(engine, server).await;
}

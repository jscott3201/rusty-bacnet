//! The inventory against the lab device: what it reads of every object,
//! what each way of reading costs on the wire (counted on the test
//! transport's send log), and what a refusal, a silent device or a stop
//! leaves.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};

use super::test_lab::{
    Faults, LAB, count, lab, lab_at, lab_device, lab_logged, oid, passing, refusing_rpm,
    serve_logged, services, stop,
};
use super::*;
use crate::client::{Halt, ReadVia, Reader};
use crate::format::Datum;

/// ReadProperty and ReadPropertyMultiple, as the send log names them.
const RP: u8 = 12;
const RPM: u8 = 14;

/// The lab's objects in Object_List order: the Device, then each as built.
fn lab_objects() -> Vec<ObjectIdentifier> {
    use ObjectType as O;
    vec![
        oid(O::DEVICE, LAB),
        oid(O::ANALOG_INPUT, 1),
        oid(O::ANALOG_OUTPUT, 1),
        oid(O::ANALOG_VALUE, 1),
        oid(O::ANALOG_VALUE, 2),
        oid(O::BINARY_INPUT, 1),
        oid(O::BINARY_OUTPUT, 1),
        oid(O::BINARY_VALUE, 1),
        oid(O::MULTI_STATE_INPUT, 1),
        oid(O::MULTI_STATE_OUTPUT, 1),
        oid(O::MULTI_STATE_VALUE, 1),
        oid(O::INTEGER_VALUE, 1),
        oid(O::TREND_LOG, 1),
    ]
}

/// The properties `options` asks of all the lab's objects together: each
/// object's list, less the Device object's Object_Name, which the summary
/// read.
fn lab_properties(options: &InventoryOptions) -> usize {
    let listed: usize = lab_objects()
        .iter()
        .map(|object| properties(object.object_type(), options).len())
        .sum();
    listed - 1
}

/// The service request of the `n`th confirmed request in `log`, from 0.
fn request_octets(log: &bacnet_transport::test_transport::SendLog, n: usize) -> Vec<u8> {
    let frames = log.frames();
    let mut requests = frames
        .iter()
        .filter_map(|frame| super::test_lab::request(&frame.npdu).map(|(_, _, rest)| rest));
    requests.nth(n).expect("that many requests").to_vec()
}

/// Every object of the device is a record, of every type, the Device
/// object and the trend log included, in Object_List order: four requests
/// for thirteen objects (the summary, the Object_List, two batches).
/// Device text comes back exactly; no value is read.
#[tokio::test(start_paused = true)]
async fn every_object_is_a_record() {
    let (client, server, log) = lab_logged(Faults::default()).await;
    let device = lab_device(&client).await;
    let inventory = device.inventory(&InventoryOptions::new()).await.unwrap();
    assert_eq!(services(&log), [RPM, RP, RPM, RPM], "{inventory:?}");
    assert_eq!(client.limits().recent(), 4);
    assert!(inventory.stopped.is_none() && inventory.unread.is_empty());
    assert_eq!(inventory.address, lab_at());
    assert_eq!(inventory.device.device, LAB);
    assert_eq!(inventory.device.name().as_deref(), Some("Lab \"AHU\""));
    assert_eq!(inventory.device.via, ReadVia::Rpm);
    let objects: Vec<_> = inventory.points.iter().map(|point| point.object).collect();
    assert_eq!(objects, lab_objects());
    for point in &inventory.points {
        assert_eq!((&point.device, point.device_instance), (&lab_at(), LAB));
        assert!(point.problems.is_empty(), "{point:?}");
        assert!(point.present_value.is_none() && point.status_flags.is_none());
        assert!(point.relinquish_default.is_none() && point.cov_increment.is_none());
    }
    let point = |object_type, instance| inventory.point(oid(object_type, instance)).unwrap();

    let ai = point(ObjectType::ANALOG_INPUT, 1);
    assert_eq!(ai.name.as_deref(), Some("Zone Temp"));
    assert_eq!(ai.description.as_deref(), Some("Room 101"));
    assert_eq!(ai.units, Some(EngineeringUnits::DEGREES_FAHRENHEIT));
    assert!(!ai.commandable && ai.number_of_states.is_none());
    assert_eq!(ai.state_texts, StateTexts::default());

    // An output is commandable by its type; a value by its Priority_Array.
    assert!(point(ObjectType::ANALOG_OUTPUT, 1).commandable);
    assert!(point(ObjectType::ANALOG_VALUE, 1).commandable);
    let writable = point(ObjectType::ANALOG_VALUE, 2);
    assert!(!writable.commandable, "no Priority_Array: {writable:?}");
    assert!(point(ObjectType::BINARY_VALUE, 1).commandable);
    assert!(!point(ObjectType::BINARY_INPUT, 1).commandable);
    assert!(!point(ObjectType::INTEGER_VALUE, 1).commandable);

    let bo = point(ObjectType::BINARY_OUTPUT, 1);
    assert_eq!(bo.state_texts.active.as_deref(), Some("Active"));
    assert_eq!(bo.state_texts.inactive.as_deref(), Some("Inactive"));
    let msv = point(ObjectType::MULTI_STATE_VALUE, 1);
    assert_eq!(msv.number_of_states, Some(3));
    assert_eq!(msv.state_texts.states, ["Off", "Low", "High"]);
    assert_eq!(
        point(ObjectType::MULTI_STATE_INPUT, 1).number_of_states,
        Some(4)
    );

    // A type the inventory has no list for still has its name.
    let log_object = point(ObjectType::TREND_LOG, 1);
    assert_eq!(log_object.name.as_deref(), Some("Log"));
    assert!(log_object.units.is_none() && !log_object.commandable);
    // The Device object's own record has its name from the summary: the
    // first batch asks only for its Description (property 28).
    let own = point(ObjectType::DEVICE, LAB);
    assert_eq!(own.name.as_deref(), Some("Lab \"AHU\""));
    assert_eq!(own.description.as_deref(), Some(""));
    let first_batch = request_octets(&log, 2);
    // Device 2436, then the opening tag, the one property and the closing.
    let asked = [0x0C, 0x02, 0x00, 0x09, 0x84, 0x1E, 0x09, 28, 0x1F];
    assert!(first_batch.starts_with(&asked), "{first_batch:02x?}");

    // The summary's list of services is noted, at no request of its own.
    assert_eq!(inventory.cov, CovSupport::Offered);
    assert_eq!(device.cov_support(), CovSupport::Offered);
    assert!(!client.refused_rpm(&lab_at()));
    drop(device);
    stop(client, server).await;
}

/// `types` narrows the reading: the other objects cost nothing, and an
/// empty list reads the summary and the Object_List alone.
#[tokio::test(start_paused = true)]
async fn types_narrow_what_is_read() {
    let (client, server, log) = lab_logged(Faults::default()).await;
    let device = lab_device(&client).await;
    let types = [ObjectType::MULTI_STATE_VALUE, ObjectType::ANALOG_VALUE];
    let options = InventoryOptions::new().with_types(types);
    let inventory = device.inventory(&options).await.unwrap();
    let objects: Vec<_> = inventory.points.iter().map(|point| point.object).collect();
    let expected = [
        oid(ObjectType::ANALOG_VALUE, 1),
        oid(ObjectType::ANALOG_VALUE, 2),
        oid(ObjectType::MULTI_STATE_VALUE, 1),
    ];
    assert_eq!(objects, expected, "in Object_List order");
    assert_eq!(services(&log), [RPM, RP, RPM]);

    let none = InventoryOptions::new().with_types([]);
    let inventory = device.inventory(&none).await.unwrap();
    assert!(inventory.points.is_empty() && inventory.stopped.is_none());
    assert_eq!(services(&log).len(), 3 + 2);
    drop(device);
    stop(client, server).await;
}

/// With values each object's Present_Value and Status_Flags come in the
/// same requests, two more properties an object, so the thirteen objects
/// take three batches instead of two; the texts name units and states.
/// Without, neither is asked for. Settings add the range and the
/// relinquish default.
#[tokio::test(start_paused = true)]
async fn values_and_settings_are_read_when_asked() {
    let (client, server, log) = lab_logged(Faults::default()).await;
    let device = lab_device(&client).await;
    let plain = device.inventory(&InventoryOptions::new()).await.unwrap();
    assert_eq!(count(&log, RPM), 1 + 2);
    assert!(
        plain
            .points
            .iter()
            .all(|point| point.present_value.is_none())
    );

    let with_values = InventoryOptions::new().with_values(true);
    let inventory = device.inventory(&with_values).await.unwrap();
    assert_eq!(count(&log, RPM), 3 + 1 + 3, "{inventory:?}");
    assert_eq!(count(&log, RP), 2, "the Object_List, twice");
    let point = |object_type, instance| inventory.point(oid(object_type, instance)).unwrap();
    let ai = point(ObjectType::ANALOG_INPUT, 1);
    let value = ai.present_value.as_ref().unwrap();
    assert_eq!(value.as_f64(), Some(0.0));
    assert_eq!(value.text, "0 °F", "the units are named");
    let flags = ai.status_flags.as_ref().unwrap();
    assert!(matches!(flags.data, Datum::Bits { .. }), "{flags:?}");
    let bo = point(ObjectType::BINARY_OUTPUT, 1)
        .present_value
        .as_ref()
        .unwrap();
    assert_eq!(bo.as_bool(), Some(false));
    assert_eq!(bo.text, "inactive (Inactive)", "the state is named");
    let msv = point(ObjectType::MULTI_STATE_VALUE, 1);
    assert_eq!(msv.present_value.as_ref().unwrap().text, "1 (Off)");
    // A trend log has no Present_Value: absent, and no problem.
    let trend = point(ObjectType::TREND_LOG, 1);
    assert!(trend.present_value.is_none(), "{trend:?}");
    assert!(trend.problem(PropertyIdentifier::PRESENT_VALUE).is_none());
    // Everything but the values is as the plain inventory read it.
    for (valued, plain) in inventory.points.iter().zip(&plain.points) {
        let mut valued = valued.clone();
        valued.present_value = None;
        valued.status_flags = None;
        assert_eq!(&valued, plain);
    }

    let settings = InventoryOptions::new().with_settings(true);
    let inventory = device.inventory(&settings).await.unwrap();
    let av = inventory.point(oid(ObjectType::ANALOG_VALUE, 1)).unwrap();
    assert_eq!(av.relinquish_default.as_ref().unwrap().as_f64(), Some(0.0));
    assert_eq!(av.cov_increment.as_ref().unwrap().as_f64(), Some(0.0));
    assert!(av.present_value.is_none());
    let ai = inventory.point(oid(ObjectType::ANALOG_INPUT, 1)).unwrap();
    assert!(ai.relinquish_default.is_none(), "an input has none");
    drop(device);
    stop(client, server).await;
}

/// A device that refuses ReadPropertyMultiple costs the one refused
/// request, then a ReadProperty a property: thirteen for the summary, the
/// Object_List, and each object's own list. The records are the ones RPM
/// gives, and the client remembers the refusal.
#[tokio::test(start_paused = true)]
async fn by_read_property_when_rpm_is_refused() {
    let (client, server) = lab(Faults::default()).await;
    let by_rpm = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    stop(client, server).await;

    let (client, server, log) = lab_logged(refusing_rpm()).await;
    let device = lab_device(&client).await;
    let options = InventoryOptions::new();
    let inventory = device.inventory(&options).await.unwrap();
    assert_eq!(count(&log, RPM), 1, "refused once");
    assert_eq!(count(&log, RP), 13 + 1 + lab_properties(&options));
    assert_eq!(lab_properties(&options), 47);
    assert!(client.refused_rpm(&lab_at()));
    assert_eq!(inventory.device.via, ReadVia::Rp);
    assert_eq!(inventory.points, by_rpm.points);
    assert_eq!(inventory.device.readings, by_rpm.device.readings);

    // The refusal is remembered: the next inventory sends no RPM at all.
    let again = device.inventory(&options).await.unwrap();
    assert_eq!(count(&log, RPM), 1);
    assert_eq!(again.points, by_rpm.points);
    drop(device);
    stop(client, server).await;
}

/// A device that fails every RPM with an error of its own, not a refusal
/// of the service, is read by ReadProperty all the same, and RPM is not
/// given up for it: each batch is tried by RPM first.
#[tokio::test(start_paused = true)]
async fn a_failed_rpm_is_read_by_read_property() {
    let faults = Faults {
        error_rpm: true,
        ..Faults::default()
    };
    let (client, server, log) = lab_logged(faults).await;
    let inventory = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    assert_eq!(inventory.points.len(), 13);
    assert!(inventory.points.iter().all(|p| p.problems.is_empty()));
    assert_eq!(inventory.device.name().as_deref(), Some("Lab \"AHU\""));
    assert_eq!(count(&log, RPM), 1 + 2, "the summary's and each batch's");
    assert_eq!(count(&log, RP), 13 + 1 + 47);
    assert!(!client.refused_rpm(&lab_at()), "an error isn't a refusal");
    stop(client, server).await;
}

/// A device that stops answering part way ends the reading: the records
/// read are kept, the objects not read are listed, `stopped` says why,
/// and nothing is sent after the timeout. One that never answers fails
/// the call, at its summary.
#[tokio::test(start_paused = true)]
async fn a_device_that_stops_answering() {
    // The summary, the Object_List and the first batch pass.
    let (client, server, log) = lab_logged(passing(3)).await;
    let inventory = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    let objects = lab_objects();
    let read: Vec<_> = inventory.points.iter().map(|point| point.object).collect();
    assert_eq!(read, objects[..9], "the first batch's objects");
    assert_eq!(inventory.unread, objects[9..]);
    let stopped = inventory.stopped.as_ref().unwrap();
    assert!(stopped.unreachable() && stopped.timed_out(), "{stopped}");
    assert_eq!(
        services(&log),
        [RPM, RP, RPM, RPM],
        "none after the timeout"
    );
    stop(client, server).await;

    // By ReadProperty, an object read only in part is not a record: the
    // refused RPM, 13 + 1, then the Device's Description and one of the
    // input's three.
    let faults = Faults {
        pass: 1 + 14 + 1 + 1,
        ..refusing_rpm()
    };
    let (client, server) = lab(faults).await;
    let inventory = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    assert_eq!(inventory.points.len(), 1, "{inventory:?}");
    assert_eq!(inventory.unread, objects[1..]);
    assert!(inventory.stopped.is_some());
    stop(client, server).await;

    let (client, server) = lab(passing(0)).await;
    let error = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap_err();
    assert!(error.unreachable(), "{error}");
    assert_eq!(client.limits().recent(), 2, "the summary, and one Who-Is");
    stop(client, server).await;
}

/// An object the device's list names and the device then refuses is a
/// record of problems, one a property, with the device's error as data;
/// the inventory goes on and its other records are whole.
#[tokio::test(start_paused = true)]
async fn a_refused_object_is_a_record_of_problems() {
    let gone = oid(ObjectType::ANALOG_OUTPUT, 1);
    let faults = Faults {
        unknown_object: Some(gone),
        ..refusing_rpm()
    };
    let (client, server) = lab(faults).await;
    let inventory = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    assert!(inventory.stopped.is_none() && inventory.unread.is_empty());
    assert_eq!(inventory.points.len(), 13);
    let refused = inventory.point(gone).unwrap();
    assert!(refused.name.is_none() && refused.units.is_none());
    let asked = properties(ObjectType::ANALOG_OUTPUT, &InventoryOptions::new());
    let problems: Vec<_> = refused.problems.iter().map(|p| p.property).collect();
    assert_eq!(problems, asked, "each property asked for");
    for problem in &refused.problems {
        let unknown = (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
        assert_eq!(problem.error.class_code(), Some(unknown));
    }
    assert!(refused.to_point().is_none());
    let others = inventory.points.iter().filter(|point| point.object != gone);
    assert!(others.clone().all(|point| point.problems.is_empty()));
    assert_eq!(others.count(), 12);
    stop(client, server).await;
}

/// An Object_List read element by element that stops early ends the
/// reading there: no object is asked for, and `stopped` says why.
#[tokio::test(start_paused = true)]
async fn a_list_that_stops_early_ends_the_reading() {
    // The summary, the refused whole list, element 0, elements 1 and 2;
    // then element 3 goes unanswered.
    let faults = Faults {
        pass: 5,
        reject_whole_list: true,
        ..Faults::default()
    };
    let (client, server, log) = lab_logged(faults).await;
    let inventory = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    assert!(inventory.points.is_empty() && inventory.unread.is_empty());
    assert!(inventory.stopped.as_ref().unwrap().unreachable());
    assert_eq!(services(&log).len(), 6, "no object asked for");
    stop(client, server).await;
}

/// A reader whose stop flag goes up is cancelled, between calls.
#[tokio::test(start_paused = true)]
async fn a_stop_flag_cancels_the_reading() {
    let flag = Arc::new(AtomicBool::new(false));
    let faults = Faults {
        cancel: Some((2, Arc::clone(&flag))),
        ..Faults::default()
    };
    let (client, server, log) = lab_logged(faults).await;
    let mut reader = Reader {
        client: client.low_level(),
        device: lab_at(),
        instance: Some(LAB),
        rate: client.limits(),
        cancel: &flag,
        rpm: true,
    };
    // The flag goes up as the Object_List's request leaves.
    let read = read_inventory(&mut reader, &InventoryOptions::new()).await;
    assert!(matches!(read, Err(Halt::Cancelled)), "{read:?}");
    assert_eq!(services(&log), [RPM, RP], "no object asked for");
    stop(client, server).await;
}

/// One object's record costs one request and is the inventory's own; an
/// object the device doesn't have is a record of problems, and a silent
/// device an error.
#[tokio::test(start_paused = true)]
async fn one_record_reads_as_the_inventory_does() {
    let (client, server, log) = lab_logged(Faults::default()).await;
    let device = lab_device(&client).await;
    let options = InventoryOptions::new().with_values(true);
    let inventory = device.inventory(&options).await.unwrap();
    let before = services(&log).len();
    for expected in &inventory.points {
        let read = device.point_record(expected.object, &options).await;
        assert_eq!(&read.unwrap(), expected);
    }
    assert_eq!(services(&log).len() - before, inventory.points.len());
    let by_text = device.point_record("msv:1", &options).await.unwrap();
    assert_eq!(by_text.object, oid(ObjectType::MULTI_STATE_VALUE, 1));

    let missing = device.point_record("ai:99", &options).await.unwrap();
    assert!(missing.name.is_none());
    assert_eq!(missing.problems.len(), 5, "{missing:?}");
    // A record read on its own asks for everything, the Device's name too.
    let own = device.point_record(oid(ObjectType::DEVICE, LAB), &options);
    assert_eq!(own.await.unwrap().name.as_deref(), Some("Lab \"AHU\""));
    let parse = device.point_record("nothing", &options).await.unwrap_err();
    assert!(matches!(parse, OpsError::Parse { .. }), "{parse}");
    drop(device);
    stop(client, server).await;

    let (client, server) = lab(passing(0)).await;
    let device = lab_device(&client).await;
    let error = device.point_record("ai:1", &options).await.unwrap_err();
    assert!(error.unreachable(), "{error}");
    drop(device);
    stop(client, server).await;
}

/// Forty analog inputs, each `name` a few characters.
fn inputs() -> Vec<Box<dyn BACnetObject>> {
    (1..=40)
        .map(|n| {
            let units = EngineeringUnits::DEGREES_CELSIUS;
            let input = AnalogInputObject::new(n, format!("T{n}"), units).unwrap();
            Box::new(input) as Box<dyn BACnetObject>
        })
        .collect()
}

/// A device that can't send a large answer: the batch is asked again an
/// object at a time, and the calls after it ask for half as many objects,
/// until one fits. Forty-one objects cost three refused batches, not one
/// for every batch of the inventory.
#[tokio::test(start_paused = true)]
async fn a_device_with_small_answers_gets_smaller_batches() {
    // A request for one object's three properties is 13 octets: four
    // objects fit under 60, eight don't.
    let faults = Faults {
        abort_longer: Some(60),
        ..Faults::default()
    };
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let database = build_database_with_extra(&identity, inputs()).unwrap();
    let (client, server, log) = serve_logged(faults, identity, database).await;
    let inventory = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    assert_eq!(inventory.points.len(), 41, "{:?}", inventory.stopped);
    assert!(inventory.points.iter().all(|p| p.problems.is_empty()));
    // The first call's sixteen objects: two batches (eleven and five) refused,
    // then sixteen requests. The second's eight: one refused, then eight.
    // From then on four a call, each one request: the seventeen left take
    // five.
    let objects = (2 + 16) + (1 + 8) + 5;
    assert_eq!(
        services(&log).len(),
        1 + 1 + objects,
        "{:?}",
        services(&log)
    );
    assert!(!client.refused_rpm(&lab_at()));
    stop(client, server).await;
}

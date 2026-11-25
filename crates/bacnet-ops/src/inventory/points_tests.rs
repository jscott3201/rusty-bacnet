//! `Device::point_records` (#2785) against the lab device: the inventory's
//! own records for the objects asked, in the order asked, at the
//! inventory's batching, with its fallbacks, and what a device going
//! silent part way leaves. Requests are counted on the test transport's
//! send log.

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};

use super::test_lab::{
    Answering, Faults, LAB, lab_at, lab_device, lab_logged, objects as lab_database, oid, passing,
    refusing_rpm, serve_logged, services, stop, text,
};
use super::*;

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

/// Every combination of the options worth telling apart.
fn every_option() -> Vec<InventoryOptions> {
    let plain = InventoryOptions::new();
    vec![
        plain.clone(),
        plain.clone().with_values(true),
        plain.clone().with_settings(true),
        plain.clone().with_details(true),
        plain
            .with_values(true)
            .with_settings(true)
            .with_details(true),
    ]
}

/// How many of `objects`, from the first, one ReadPropertyMultiple holds
/// by the rule the module docs state: up to 16 objects or 32 properties,
/// at least one object.
fn first_batch(objects: &[ObjectIdentifier], options: &InventoryOptions) -> usize {
    let (mut count, mut total) = (0, 0);
    for object in objects {
        let more = properties(object.object_type(), options).len();
        if count == 16 || (count > 0 && total + more > 32) {
            break;
        }
        count += 1;
        total += more;
    }
    count
}

/// The ReadPropertyMultiple requests `objects` take under `options`, a
/// [`first_batch`] at a time.
fn batches(objects: &[ObjectIdentifier], options: &InventoryOptions) -> usize {
    let mut requests = 0;
    let mut at = 0;
    while at < objects.len() {
        at += first_batch(&objects[at..], options);
        requests += 1;
    }
    requests
}

/// `count` analog inputs, each `name` a few characters.
fn inputs(count: u32) -> Vec<Box<dyn BACnetObject>> {
    (1..=count)
        .map(|n| {
            let units = EngineeringUnits::DEGREES_CELSIUS;
            let input = AnalogInputObject::new(n, format!("T{n}"), units).unwrap();
            Box::new(input) as Box<dyn BACnetObject>
        })
        .collect()
}

/// Under every option the records of the device's objects are the
/// inventory's, one ReadPropertyMultiple a batch and nothing else: no
/// summary, no Object_List. With no option the thirteen objects take two
/// (8 objects of 2 + 3 + 3 + 4 + 4 + 4 + 4 + 5 = 29 properties, then 5 of
/// 19), with values four (4 + 3 + 3 + 3 objects, each 4 or 5 properties
/// more). The Device object's record reads its name itself, the same name
/// the inventory takes from its summary.
#[tokio::test(start_paused = true)]
async fn the_records_are_the_inventorys() {
    let plain = InventoryOptions::new();
    assert_eq!(batches(&lab_objects(), &plain), 2);
    assert_eq!(batches(&lab_objects(), &plain.clone().with_values(true)), 4);
    let (client, server, log) = lab_logged(Faults::default()).await;
    let device = lab_device(&client).await;
    for options in every_option() {
        let inventory = device.inventory(&options).await.unwrap();
        let objects: Vec<_> = inventory.points.iter().map(|point| point.object).collect();
        assert_eq!(objects, lab_objects());
        let batch = device.point_batch(&objects, &options);
        assert_eq!(batch, first_batch(&objects, &options), "{options:?}");
        let before = services(&log).len();
        let read = device.point_records(&objects, &options).await.unwrap();
        let sent = &services(&log)[before..];
        assert!(sent.iter().all(|&service| service == RPM), "{sent:?}");
        assert_eq!(sent.len(), batches(&objects, &options), "{options:?}");
        assert_eq!(read.points, inventory.points, "{options:?}");
        assert!(read.unread.is_empty() && read.stopped.is_none());
    }
    drop(device);
    stop(client, server).await;
}

/// The records come in the order asked, whatever it is, an object named
/// twice read twice; a page from the middle of the list is that part of
/// the inventory; objects may be named by their text. Nothing asked, or a
/// name that names no object, and nothing is sent.
#[tokio::test(start_paused = true)]
async fn records_come_in_the_order_asked() {
    let (client, server, log) = lab_logged(Faults::default()).await;
    let device = lab_device(&client).await;
    let options = InventoryOptions::new().with_values(true);
    let inventory = device.inventory(&options).await.unwrap();
    let mut objects = lab_objects();
    objects.reverse();
    objects.push(objects[2]);
    let read = device.point_records(&objects, &options).await.unwrap();
    let got: Vec<_> = read.points.iter().map(|point| point.object).collect();
    assert_eq!(got, objects);
    for point in &read.points {
        assert_eq!(Some(point), inventory.point(point.object));
    }
    let page = &lab_objects()[3..9];
    let read = device.point_records(page, &options).await.unwrap();
    assert_eq!(read.points, inventory.points[3..9]);

    // Objects by their text, as anywhere on a handle.
    let by_text = device.point_records(["msv:1", "ai:1"], &options).await;
    let got: Vec<_> = by_text.unwrap().points.iter().map(|p| p.object).collect();
    let (msv, ai) = (ObjectType::MULTI_STATE_VALUE, ObjectType::ANALOG_INPUT);
    assert_eq!(got, [oid(msv, 1), oid(ai, 1)]);

    let before = services(&log).len();
    let none: [ObjectIdentifier; 0] = [];
    let none = device.point_records(none, &options).await.unwrap();
    assert!(none.points.is_empty() && none.unread.is_empty() && none.stopped.is_none());
    assert_eq!(device.point_batch(&[], &options), 0);
    // Text that names no object fails the call before anything is sent.
    let parse = device.point_records(["ai:1", "nothing"], &options).await;
    assert!(matches!(parse, Err(OpsError::Parse { .. })), "{parse:?}");
    assert_eq!(services(&log).len(), before, "nothing sent");
    drop(device);
    stop(client, server).await;
}

/// A property the device refuses or gives wrong, and an object it doesn't
/// have, are problems on their own records, as in the inventory; the
/// objects read with them in the same request are whole.
#[tokio::test(start_paused = true)]
async fn problems_are_the_inventorys() {
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let odd = AnalogInputObject::new(20, "Odd", EngineeringUnits::PERCENT).unwrap();
    let answers = vec![
        (PropertyIdentifier::UNITS, Ok(text("degrees"))),
        (
            PropertyIdentifier::DESCRIPTION,
            Err((ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED)),
        ),
    ];
    let mut objects = lab_database();
    objects.push(Answering::wrap(Box::new(odd), answers));
    let database = build_database_with_extra(&identity, objects).unwrap();
    let (client, server, log) = serve_logged(Faults::default(), identity, database).await;
    let device = lab_device(&client).await;
    let options = InventoryOptions::new();
    let inventory = device.inventory(&options).await.unwrap();
    let (odd, gone) = (
        oid(ObjectType::ANALOG_INPUT, 20),
        oid(ObjectType::ANALOG_INPUT, 99),
    );
    let asked = [
        oid(ObjectType::ANALOG_INPUT, 1),
        odd,
        gone,
        oid(ObjectType::MULTI_STATE_VALUE, 1),
    ];
    let before = services(&log).len();
    let read = device.point_records(&asked, &options).await.unwrap();
    assert_eq!(services(&log)[before..], [RPM], "one request");
    assert_eq!(read.points[1], *inventory.point(odd).unwrap());
    let problems: Vec<_> = read.points[1].problems.iter().map(|p| p.property).collect();
    assert_eq!(
        problems,
        [PropertyIdentifier::DESCRIPTION, PropertyIdentifier::UNITS]
    );
    let missing = &read.points[2];
    assert!(missing.name.is_none());
    let problems: Vec<_> = missing.problems.iter().map(|p| p.property).collect();
    assert_eq!(problems, properties(ObjectType::ANALOG_INPUT, &options));
    for at in [0, 3] {
        assert_eq!(Some(&read.points[at]), inventory.point(asked[at]));
        assert!(read.points[at].problems.is_empty());
    }
    drop(device);
    stop(client, server).await;
}

/// A device that can't send a batch's answer for its size: the batch is
/// asked again an object at a time, and the client remembers half the
/// batch's properties for the device, so the next call starts there and
/// the call after a second refusal at half again (30 properties refused,
/// then 15, leaving 7: ten inputs, five, two, as an inventory's batches
/// shrink; 11 + 6 + 13 requests for forty inputs). The records are whole.
#[tokio::test(start_paused = true)]
async fn a_batch_too_large_shrinks_the_next_calls() {
    // A request for one input's three properties is 13 octets: four
    // inputs fit under 60, five don't.
    let faults = Faults {
        abort_longer: Some(60),
        ..Faults::default()
    };
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let database = build_database_with_extra(&identity, inputs(40)).unwrap();
    let (client, server, log) = serve_logged(faults, identity, database).await;
    let device = lab_device(&client).await;
    let options = InventoryOptions::new();
    let objects: Vec<_> = (1..=40).map(|n| oid(ObjectType::ANALOG_INPUT, n)).collect();
    let mut read = Vec::new();
    let mut costs = Vec::new();
    let mut at = 0;
    while at < objects.len() {
        let batch = device.point_batch(&objects[at..], &options);
        let before = services(&log).len();
        let page = device
            .point_records(&objects[at..at + batch], &options)
            .await;
        let page = page.unwrap();
        assert!(page.stopped.is_none(), "{:?}", page.stopped);
        costs.push((batch, services(&log).len() - before));
        at += page.points.len();
        read.extend(page.points);
    }
    // Ten refused, then one at a time; five refused, then one at a time;
    // then two a request.
    let mut expected = vec![(10, 1 + 10), (5, 1 + 5)];
    expected.extend([(2, 1); 12]);
    expected.push((1, 1));
    assert_eq!(costs, expected);
    assert!(services(&log).iter().all(|&service| service == RPM));
    assert!(read.iter().all(|point| point.problems.is_empty()));
    assert_eq!(read.len(), 40);
    assert_eq!(read[39].name.as_deref(), Some("T40"));
    assert!(!client.refused_rpm(&lab_at()));
    assert_eq!(client.rpm_limit(&lab_at()), Some(7));
    drop(device);
    stop(client, server).await;
}

/// A device that refuses ReadPropertyMultiple: the first call's batch
/// costs the refused request, then a ReadProperty a property; the client
/// remembers the refusal, so the next call sends no RPM and a batch is one
/// object. The records are the ones RPM gives.
#[tokio::test(start_paused = true)]
async fn a_device_without_rpm_is_read_a_property_a_request() {
    let (client, server, _) = lab_logged(Faults::default()).await;
    let options = InventoryOptions::new();
    let by_rpm = lab_device(&client)
        .await
        .point_records(&lab_objects(), &options)
        .await
        .unwrap();
    stop(client, server).await;

    let (client, server, log) = lab_logged(refusing_rpm()).await;
    let device = lab_device(&client).await;
    let objects = lab_objects();
    assert_eq!(device.point_batch(&objects, &options), 8, "not known yet");
    let first = device.point_records(&objects[..8], &options).await.unwrap();
    // The refused request, then 2 + 3 + 3 + 4 + 4 + 4 + 4 + 5 properties.
    let mut expected = vec![RPM];
    expected.extend([RP; 29]);
    assert_eq!(services(&log), expected);
    assert!(client.refused_rpm(&lab_at()));
    assert_eq!(device.point_batch(&objects[8..], &options), 1);
    let rest = device.point_records(&objects[8..], &options).await.unwrap();
    // 4 + 4 + 5 + 4 + 2, and no RPM.
    assert_eq!(services(&log).len(), 1 + 29 + 19);
    let read: Vec<_> = first.points.into_iter().chain(rest.points).collect();
    assert_eq!(read, by_rpm.points);
    drop(device);
    stop(client, server).await;
}

/// A device that fails a whole ReadPropertyMultiple with an error of its
/// own has that batch read by ReadProperty, and RPM is tried again for
/// the next: one wasted request a batch, nothing remembered.
#[tokio::test(start_paused = true)]
async fn a_failed_batch_is_read_by_read_property() {
    let faults = Faults {
        error_rpm: true,
        ..Faults::default()
    };
    let (client, server, log) = lab_logged(faults).await;
    let device = lab_device(&client).await;
    let options = InventoryOptions::new();
    let read = device
        .point_records(&lab_objects(), &options)
        .await
        .unwrap();
    assert_eq!(read.points.len(), 13);
    assert!(read.points.iter().all(|point| point.problems.is_empty()));
    let sent = services(&log);
    assert_eq!(
        sent.iter().filter(|&&s| s == RPM).count(),
        2,
        "each batch's"
    );
    assert_eq!(sent.iter().filter(|&&s| s == RP).count(), 29 + 19);
    assert!(!client.refused_rpm(&lab_at()));
    assert_eq!(device.point_batch(&lab_objects(), &options), 8);
    drop(device);
    stop(client, server).await;
}

/// A device that stops answering part way leaves the records read before
/// it, in order, the rest as unread and why: by RPM the first batch's
/// eight; by ReadProperty the objects read whole, one read in part not a
/// record. Nothing is sent after the timeout. One that never answers is
/// stopped with no record, not an error.
#[tokio::test(start_paused = true)]
async fn a_device_that_stops_answering_keeps_what_was_read() {
    let options = InventoryOptions::new();
    let objects = lab_objects();
    let (client, server, log) = lab_logged(passing(1)).await;
    let device = lab_device(&client).await;
    let read = device.point_records(&objects, &options).await.unwrap();
    let got: Vec<_> = read.points.iter().map(|point| point.object).collect();
    assert_eq!(got, objects[..8]);
    assert_eq!(read.unread, objects[8..]);
    let stopped = read.stopped.as_ref().unwrap();
    assert!(stopped.unreachable() && stopped.timed_out(), "{stopped}");
    assert_eq!(services(&log), [RPM, RPM], "none after the timeout");
    drop(device);
    stop(client, server).await;

    // The refused RPM, the Device's two properties, one of the input's
    // three, then silence.
    let faults = Faults {
        pass: 1 + 2 + 1,
        ..refusing_rpm()
    };
    let (client, server) = super::test_lab::lab(faults).await;
    let device = lab_device(&client).await;
    let read = device.point_records(&objects, &options).await.unwrap();
    assert_eq!(read.points.len(), 1, "{read:?}");
    assert_eq!(read.unread, objects[1..]);
    assert!(read.stopped.is_some());
    drop(device);
    stop(client, server).await;

    let (client, server, log) = lab_logged(passing(0)).await;
    let device = lab_device(&client).await;
    let read = device.point_records(&objects, &options).await.unwrap();
    assert!(read.points.is_empty());
    assert_eq!(read.unread, objects);
    assert!(read.stopped.unwrap().unreachable());
    assert_eq!(services(&log), [RPM]);
    drop(device);
    stop(client, server).await;
}

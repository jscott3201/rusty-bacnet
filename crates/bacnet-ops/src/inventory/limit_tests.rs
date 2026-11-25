//! The client's memory of how large a ReadPropertyMultiple of point
//! records a device can answer (#2785, `Client::rpm_limit`): in
//! properties, so it carries from one set of options to another; only
//! shrinking; left alone by a timeout; another address's untouched; gone
//! when another device may answer at the address.

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{EngineeringUnits, ObjectType};

use super::test_lab::{Faults, LAB, lab, lab_at, lab_device, oid, passing, serve_logged, stop};
use super::*;
use crate::Client;

/// The lab device with `count` analog outputs, each named `V<n>`, whose
/// answer to a ReadPropertyMultiple request longer than 65 octets doesn't
/// fit. With no option an output asks for three properties, 13 octets of
/// request: five fit, six don't. With every option it asks for 14, 36
/// octets: one fits, two don't.
async fn outputs_lab(
    count: u32,
) -> (
    Client<TestTransport>,
    BACnetServer<LoopbackTransport>,
    SendLog,
) {
    let faults = Faults {
        abort_longer: Some(65),
        ..Faults::default()
    };
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let outputs = (1..=count).map(|n| {
        let units = EngineeringUnits::PERCENT;
        let output = AnalogOutputObject::new(n, format!("V{n}"), units).unwrap();
        Box::new(output) as Box<dyn BACnetObject>
    });
    let database = build_database_with_extra(&identity, outputs.collect()).unwrap();
    serve_logged(faults, identity, database).await
}

/// The ReadPropertyMultiple requests in `log` from the `from`th on, and
/// how many of them were longer than the lab answers.
fn rpms(log: &SendLog, from: usize) -> (usize, usize) {
    let frames = log.frames();
    let lengths: Vec<usize> = frames[from..]
        .iter()
        .filter_map(|frame| super::test_lab::request(&frame.npdu))
        .filter(|&(_, service, _)| service == 14)
        .map(|(_, _, rest)| rest.len())
        .collect();
    let refused = lengths.iter().filter(|&&length| length > 65).count();
    (lengths.len(), refused)
}

fn outputs(range: std::ops::RangeInclusive<u32>) -> Vec<ObjectIdentifier> {
    range.map(|n| oid(ObjectType::ANALOG_OUTPUT, n)).collect()
}

fn every_option() -> InventoryOptions {
    InventoryOptions::new()
        .with_values(true)
        .with_settings(true)
        .with_details(true)
}

/// A refusal with every option (two outputs, 28 properties) leaves a limit
/// of 14 properties, which with no option still takes four outputs a
/// request: plain pages aren't cut to one object each.
#[tokio::test(start_paused = true)]
async fn a_limit_learned_with_every_option_holds_for_plain_reads() {
    let (client, server, log) = outputs_lab(12).await;
    let device = lab_device(&client).await;
    let all = every_option();
    let objects = outputs(1..=12);
    assert_eq!(device.point_batch(&objects, &all), 2);
    let read = device.point_records(&objects[..2], &all).await.unwrap();
    assert_eq!(read.points.len(), 2);
    assert_eq!(rpms(&log, 0), (1 + 2, 1), "refused, then one at a time");
    assert_eq!(client.rpm_limit(&lab_at()), Some(14));
    let plain = InventoryOptions::new();
    assert_eq!(device.point_batch(&objects, &plain), 4);
    let before = log.frames().len();
    let read = device.point_records(&objects[..8], &plain).await.unwrap();
    assert_eq!(read.points.len(), 8);
    assert_eq!(
        rpms(&log, before),
        (2, 0),
        "two batches of four, none refused"
    );
    assert_eq!(read.points[7].name.as_deref(), Some("V8"));
    drop(device);
    stop(client, server).await;
}

/// A refusal with no option (ten outputs, 30 properties) leaves a limit of
/// 15, which with every option takes one output a request: no batch is
/// refused again for its size.
#[tokio::test(start_paused = true)]
async fn a_limit_learned_with_no_option_holds_for_every_option() {
    let (client, server, log) = outputs_lab(12).await;
    let device = lab_device(&client).await;
    let plain = InventoryOptions::new();
    let objects = outputs(1..=12);
    let read = device.point_records(&objects[..10], &plain).await.unwrap();
    assert_eq!(read.points.len(), 10);
    assert_eq!(rpms(&log, 0), (1 + 10, 1));
    assert_eq!(client.rpm_limit(&lab_at()), Some(15));
    assert_eq!(device.point_batch(&objects, &plain), 5);
    let all = every_option();
    assert_eq!(device.point_batch(&objects, &all), 1);
    let before = log.frames().len();
    let read = device.point_records(&objects[..3], &all).await.unwrap();
    assert!(read.points.iter().all(|point| point.problems.is_empty()));
    assert_eq!(
        rpms(&log, before),
        (3, 0),
        "one output a request, none refused"
    );
    drop(device);
    stop(client, server).await;
}

/// The limit only shrinks; it is the device's at its address alone; and a
/// device that doesn't answer, a timeout, leaves it as it was.
#[tokio::test(start_paused = true)]
async fn the_limit_only_shrinks_and_is_the_addresss_own() {
    let (client, server, _) = outputs_lab(4).await;
    let at = lab_at();
    let plain = InventoryOptions::new();
    // Another address's limit is its own.
    let other = DeviceAddress::local(&[3]).unwrap();
    client.settle_rpm_limit(&other, LAB + 1, 3);
    assert_eq!(client.rpm_limit(&other), Some(3));
    assert_eq!(client.rpm_limit(&at), None);
    let device = lab_device(&client).await;
    assert_eq!(device.point_batch(&outputs(1..=4), &plain), 4);
    client.settle_rpm_limit(&at, LAB, 14);
    client.settle_rpm_limit(&at, LAB, 20);
    assert_eq!(client.rpm_limit(&at), Some(14), "never grows");
    client.settle_rpm_limit(&at, LAB, 32);
    assert_eq!(client.rpm_limit(&at), Some(14));
    client.settle_rpm_limit(&at, LAB, 7);
    assert_eq!(client.rpm_limit(&at), Some(7));
    assert_eq!(device.point_batch(&outputs(1..=4), &plain), 2);
    assert_eq!(client.rpm_limit(&other), Some(3));
    drop(device);
    stop(client, server).await;

    // A device that never answers: the read stops, and the limit stays.
    let (client, server) = lab(passing(0)).await;
    client.settle_rpm_limit(&at, LAB, 14);
    let device = lab_device(&client).await;
    let read = device.point_records(&outputs(1..=4), &plain).await.unwrap();
    assert!(read.stopped.as_ref().is_some_and(OpsError::timed_out));
    assert_eq!(client.rpm_limit(&at), Some(14));
    drop(device);
    stop(client, server).await;
}

/// The limit goes when another device may answer at the address: a device
/// there announcing another instance. The same instance announced again
/// keeps it.
#[tokio::test(start_paused = true)]
async fn the_limit_goes_with_the_device_at_its_address() {
    use crate::device::test_bus::{Bus, N, at, client, settle};
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    client.settle_rpm_limit(&at(2), N, 14);
    client.settle_rpm_limit(&at(3), N + 1, 10);
    bus.i_am(2, N);
    settle().await;
    assert_eq!(client.rpm_limit(&at(2)), Some(14), "the same device");
    bus.i_am(2, 2002);
    settle().await;
    assert_eq!(client.rpm_limit(&at(2)), None, "another device there now");
    assert_eq!(
        client.rpm_limit(&at(3)),
        Some(10),
        "another address's stays"
    );
}

/// A limit below one object's properties still reads that one object.
#[test]
fn a_limit_below_one_object_still_reads_one() {
    use super::read::{Sizing, one_batch};
    let objects = outputs(1..=3);
    let one = one_batch(&objects, Sizing::pager(1), &every_option(), None);
    assert_eq!(one, (1, 14), "one output, all of its properties");
    let two = one_batch(&objects, Sizing::pager(28), &every_option(), None);
    assert_eq!(two, (2, 28));
}

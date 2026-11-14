//! More of [`Device::read_many`](crate::Device::read_many) and
//! [`Device::read_object`](crate::Device::read_object) on the scripted lab
//! link: answers that leave an object out, a device that stops part way
//! through an object, a request the device fails whole, an element out of
//! range, and a property asked for twice.

use bacnet_services::rpm::{
    ReadAccessResult, ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, RejectReason};
use bytes::BytesMut;

use super::read_many_tests::{Script, Service, forwards, remote, rpms, rps, scripted};
use super::tests::{lab_at, lab_device, lab_on, oid, stop};
use super::*;
use crate::format::{Datum, PropertyError};

/// A REAL that names its object: ten times its type's number, plus its
/// instance.
fn marker(object: bacnet_types::primitives::ObjectIdentifier) -> f32 {
    (object.object_type().to_raw() * 10 + object.instance_number()) as f32
}

/// The lab link answering every RPM itself, leaving the analog output out
/// of its answer and giving every other property asked for the marker
/// REAL of its object.
fn leaves_out_the_output(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |service, _, data| {
        if service != Service::Rpm {
            return Script::Forward;
        }
        let request = ReadPropertyMultipleRequest::decode(data).expect("an RPM request");
        let left_out = oid(ObjectType::ANALOG_OUTPUT, 1);
        let results = request
            .list_of_read_access_specs
            .iter()
            .filter(|spec| spec.object_identifier != left_out)
            .map(|spec| ReadAccessResult {
                object_identifier: spec.object_identifier,
                list_of_results: spec
                    .list_of_property_references
                    .iter()
                    .map(|reference| {
                        let mut value = BytesMut::new();
                        bacnet_encoding::primitives::encode_property_value(
                            &mut value,
                            &bacnet_types::primitives::PropertyValue::Real(marker(
                                spec.object_identifier,
                            )),
                        )
                        .unwrap();
                        ReadResultElement {
                            property_identifier: reference.property_identifier,
                            property_array_index: reference.property_array_index,
                            property_value: Some(value.to_vec()),
                            error: None,
                        }
                    })
                    .collect(),
            })
            .collect();
        let mut ack = BytesMut::new();
        ReadPropertyMultipleACK {
            list_of_read_access_results: results,
        }
        .encode(&mut ack)
        .unwrap();
        Script::Ack(ack.to_vec())
    })
}

/// An object the device's answer leaves out reads as no value, and the
/// objects after it keep their own values, not its.
#[tokio::test(start_paused = true)]
async fn an_object_left_out_of_an_answer_is_missing_not_shifted() {
    let (client, server) = lab_on(leaves_out_the_output).await;
    let device = lab_device(&client).await;
    let read = device
        .read_many(["ai:1 pv", "ao:1 pv", "bv:2 pv", "bv:2 units"])
        .await
        .unwrap();
    assert!(read.stopped.is_none(), "{read:?}");
    assert_eq!(rpms(client.low_level().transport()), 1);
    let values: Vec<_> = read
        .readings
        .iter()
        .map(|r| r.value.as_ref().map(|v| v.data.clone()))
        .collect();
    assert_eq!(values[0], Ok(Datum::Real(1.0)), "analog-input 1");
    assert_eq!(values[1], Err(&PropertyError::NoValue), "left out");
    assert_eq!(values[2], Ok(Datum::Real(52.0)), "binary-value 2, its own");
    assert_eq!(values[3], Ok(Datum::Real(52.0)));
    drop(device);
    stop(client, server).await;
}

/// The lab link without RPM whose device answers two ReadProperty
/// requests and then nothing.
fn stops_after_two_reads(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |service, n, _| match (service, n) {
        (Service::Rpm, _) => Script::Reject(RejectReason::UNRECOGNIZED_SERVICE),
        (Service::Rp, 0 | 1) => Script::Forward,
        (Service::Rp, _) => Script::Silent,
    })
}

/// A device that stops answering part way through an object read
/// property by property keeps that object's properties read before it.
#[tokio::test(start_paused = true)]
async fn a_stop_part_way_through_an_object_keeps_what_was_read() {
    let (client, server) = lab_on(stops_after_two_reads).await;
    let device = lab_device(&client).await;
    let read = device
        .read_many(["ai:1 pv", "ai:1 units", "ai:1 description", "ao:1 pv"])
        .await
        .unwrap();
    let read_here: Vec<_> = read.readings.iter().map(|r| r.property).collect();
    assert_eq!(
        read_here,
        [PropertyIdentifier::PRESENT_VALUE, PropertyIdentifier::UNITS],
        "{read:?}"
    );
    assert_eq!(read.readings[0].value.as_ref().unwrap().text, "21.5 °C");
    let stopped = read.stopped.as_ref().expect("stopped");
    assert!(stopped.unreachable(), "{stopped:?}");
    assert_eq!(read.via, Some(ReadVia::Rp));
    assert_eq!(rps(client.low_level().transport()), 3);
    drop(device);
    stop(client, server).await;
}

/// The lab link whose device fails every RPM whole with an Error PDU, as
/// some do for an object they don't have.
fn fails_every_rpm(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |service, _, _| match service {
        Service::Rpm => Script::Error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        Service::Rp => Script::Forward,
    })
}

/// An RPM the device fails as a whole is read again by ReadProperty in an
/// object's read as in a bulk read, so both give the same rows, and the
/// device isn't remembered as refusing RPM.
#[tokio::test(start_paused = true)]
async fn a_whole_rpm_error_reads_the_same_in_both_reads() {
    let (client, server) = lab_on(fails_every_rpm).await;
    let device = lab_device(&client).await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let wanted = [PropertyIdentifier::PRESENT_VALUE, PropertyIdentifier::UNITS];
    let object = device.read_object(ai, Wanted::List(&wanted)).await.unwrap();
    assert_eq!(object.via, ReadVia::Rp);
    let many = device
        .read_many(wanted.map(|property| (ai, property)))
        .await
        .unwrap();
    assert_eq!(many.via, Some(ReadVia::Rp));
    assert!(many.stopped.is_none());
    assert_eq!(many.readings, object.readings);
    assert_eq!(object.readings[0].value.as_ref().unwrap().text, "21.5 °C");
    assert_eq!(rpms(client.low_level().transport()), 2, "one each");
    assert!(!client.refused_rpm(&lab_at()));
    drop(device);
    stop(client, server).await;
}

/// An array element past the end is an error in its own row; a property
/// asked for twice reads twice, and `get` gives the first.
#[tokio::test(start_paused = true)]
async fn an_element_out_of_range_and_a_property_asked_twice() {
    let (client, server) = lab_on(forwards).await;
    let device = lab_device(&client).await;
    let read = device
        .read_many(["ao:1 priority-array[17]", "ai:1 pv", "ao:1 pv", "ai:1 pv"])
        .await
        .unwrap();
    assert!(read.stopped.is_none(), "{read:?}");
    assert_eq!(
        read.readings[0].value,
        Err(remote(ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX))
    );
    assert_eq!(read.readings[0].index, Some(17));
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let first = read.get(ai, PropertyIdentifier::PRESENT_VALUE).unwrap();
    assert!(std::ptr::eq(first, &read.readings[1]), "the first of two");
    assert_eq!(read.readings[3].value, read.readings[1].value);
    let object = device
        .read_object(ai, Wanted::List(&[PropertyIdentifier::PRESENT_VALUE]))
        .await
        .unwrap();
    assert_eq!(
        object
            .get(PropertyIdentifier::PRESENT_VALUE)
            .map(|r| r.index),
        Some(None)
    );
    drop(device);
    stop(client, server).await;
}

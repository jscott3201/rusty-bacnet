//! [`Device::read_many`](crate::Device::read_many) against the lab device:
//! row errors stay in their rows, a device that stops answering ends the
//! read with what was read, and the batches split and fall back as the
//! watch hub's polls do.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendMode, StartMode, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::RemoteError;
use bytes::Bytes;

use super::tests::{lab_at, lab_device, lab_on, oid, refuses_rpm, rejections, rpm_invoke_id, stop};
use super::*;
use crate::error::OpsError;
use crate::format::PropertyError;
use crate::reading::PropertyRef;

/// What the scripted link does with a ReadPropertyMultiple, by its number
/// from 0.
#[derive(Clone, Copy)]
enum Script {
    /// Pass it to the device.
    Forward,
    /// Answer it with an Abort for this reason, as the device.
    Abort(AbortReason),
    /// Drop it: the device never answers.
    Silent,
}

/// The lab link, doing with each ReadPropertyMultiple what `script` says
/// for its number; everything else passes through. Its state counts the
/// RPMs sent.
fn scripted(inner: LoopbackTransport, script: fn(usize) -> Script) -> TestTransport {
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .start(StartMode::Relay(256));
    let inbox = builder.handle();
    let counter = Arc::new(AtomicUsize::new(0));
    builder
        .state(Arc::clone(&counter))
        .intercept(move |frame| {
            let invoke_id = (!frame.broadcast)
                .then(|| rpm_invoke_id(&frame.npdu))
                .flatten();
            let inbox = inbox.clone();
            let counter = Arc::clone(&counter);
            async move {
                let Some(invoke_id) = invoke_id else {
                    return true;
                };
                match script(counter.fetch_add(1, Ordering::Relaxed)) {
                    Script::Forward => true,
                    Script::Silent => false,
                    Script::Abort(reason) => {
                        // An Abort PDU from the server side.
                        let abort = Bytes::from(vec![0x01, 0x00, 0x71, invoke_id, reason.to_raw()]);
                        if let Some(inbox) = inbox.inbound() {
                            let source = MacAddr::from_slice(&frame.mac);
                            let back =
                                ReceivedNpdu::unverified(abort, source, false, Vec::new(), None);
                            let _ = inbox.send(back).await;
                        }
                        false
                    }
                }
            }
        })
        .build()
}

/// RPMs the scripted link has seen.
fn rpms(transport: &TestTransport) -> usize {
    transport.state::<AtomicUsize>().load(Ordering::Relaxed)
}

fn forwards(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |_| Script::Forward)
}

fn too_large_first(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |n| match n {
        0 => Script::Abort(AbortReason::SEGMENTATION_NOT_SUPPORTED),
        _ => Script::Forward,
    })
}

fn fails_first(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |n| match n {
        0 => Script::Abort(AbortReason::OTHER),
        _ => Script::Forward,
    })
}

fn silent_after_first(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |n| match n {
        0 => Script::Forward,
        _ => Script::Silent,
    })
}

/// `count` properties alternating between the analog input and the
/// analog output, so each is a batch row of its own: Present_Value and
/// Units turn about.
fn alternating(count: usize) -> Vec<PropertyRef> {
    (0..count)
        .map(|at| {
            let object = if at % 2 == 0 {
                oid(ObjectType::ANALOG_INPUT, 1)
            } else {
                oid(ObjectType::ANALOG_OUTPUT, 1)
            };
            let property = if (at / 2) % 2 == 0 {
                PropertyIdentifier::PRESENT_VALUE
            } else {
                PropertyIdentifier::UNITS
            };
            PropertyRef::new(object, property)
        })
        .collect()
}

fn remote(class: ErrorClass, code: ErrorCode) -> PropertyError {
    PropertyError::Remote(RemoteError::Error { class, code })
}

/// An unknown object and a refused property are errors in their own rows;
/// the rest read, in order, named by the Units and state texts the same
/// read holds, all in one RPM.
#[tokio::test(start_paused = true)]
async fn row_errors_stay_in_their_rows() {
    let (client, server) = lab_on(forwards).await;
    let device = lab_device(&client).await;
    let read = device
        .read_many([
            "ai:1 pv",
            "ai:99 pv",
            "ai:1 units",
            "ai:1 state-text",
            "bv:1 pv",
            "bv:1 active-text",
            "bv:1 inactive-text",
        ])
        .await;
    assert!(read.stopped.is_none(), "{read:?}");
    assert_eq!(read.via, Some(ReadVia::Rpm));
    assert_eq!(rpms(client.low_level().transport()), 1, "one batch");
    let texts: Vec<_> = read
        .readings
        .iter()
        .map(|r| r.value.clone().map(|v| v.text))
        .collect();
    assert_eq!(texts[0], Ok("21.5 °C".into()));
    assert_eq!(
        texts[1],
        Err(remote(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
    );
    assert_eq!(texts[2], Ok("degrees-celsius".into()));
    assert_eq!(
        texts[3],
        Err(remote(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
    assert_eq!(texts[4], Ok("inactive (Stopped)".into()));
    let asked: Vec<_> = read
        .readings
        .iter()
        .map(|r| (r.object, r.property))
        .collect();
    assert_eq!(
        asked[1],
        (
            oid(ObjectType::ANALOG_INPUT, 99),
            PropertyIdentifier::PRESENT_VALUE
        )
    );
    assert_eq!(
        asked[6],
        (
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::INACTIVE_TEXT
        )
    );
    assert_eq!(
        read.readings[4].value.as_ref().unwrap().as_bool(),
        Some(false)
    );
    drop(device);
    stop(client, server).await;
}

/// A device that stops answering ends the read: the first batch's
/// readings stand, and `stopped` says it is unreachable.
#[tokio::test(start_paused = true)]
async fn a_device_that_stops_answering_ends_the_read_with_what_was_read() {
    let (client, server) = lab_on(silent_after_first).await;
    let device = lab_device(&client).await;
    let read = device.read_many(alternating(20)).await;
    assert_eq!(read.readings.len(), 16, "one batch of sixteen rows read");
    assert!(read.readings.iter().all(|r| r.value.is_ok()), "{read:?}");
    let stopped = read.stopped.as_ref().expect("stopped");
    assert!(stopped.unreachable(), "{stopped:?}");
    assert!(
        matches!(stopped, OpsError::Unreachable { device, .. } if *device == lab_at()),
        "{stopped:?}"
    );
    assert_eq!(read.via, Some(ReadVia::Rpm));
    assert_eq!(rpms(client.low_level().transport()), 2);
    drop(device);
    stop(client, server).await;
}

/// A batch whose answer is too large is asked again a row at a time, still
/// by RPM.
#[tokio::test(start_paused = true)]
async fn an_answer_too_large_is_split_into_rows() {
    let (client, server) = lab_on(too_large_first).await;
    let device = lab_device(&client).await;
    let read = device.read_many(alternating(4)).await;
    assert!(read.stopped.is_none(), "{read:?}");
    assert_eq!(read.readings.len(), 4);
    assert!(read.readings.iter().all(|r| r.value.is_ok()), "{read:?}");
    assert_eq!(read.via, Some(ReadVia::Rpm));
    assert_eq!(
        rpms(client.low_level().transport()),
        1 + 4,
        "one refused, then a row each"
    );
    assert!(!client.refused_rpm(&lab_at()), "too large is no refusal");
    drop(device);
    stop(client, server).await;
}

/// A batch the device fails as a whole is read by ReadProperty, this time
/// only: the next batch goes by RPM again, so the read is mixed.
#[tokio::test(start_paused = true)]
async fn a_failed_batch_falls_back_to_read_property() {
    let (client, server) = lab_on(fails_first).await;
    let device = lab_device(&client).await;
    let read = device.read_many(alternating(18)).await;
    assert!(read.stopped.is_none(), "{read:?}");
    assert_eq!(read.readings.len(), 18);
    assert!(read.readings.iter().all(|r| r.value.is_ok()), "{read:?}");
    assert_eq!(read.via, Some(ReadVia::Mixed));
    assert_eq!(rpms(client.low_level().transport()), 2);
    assert_eq!(
        read.readings[0].value.as_ref().unwrap().text,
        "21.5 °C",
        "named by the Units read by ReadProperty in the same call"
    );
    drop(device);
    stop(client, server).await;
}

/// A device that refuses RPM is read by ReadProperty, with row errors as
/// before, and the refusal is remembered.
#[tokio::test(start_paused = true)]
async fn a_device_without_rpm_is_read_by_read_property() {
    let (client, server) = lab_on(refuses_rpm).await;
    let device = lab_device(&client).await;
    let wanted = ["ai:1 pv", "ai:1 units", "ai:1 state-text"];
    let read = device.read_many(wanted).await;
    assert!(read.stopped.is_none(), "{read:?}");
    assert_eq!(read.via, Some(ReadVia::Rp));
    assert_eq!(read.readings[0].value.as_ref().unwrap().text, "21.5 °C");
    assert_eq!(
        read.readings[2].value,
        Err(remote(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
    assert!(client.refused_rpm(&lab_at()));
    let again = device.read_many(wanted).await;
    assert_eq!(again.readings, read.readings);
    assert_eq!(
        rejections(client.low_level().transport()),
        1,
        "no RPM tried again"
    );
    drop(device);
    stop(client, server).await;
}

/// Text that names no property stops the read before anything is sent;
/// nothing to read is no read.
#[tokio::test(start_paused = true)]
async fn a_property_that_doesnt_parse_sends_nothing() {
    let (client, server) = lab_on(forwards).await;
    let device = lab_device(&client).await;
    let read = device.read_many(["ai:1 pv", "ai:1 no-such-thing"]).await;
    assert!(read.readings.is_empty());
    assert!(
        matches!(
            &read.stopped,
            Some(OpsError::Parse {
                what: "property",
                ..
            })
        ),
        "{read:?}"
    );
    assert_eq!(read.via, None);
    let none = device.read_many(Vec::<PropertyRef>::new()).await;
    assert!(none.readings.is_empty() && none.stopped.is_none());
    assert_eq!(rpms(client.low_level().transport()), 0);
    drop(device);
    stop(client, server).await;
}

/// The invoke ID of `npdu` when it is a local, unsegmented ReadProperty of
/// a Device object's Object_List, and whether it names an element.
fn object_list_read(npdu: &[u8]) -> Option<(u8, bool)> {
    match npdu {
        [
            0x01,
            control,
            apdu,
            _,
            invoke_id,
            12,
            0x0C,
            _,
            _,
            _,
            _,
            0x19,
            0x4C,
            rest @ ..,
        ] if control & 0x28 == 0 && apdu & 0xF8 == 0x00 => {
            Some((*invoke_id, rest.first() == Some(&0x29)))
        }
        _ => None,
    }
}

/// The lab link answering a whole Object_List read with an Abort, as a
/// device that can't segment its answer does, and element 0 with a count
/// past what the reader takes: 100,001.
fn claims_a_huge_list(inner: LoopbackTransport) -> TestTransport {
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .start(StartMode::Relay(256));
    let inbox = builder.handle();
    builder
        .intercept(move |frame| {
            let read = (!frame.broadcast)
                .then(|| object_list_read(&frame.npdu))
                .flatten();
            let inbox = inbox.clone();
            async move {
                let Some((invoke_id, element)) = read else {
                    return true;
                };
                let reason = AbortReason::SEGMENTATION_NOT_SUPPORTED.to_raw();
                let answer = if element {
                    // ReadProperty-ACK: Device 1001, Object_List[0], Unsigned 100001.
                    vec![
                        0x01, 0x00, 0x30, invoke_id, 0x0C, 0x0C, 0x02, 0x00, 0x03, 0xE9, 0x19,
                        0x4C, 0x29, 0x00, 0x3E, 0x24, 0x00, 0x01, 0x86, 0xA1, 0x3F,
                    ]
                } else {
                    vec![0x01, 0x00, 0x71, invoke_id, reason]
                };
                if let Some(inbox) = inbox.inbound() {
                    let source = MacAddr::from_slice(&frame.mac);
                    let back = ReceivedNpdu::unverified(
                        Bytes::from(answer),
                        source,
                        false,
                        Vec::new(),
                        None,
                    );
                    let _ = inbox.send(back).await;
                }
                false
            }
        })
        .build()
}

/// An Object_List claiming more elements than the reader takes fails as
/// the decoders refuse a count past their limit: a typed client error, no
/// longer an encoding error's text (#2560).
#[tokio::test(start_paused = true)]
async fn an_object_list_too_long_is_a_typed_overflow() {
    use bacnet_types::error::{DecodingKind, Error};
    let (client, server) = lab_on(claims_a_huge_list).await;
    let error = lab_device(&client).await.list_objects().await.unwrap_err();
    let OpsError::Client(Error::Decoding(refused)) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(refused.kind, DecodingKind::Overflow);
    assert_eq!(refused.values, [100_001, u64::from(MAX_OBJECTS)]);
    assert!(
        error
            .to_string()
            .contains("Object_List: 100001 elements claimed"),
        "{error}"
    );
    stop(client, server).await;
}

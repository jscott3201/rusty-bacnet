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
use bacnet_types::enums::{
    AbortReason, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, RejectReason,
};
use bacnet_types::error::RemoteError;
use bytes::Bytes;

use super::tests::{lab_at, lab_device, lab_on, oid, refuses_rpm, rejections, stop};
use super::*;
use crate::error::OpsError;
use crate::format::PropertyError;
use crate::reading::PropertyRef;

/// The confirmed services the scripted link scripts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Service {
    /// ReadProperty.
    Rp,
    /// ReadPropertyMultiple.
    Rpm,
}

/// What the scripted link does with a request.
pub(super) enum Script {
    /// Pass it to the device.
    Forward,
    /// Answer it with an Abort for this reason, as the device.
    Abort(AbortReason),
    /// Answer it with a Reject for this reason, as the device.
    Reject(RejectReason),
    /// Answer it with an Error PDU of this class and code, as the device.
    Error(ErrorClass, ErrorCode),
    /// Answer it with this service ACK's data, as the device.
    Ack(Vec<u8>),
    /// Drop it: the device never answers.
    Silent,
}

/// How a script decides: from the service, its number among requests of
/// that service from 0, and the request's service data.
pub(super) type Scripter = fn(Service, usize, &[u8]) -> Script;

/// The service, invoke ID and service data of `npdu` when it is a local,
/// unsegmented ReadProperty or ReadPropertyMultiple request.
pub(super) fn scripted_request(npdu: &[u8]) -> Option<(Service, u8, &[u8])> {
    match npdu {
        [0x01, control, apdu, _, invoke_id, service, data @ ..]
            if control & 0x28 == 0 && apdu & 0xF8 == 0x00 =>
        {
            let service = match service {
                12 => Service::Rp,
                14 => Service::Rpm,
                _ => return None,
            };
            Some((service, *invoke_id, data))
        }
        _ => None,
    }
}

/// The reply `script` makes for `invoke_id` and `service`, as the
/// device's NPDU; `None` for a request it forwards or drops.
fn reply(script: Script, service: Service, invoke_id: u8) -> Option<Vec<u8>> {
    let choice = match service {
        Service::Rp => 12,
        Service::Rpm => 14,
    };
    let apdu = match script {
        Script::Forward | Script::Silent => return None,
        Script::Abort(reason) => vec![0x71, invoke_id, reason.to_raw()],
        Script::Reject(reason) => vec![0x60, invoke_id, reason.to_raw()],
        Script::Error(class, code) => {
            let (class, code) = (class.to_raw(), code.to_raw());
            vec![0x50, invoke_id, choice, 0x91, class as u8, 0x91, code as u8]
        }
        Script::Ack(data) => [vec![0x30, invoke_id, choice], data].concat(),
    };
    Some([vec![0x01, 0x00], apdu].concat())
}

/// The lab link, doing with each ReadProperty and ReadPropertyMultiple
/// what `script` says; everything else passes through. Its state counts
/// the requests of each service sent: RPMs first.
pub(super) fn scripted(inner: LoopbackTransport, script: Scripter) -> TestTransport {
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .start(StartMode::Relay(256));
    let inbox = builder.handle();
    let counts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    builder
        .state(Arc::clone(&counts))
        .intercept(move |frame| {
            let decided = (!frame.broadcast)
                .then(|| scripted_request(&frame.npdu))
                .flatten()
                .map(|(service, invoke_id, data)| {
                    let slot = &counts[usize::from(service == Service::Rp)];
                    let script = script(service, slot.fetch_add(1, Ordering::Relaxed), data);
                    let forward = matches!(script, Script::Forward);
                    (forward, reply(script, service, invoke_id))
                });
            let inbox = inbox.clone();
            async move {
                let Some((forward, reply)) = decided else {
                    return true;
                };
                if let (Some(reply), Some(inbox)) = (reply, inbox.inbound()) {
                    let source = MacAddr::from_slice(&frame.mac);
                    let back = ReceivedNpdu::unverified(
                        Bytes::from(reply),
                        source,
                        false,
                        Vec::new(),
                        None,
                    );
                    let _ = inbox.send(back).await;
                }
                forward
            }
        })
        .build()
}

/// RPMs the scripted link has seen.
pub(super) fn rpms(transport: &TestTransport) -> usize {
    transport.state::<[AtomicUsize; 2]>()[0].load(Ordering::Relaxed)
}

/// ReadProperty requests the scripted link has seen.
pub(super) fn rps(transport: &TestTransport) -> usize {
    transport.state::<[AtomicUsize; 2]>()[1].load(Ordering::Relaxed)
}

pub(super) fn forwards(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |_, _, _| Script::Forward)
}

fn too_large_first(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |service, n, _| match (service, n) {
        (Service::Rpm, 0) => Script::Abort(AbortReason::SEGMENTATION_NOT_SUPPORTED),
        _ => Script::Forward,
    })
}

fn fails_first(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |service, n, _| match (service, n) {
        (Service::Rpm, 0) => Script::Abort(AbortReason::OTHER),
        _ => Script::Forward,
    })
}

fn silent_after_first(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |service, n, _| match (service, n) {
        (Service::Rpm, 0) => Script::Forward,
        (Service::Rpm, _) => Script::Silent,
        _ => Script::Forward,
    })
}

/// `count` properties alternating between the analog input and the
/// analog output, so each is a batch row of its own: Present_Value and
/// Units turn about.
pub(super) fn alternating(count: usize) -> Vec<PropertyRef> {
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

pub(super) fn remote(class: ErrorClass, code: ErrorCode) -> PropertyError {
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
        .await
        .unwrap();
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
    let read = device.read_many(alternating(20)).await.unwrap();
    assert_eq!(read.readings.len(), 16, "one batch of sixteen rows read");
    assert!(read.readings.iter().all(|r| r.value.is_ok()), "{read:?}");
    let stopped = read.stopped.as_ref().expect("stopped");
    assert!(stopped.unreachable(), "{stopped:?}");
    assert!(
        matches!(stopped, OpsError::Unreachable { device, .. } if device == &lab_at()),
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
    let read = device.read_many(alternating(4)).await.unwrap();
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
    let read = device.read_many(alternating(18)).await.unwrap();
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
    let read = device.read_many(wanted).await.unwrap();
    assert!(read.stopped.is_none(), "{read:?}");
    assert_eq!(read.via, Some(ReadVia::Rp));
    assert_eq!(read.readings[0].value.as_ref().unwrap().text, "21.5 °C");
    assert_eq!(
        read.readings[2].value,
        Err(remote(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
    assert!(client.refused_rpm(&lab_at()));
    let again = device.read_many(wanted).await.unwrap();
    assert_eq!(again.readings, read.readings);
    assert_eq!(
        rejections(client.low_level().transport()),
        1,
        "no RPM tried again"
    );
    drop(device);
    stop(client, server).await;
}

/// Text that names no property, or one that names many (ALL, REQUIRED,
/// OPTIONAL), fails the read before anything is sent, pointing to
/// `read_object`; nothing to read is no read.
#[tokio::test(start_paused = true)]
async fn a_property_that_doesnt_parse_sends_nothing() {
    let (client, server) = lab_on(forwards).await;
    let device = lab_device(&client).await;
    let error = device
        .read_many(["ai:1 pv", "ai:1 no-such-thing"])
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            OpsError::Parse {
                what: "property",
                ..
            }
        ),
        "{error:?}"
    );
    for many in ["ai:1 all", "ai:1 required", "ai:1 optional"] {
        let error = device.read_many(["ai:1 pv", many]).await.unwrap_err();
        assert!(
            matches!(&error, OpsError::Parse { what: "property", reason, .. } if reason.contains("read_object")),
            "{many}: {error:?}"
        );
        let error = device.read(many).await.unwrap_err();
        assert!(
            matches!(&error, OpsError::Parse { .. }),
            "{many}: {error:?}"
        );
    }
    let none = device.read_many(Vec::<PropertyRef>::new()).await.unwrap();
    assert!(none.readings.is_empty() && none.stopped.is_none() && none.via.is_none());
    assert_eq!(rpms(client.low_level().transport()), 0);
    assert_eq!(rps(client.low_level().transport()), 0);
    drop(device);
    stop(client, server).await;
}

/// The lab link answering a whole Object_List read with an Abort, as a
/// device that can't segment its answer does, and element 0 with a count
/// past what the reader takes: 100,001.
fn claims_a_huge_list(inner: LoopbackTransport) -> TestTransport {
    scripted(inner, |service, _, data| match (service, data) {
        // Device 1001's Object_List (76), whole or by element.
        (Service::Rp, [0x0C, 0x02, 0x00, 0x03, 0xE9, 0x19, 0x4C]) => {
            Script::Abort(AbortReason::SEGMENTATION_NOT_SUPPORTED)
        }
        (Service::Rp, [0x0C, 0x02, 0x00, 0x03, 0xE9, 0x19, 0x4C, 0x29, 0x00]) => {
            // Object_List[0]: Unsigned 100001.
            Script::Ack(vec![
                0x0C, 0x02, 0x00, 0x03, 0xE9, 0x19, 0x4C, 0x29, 0x00, 0x3E, 0x24, 0x00, 0x01, 0x86,
                0xA1, 0x3F,
            ])
        }
        _ => Script::Forward,
    })
}

/// An Object_List claiming more elements than the reader takes fails with
/// a typed client error holding both numbers, no longer an encoding
/// error's text, and no decoder's offset either (#2560).
#[tokio::test(start_paused = true)]
async fn an_object_list_too_long_is_a_typed_limit() {
    use bacnet_types::error::Error;
    let (client, server) = lab_on(claims_a_huge_list).await;
    let error = lab_device(&client).await.list_objects().await.unwrap_err();
    let OpsError::Client(Error::Range(refused)) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(refused.values, [100_001, u64::from(MAX_OBJECTS)]);
    assert_eq!(
        error.to_string(),
        "value out of range: Object_List claims 100001 objects; at most 100000 are read"
    );
    stop(client, server).await;
}

//! The client against an in-process lab device over a loopback link.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_client::log_reader::LogCursor;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::read_range::LogRecords;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendMode, StartMode, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetLogRecord, LogDatum};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::Bytes;

use super::*;
use crate::device::DeviceAddress;
use crate::format::{Floats, PropertyError, StateTexts, Value, ValueFormat, decode_all};

/// The lab device's instance.
pub(super) const LAB: u32 = 1001;

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A trend log holding `records` REAL samples, one a minute from midnight.
fn trend(records: u8) -> TrendLogObject {
    let mut log = TrendLogObject::new(1, "Zone Temp Log", 100).unwrap();
    for minute in 0..records {
        log.add_record(BACnetLogRecord {
            date: Date {
                year: 126,
                month: 10,
                day: 25,
                day_of_week: 7,
            },
            time: Time {
                hour: 0,
                minute,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::RealValue(f32::from(minute)),
            status_flags: None,
        })
        .unwrap();
    }
    log
}

/// Trend log 2, sampling the analog input's Event_State (#2091).
fn event_state_trend() -> TrendLogObject {
    let mut log = TrendLogObject::new(2, "Zone Temp State Log", 10).unwrap();
    log.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::ANALOG_INPUT, 1),
        PropertyIdentifier::EVENT_STATE.to_raw(),
    )))
    .unwrap();
    log
}

/// Binary Value `instance`, with its Active_Text and Inactive_Text (#2122).
fn binary_value(instance: u32, active: &str, inactive: &str) -> BinaryValueObject {
    let mut value = BinaryValueObject::new(instance, format!("BV-{instance}")).unwrap();
    for (property, text) in [
        (PropertyIdentifier::ACTIVE_TEXT, active),
        (PropertyIdentifier::INACTIVE_TEXT, inactive),
    ] {
        value
            .write_property(
                property,
                None,
                PropertyValue::CharacterString(text.into()),
                None,
            )
            .unwrap();
    }
    value
}

/// The lab: a device with an analog input, an analog output, two binary
/// values and two trend logs, a client over a low-level client paired with
/// it, and the server to stop.
async fn lab() -> (Client<LoopbackTransport>, BACnetServer<LoopbackTransport>) {
    lab_on(|port| port).await
}

/// [`lab`], with the client's link wrapped by `wrap`.
pub(super) async fn lab_on<C: TransportPort + 'static>(
    wrap: fn(LoopbackTransport) -> C,
) -> (Client<C>, BACnetServer<LoopbackTransport>) {
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let mut ai = AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(21.5);
    let ao = AnalogOutputObject::new(1, "Damper", EngineeringUnits::PERCENT).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(ai),
        Box::new(ao),
        Box::new(trend(3)),
        Box::new(event_state_trend()),
        Box::new(NotificationClass::new(0, "Alarms").unwrap()),
        Box::new(binary_value(1, "Running", "Stopped")),
        Box::new(binary_value(2, "Open", "Shut")),
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
    let low = BACnetClient::start(config, wrap(client_port))
        .await
        .unwrap();
    low.add_device(LAB, &[2]).await.unwrap();
    let client = Client::new(Arc::new(low), Arc::new(RequestLimits::new(0)));
    (client, server)
}

/// The lab device's handle, following its instance.
pub(super) async fn lab_device<C: TransportPort + 'static>(client: &Client<C>) -> crate::Device<C> {
    client.device(LAB).await.unwrap()
}

/// Where the lab device answers.
pub(super) fn lab_at() -> DeviceAddress {
    DeviceAddress::local(&[2]).unwrap()
}

pub(super) async fn stop<C: TransportPort + 'static>(
    client: Client<C>,
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

/// An object's read gives each property's value by RPM, named with the
/// Units the same read holds, and its octets as sent.
#[tokio::test(start_paused = true)]
async fn reads_properties_by_rpm_and_formats_them() {
    let (client, server) = lab().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let read = lab_device(&client)
        .await
        .read_object(
            ai,
            Wanted::List(&[PropertyIdentifier::PRESENT_VALUE, PropertyIdentifier::UNITS]),
        )
        .await
        .unwrap();
    assert_eq!(read.via, ReadVia::Rpm);
    let reading = &read.readings[0];
    assert_eq!(
        (reading.object, reading.property, reading.index),
        (ai, PropertyIdentifier::PRESENT_VALUE, None)
    );
    let value = reading.value.as_ref().unwrap();
    assert_eq!(value.text, "21.5 °C");
    assert_eq!(value.as_f64(), Some(21.5));
    assert_eq!(decode_all(&value.raw).unwrap(), [PropertyValue::Real(21.5)]);
    // The same octets in another front end's style.
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::ANALOG_INPUT, &texts, Floats::Display);
    assert_eq!(
        format.value(PropertyIdentifier::PRESENT_VALUE, &value.raw),
        "21.5"
    );
    // One element of an array, by ReadProperty, and the same from text.
    let device = lab_device(&client).await;
    let length = device
        .read((
            oid(ObjectType::ANALOG_OUTPUT, 1),
            PropertyIdentifier::PRIORITY_ARRAY,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(length.as_u64(), Some(16));
    assert_eq!(device.read("ao:1 pa[0]").await.unwrap(), length);
    assert_eq!(device.read("ai:1").await.unwrap().as_f64(), Some(21.5));
    drop(device);
    assert!(!client.refused_rpm(&lab_at()));
    stop(client, server).await;
}

/// The object list and the device summary come from the Device object.
#[tokio::test(start_paused = true)]
async fn lists_objects_and_describes_the_device() {
    let (client, server) = lab().await;
    let list = lab_device(&client).await.list_objects().await.unwrap();
    assert!(!list.indexed);
    assert_eq!(list.stopped, None);
    for object in [
        oid(ObjectType::DEVICE, LAB),
        oid(ObjectType::ANALOG_INPUT, 1),
        oid(ObjectType::ANALOG_OUTPUT, 1),
        oid(ObjectType::TREND_LOG, 1),
    ] {
        assert!(list.objects.contains(&object), "{object:?} in {list:?}");
    }
    let summary = lab_device(&client).await.describe_device().await.unwrap();
    assert_eq!(summary.device, LAB);
    assert_eq!(summary.readings.len(), DEVICE_PROPERTIES.len());
    let properties: Vec<_> = summary.readings.iter().map(|r| r.property).collect();
    assert_eq!(properties, DEVICE_PROPERTIES);
    assert!(summary.name().is_some(), "{summary:?}");
    assert_eq!(
        summary
            .value(PropertyIdentifier::VENDOR_IDENTIFIER)
            .and_then(Value::as_u64),
        Some(42)
    );
    stop(client, server).await;
}

/// A trend log reads page by page from the oldest record, and alarms
/// decode (the lab has none).
#[tokio::test(start_paused = true)]
async fn reads_trend_pages_and_alarms() {
    let (client, server) = lab().await;
    let log = oid(ObjectType::TREND_LOG, 1);
    let first = lab_device(&client)
        .await
        .read_trend(log, LogCursor::Oldest, 2)
        .await
        .unwrap();
    let LogRecords::TrendLog(records) = &first.records else {
        panic!("trend records: {first:?}");
    };
    assert_eq!(records.len(), 2);
    assert!(!first.done);
    let second = lab_device(&client)
        .await
        .read_trend(log, first.next, 2)
        .await
        .unwrap();
    let LogRecords::TrendLog(records) = &second.records else {
        panic!("trend records: {second:?}");
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].log_datum, LogDatum::RealValue(2.0));
    assert!(second.done);

    let alarms = lab_device(&client).await.alarms(None).await.unwrap();
    assert!(alarms.list_of_event_summaries.is_empty());
    assert!(!alarms.more_events);
    stop(client, server).await;
}

/// A device that isn't in the discovery table fails the read.
#[tokio::test(start_paused = true)]
async fn an_unknown_device_fails() {
    let (client, server) = lab().await;
    let error = client.device(4242).await.unwrap_err();
    assert!(error.to_string().contains("4242"), "{error}");
    stop(client, server).await;
}

/// A device that rejects ReadPropertyMultiple is read by ReadProperty
/// instead, and the client remembers it: the next read of that device goes
/// straight to ReadProperty, with the same values. Errors stay structured
/// on both paths.
#[tokio::test(start_paused = true)]
async fn a_refusal_of_rpm_is_remembered_per_device() {
    let (client, server) = lab_on(refuses_rpm).await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    // State_Text: an analog input has none.
    let wanted = [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::STATE_TEXT,
    ];
    assert!(!client.refused_rpm(&lab_at()));
    let first = lab_device(&client)
        .await
        .read_object(ai, Wanted::List(&wanted))
        .await
        .unwrap();
    assert_eq!(first.via, ReadVia::Rp);
    assert!(client.refused_rpm(&lab_at()), "the refusal is remembered");
    assert!(!client.refused_rpm(&DeviceAddress::local(&[3]).unwrap()));
    let rejected = rejections(client.low_level().transport());
    assert_eq!(rejected, 1);
    let second = lab_device(&client)
        .await
        .read_object(ai, Wanted::List(&wanted))
        .await
        .unwrap();
    assert_eq!(second.via, ReadVia::Rp);
    assert_eq!(
        rejections(client.low_level().transport()),
        rejected,
        "no RPM was tried again"
    );
    assert_eq!(second.readings, first.readings);
    assert_eq!(
        second.readings[0].value.as_ref().unwrap().as_f64(),
        Some(21.5)
    );
    let missing = second.readings[1].value.as_ref().unwrap_err();
    assert_eq!(
        missing.class_code(),
        Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
        "{missing:?}"
    );
    stop(client, server).await;
}

/// An RPM answer's per-property error comes back as the device's class and
/// code, shown as `CLASS:CODE`.
#[tokio::test(start_paused = true)]
async fn a_missing_property_is_a_typed_error_row() {
    let (client, server) = lab().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let wanted = [PropertyIdentifier::STATE_TEXT];
    let read = lab_device(&client)
        .await
        .read_object(ai, Wanted::List(&wanted))
        .await
        .unwrap();
    assert_eq!(read.via, ReadVia::Rpm);
    let error = read.readings[0].value.as_ref().unwrap_err();
    assert_eq!(
        *error,
        PropertyError::Remote(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::UNKNOWN_PROPERTY,
        })
    );
    assert_eq!(error.to_string(), "PROPERTY:UNKNOWN_PROPERTY");
    assert_eq!(PropertyError::NoValue.to_string(), "no value");
    stop(client, server).await;
}

/// The invoke ID of `npdu` when it is a local, unsegmented
/// ReadPropertyMultiple request: NPDU version and a control octet with no
/// addresses, then a Confirmed-Request APDU.
pub(super) fn rpm_invoke_id(npdu: &[u8]) -> Option<u8> {
    match npdu {
        [0x01, control, apdu, _, invoke_id, service, ..]
            if control & 0x28 == 0 && apdu & 0xF8 == 0x00 && *service == 14 =>
        {
            Some(*invoke_id)
        }
        _ => None,
    }
}

/// The client's side of the lab link, answering every ReadPropertyMultiple
/// itself with a Reject (unrecognized service), as a device without RPM
/// does. Everything else passes through. The link's state is an
/// [`AtomicUsize`] counting those Rejects.
pub(super) fn refuses_rpm(inner: LoopbackTransport) -> TestTransport {
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
                counter.fetch_add(1, Ordering::Relaxed);
                // A Reject PDU for UNRECOGNIZED_SERVICE (9), from the device.
                let reject = Bytes::from(vec![0x01, 0x00, 0x60, invoke_id, 9]);
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

/// ReadPropertyMultiple requests the [`refuses_rpm`] link has answered with
/// a Reject so far.
pub(super) fn rejections(transport: &TestTransport) -> usize {
    transport.state::<AtomicUsize>().load(Ordering::Relaxed)
}

/// Every call waits for the device's gate: while another request to the
/// device holds it, nothing goes, and once it is free the call completes.
#[tokio::test(start_paused = true)]
async fn every_call_waits_for_the_device_gate() {
    let (client, server) = lab().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let log = oid(ObjectType::TREND_LOG, 1);
    let wait = std::time::Duration::from_secs(30);
    let held = client
        .limits()
        .gate(&lab_at())
        .acquire_owned()
        .await
        .unwrap();
    let pv = [PropertyIdentifier::PRESENT_VALUE];
    assert!(
        tokio::time::timeout(
            wait,
            lab_device(&client).await.read_object(ai, Wanted::List(&pv))
        )
        .await
        .is_err()
    );
    assert!(
        tokio::time::timeout(wait, lab_device(&client).await.list_objects())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(
            wait,
            lab_device(&client)
                .await
                .read_trend(log, LogCursor::Oldest, 2)
        )
        .await
        .is_err()
    );
    assert!(
        tokio::time::timeout(wait, lab_device(&client).await.alarms(None))
            .await
            .is_err()
    );
    drop(held);
    tokio::time::timeout(wait, lab_device(&client).await.alarms(None))
        .await
        .expect("the gate is free")
        .unwrap();
    stop(client, server).await;
}

/// A trend page is charged for every request it sends, not once per page:
/// the oldest page reads the log's counts before its ReadRange, and the
/// next page, by sequence number, sends the ReadRange alone.
#[tokio::test(start_paused = true)]
async fn a_trend_page_is_charged_per_request() {
    use crate::limits::{Account, charged};
    let (client, server) = lab().await;
    let log = oid(ObjectType::TREND_LOG, 1);
    let first = Arc::new(Account::new(None));
    let page = charged(
        Arc::clone(&first),
        lab_device(&client)
            .await
            .read_trend(log, LogCursor::Oldest, 2),
    )
    .await
    .unwrap();
    assert!(
        first.requests() >= 3,
        "counts then range: {}",
        first.requests()
    );
    let second = Arc::new(Account::new(None));
    charged(
        Arc::clone(&second),
        lab_device(&client).await.read_trend(log, page.next, 2),
    )
    .await
    .unwrap();
    assert_eq!(second.requests(), 1);
    stop(client, server).await;
}

/// A log's columns come from its Log_DeviceObjectProperty in one charged
/// request; an empty reference, a refused read and a log without the
/// property name nothing, and the last sends nothing (#2091).
#[tokio::test(start_paused = true)]
async fn log_columns_read_the_reference_once_and_fall_back() {
    use crate::format::LoggedProperty;
    use crate::limits::{Account, charged};
    let (client, server) = lab().await;
    let account = Arc::new(Account::new(None));
    let (columns, naming) = charged(
        Arc::clone(&account),
        lab_device(&client)
            .await
            .log_columns(oid(ObjectType::TREND_LOG, 2)),
    )
    .await;
    assert_eq!(account.requests(), 1);
    assert_eq!(naming, super::Naming::Answered);
    assert_eq!(
        columns.column(0),
        Some(LoggedProperty {
            object: oid(ObjectType::ANALOG_INPUT, 1),
            property: PropertyIdentifier::EVENT_STATE,
            index: None,
            device: None,
        })
    );
    let texts = StateTexts::default();
    let format = ValueFormat::new(ObjectType::TREND_LOG, &texts, Floats::Exact);
    assert_eq!(
        format.logged(columns.column(0), &PropertyValue::Enumerated(3)),
        "high-limit"
    );

    // Trend log 1 has no reference; trend log 9 doesn't exist.
    for log in [1, 9] {
        let (columns, naming) = lab_device(&client)
            .await
            .log_columns(oid(ObjectType::TREND_LOG, log))
            .await;
        assert!(columns.is_empty(), "trend log {log}: {columns:?}");
        // A refusal is an answer, not a timeout (#2131).
        assert!(!naming.timed_out(), "trend log {log}");
    }
    let account = Arc::new(Account::new(None));
    let (columns, _) = charged(
        Arc::clone(&account),
        lab_device(&client)
            .await
            .log_columns(oid(ObjectType::EVENT_LOG, 1)),
    )
    .await;
    assert!(columns.is_empty());
    assert_eq!(account.requests(), 0, "an Event Log has no reference");
    stop(client, server).await;
}

/// A local Who-Is puts the lab device in the table; a device instance is
/// no station address for a directed one.
#[tokio::test(start_paused = true)]
async fn a_who_is_fills_the_table() {
    let (client, server) = lab().await;
    client.low_level().clear_devices().await;
    assert!(client.low_level().get_device(LAB).await.unwrap().is_none());
    client.who_is(&Scope::Local, None).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let found = client
        .low_level()
        .get_device(LAB)
        .await
        .unwrap()
        .expect("the lab device");
    assert_eq!(found.vendor_id, 42);
    assert!(
        client
            .devices()
            .await
            .iter()
            .any(|d| d.object_identifier.instance_number() == LAB)
    );
    // Once an encoding error (#2560): text that names no station.
    let refused = client.station_mac("1234").await.unwrap_err();
    assert!(
        matches!(&refused, crate::OpsError::Parse { what: "address", input, .. } if input == "1234"),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("station address"), "{refused}");
    let refused = client.station_mac("2:1234").await.unwrap_err();
    assert!(
        matches!(
            &refused,
            crate::OpsError::Parse {
                what: "address",
                ..
            }
        ),
        "{refused:?}"
    );
    stop(client, server).await;
}

/// The client's side of the lab link, refusing every broadcast, as a
/// transport refuses a send it can't make.
fn refuses_broadcasts(inner: LoopbackTransport) -> TestTransport {
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Fail("broadcast refused"))
        .build()
}

/// A Who-Is the transport refuses never left, so the caller isn't charged
/// for it.
#[tokio::test(start_paused = true)]
async fn a_refused_who_is_is_not_charged() {
    use crate::limits::{Account, charged};
    let (client, server) = lab_on(refuses_broadcasts).await;
    let account = Arc::new(Account::new(None));
    let sent = charged(Arc::clone(&account), client.who_is(&Scope::Local, None)).await;
    assert!(sent.is_err());
    assert_eq!(account.requests(), 0);
    stop(client, server).await;
}

/// A Who-Has the transport refuses never left either, so a search that
/// fails on it isn't charged (#2596).
#[tokio::test(start_paused = true)]
async fn a_refused_who_has_is_not_charged() {
    use crate::limits::{Account, charged};
    let (client, server) = lab_on(refuses_broadcasts).await;
    let account = Arc::new(Account::new(None));
    let found = charged(Arc::clone(&account), client.find(super::Find::name("A"))).await;
    assert!(found.is_err(), "{found:?}");
    assert_eq!(account.requests(), 0);
    stop(client, server).await;
}

/// A timeout is the client's own or the state machine's TSM_TIMEOUT abort;
/// a transport failure is unreachable but no timeout, and a refusal is
/// neither (#2131).
#[test]
fn timeouts_are_a_kind_of_unreachable() {
    use bacnet_types::error::Error;
    let cases = [
        (
            Error::Timeout(std::time::Duration::from_secs(1)),
            true,
            true,
        ),
        (
            Error::Remote(RemoteError::Abort {
                reason: bacnet_types::enums::AbortReason::TSM_TIMEOUT,
            }),
            true,
            true,
        ),
        (
            Error::Transport(std::io::Error::other("link down")),
            false,
            true,
        ),
        (
            Error::Remote(RemoteError::Abort {
                reason: bacnet_types::enums::AbortReason::OTHER,
            }),
            false,
            false,
        ),
    ];
    for (error, timeout, gone) in cases {
        assert_eq!(super::timed_out(&error), timeout, "{error}");
        assert_eq!(super::unreachable(&error), gone, "{error}");
    }
}

/// DeviceCommunicationControl on the client's node stops a request at the
/// client: neither a timeout nor the device's answer (#2617). Reads end on
/// it, a watch asks for its subscription again, and a batch doesn't fall
/// back to one property at a time.
#[test]
fn a_disabled_node_fails_locally() {
    use bacnet_types::error::Error;
    for sent in [false, true] {
        let error = Error::InitiationDisabled { sent };
        assert!(super::failed_locally(&error), "{error}");
        assert!(!super::unreachable(&error), "{error}");
        assert!(!super::timed_out(&error), "{error}");
    }
}

//! The one-shot CLI's log column and state text reads, by [`Destination`],
//! against an in-process lab device over a loopback link (#2122): a refused
//! RPM is tried once a read, and a reference that names a device is read
//! only when the discovery table ties the MAC to it.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_client::client::ClientConfig;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_ops::format::LoggedProperty;
use bacnet_server::server::BACnetServer;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendMode, StartMode, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::PropertyValue;
use bytes::Bytes;

use super::*;

/// The lab device's instance.
const LAB: u32 = 1001;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Binary Value `instance`, with its Active_Text and Inactive_Text.
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

/// The lab: a device with an analog input, two binary values and a trend
/// log, a client paired with it that knows it, and the server to stop.
async fn lab() -> (
    BACnetClient<LoopbackTransport>,
    BACnetServer<LoopbackTransport>,
) {
    lab_on(|port| port).await
}

/// [`lab`], with the client's link wrapped by `wrap`.
async fn lab_on<C: TransportPort + 'static>(
    wrap: fn(LoopbackTransport) -> C,
) -> (BACnetClient<C>, BACnetServer<LoopbackTransport>) {
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let ai = AnalogInputObject::new(1, "Zone Temp", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(ai),
        Box::new(event_state_trend()),
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
    let client = BACnetClient::start(config, wrap(client_port))
        .await
        .unwrap();
    client.add_device(LAB, &[2]).await.unwrap();
    (client, server)
}

async fn stop<C: TransportPort + 'static>(
    mut client: BACnetClient<C>,
    mut server: BACnetServer<LoopbackTransport>,
) {
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// The invoke ID of `npdu` when it is a local, unsegmented
/// ReadPropertyMultiple request: NPDU version and a control octet with no
/// addresses, then a Confirmed-Request APDU.
fn rpm_invoke_id(npdu: &[u8]) -> Option<u8> {
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
fn refuses_rpm(inner: LoopbackTransport) -> TestTransport {
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
fn rejections(transport: &TestTransport) -> usize {
    transport.state::<AtomicUsize>().load(Ordering::Relaxed)
}

/// Present_Value of Binary Value `instance`, on `device` if named.
fn binary(instance: u32, device: Option<u32>) -> Option<LoggedProperty> {
    Some(LoggedProperty {
        object: oid(ObjectType::BINARY_VALUE, instance),
        property: PropertyIdentifier::PRESENT_VALUE,
        index: None,
        device,
    })
}

fn active(columns: &LogColumns, column: usize) -> Option<&str> {
    columns.state_texts(column).active.as_deref()
}

/// A log's columns come from its Log_DeviceObjectProperty, as the engine
/// reads them (#2091): the referenced property names the column, and a
/// refused read names nothing.
#[tokio::test(start_paused = true)]
async fn log_columns_name_the_referenced_property() {
    let (client, server) = lab().await;
    let to = Destination::Device(LAB);
    let named = to.log_columns(&client, oid(ObjectType::TREND_LOG, 2)).await;
    assert_eq!(
        named.column(0).map(|column| column.property),
        Some(PropertyIdentifier::EVENT_STATE)
    );
    let refused = to.log_columns(&client, oid(ObjectType::TREND_LOG, 9)).await;
    assert!(refused.is_empty(), "{refused:?}");
    stop(client, server).await;
}

/// A device that refuses ReadPropertyMultiple gets one RPM a read: the
/// first object falls back to ReadProperty, and the second goes straight
/// there. Both are named.
#[tokio::test(start_paused = true)]
async fn a_refused_rpm_is_tried_once_a_read() {
    let (client, server) = lab_on(refuses_rpm).await;
    let mut columns: LogColumns = [binary(1, None), binary(2, None)].into_iter().collect();
    Destination::Device(LAB)
        .log_state_texts(&client, &mut columns)
        .await;
    assert_eq!(rejections(client.transport()), 1);
    assert_eq!(active(&columns, 0), Some("Running"));
    assert_eq!(active(&columns, 1), Some("Open"));
    assert_eq!(columns.state_texts(1).inactive.as_deref(), Some("Shut"));
    stop(client, server).await;
}

/// By MAC, a reference that names the lab device is read once the
/// discovery table holds the lab at that MAC, and one that names another
/// device isn't. A second device at the same MAC makes the match
/// ambiguous, so only unqualified references are read.
#[tokio::test(start_paused = true)]
async fn a_device_qualified_reference_needs_one_device_at_the_mac() {
    let (client, server) = lab().await;
    let to = Destination::Mac(vec![2]);
    let wanted = [
        binary(1, Some(LAB)),
        binary(2, Some(LAB + 1)),
        binary(2, None),
    ];
    let mut columns: LogColumns = wanted.into_iter().collect();
    to.log_state_texts(&client, &mut columns).await;
    assert_eq!(active(&columns, 0), Some("Running"), "the lab, by MAC");
    assert_eq!(active(&columns, 1), None, "another device");
    assert_eq!(active(&columns, 2), Some("Open"));

    client.add_device(LAB + 5, &[2]).await.unwrap();
    let mut columns: LogColumns = wanted.into_iter().collect();
    to.log_state_texts(&client, &mut columns).await;
    assert_eq!(active(&columns, 0), None, "two devices at the MAC");
    assert_eq!(active(&columns, 2), Some("Open"));
    stop(client, server).await;
}

/// One object's texts for a one-shot read (#2132): a binary value's pair,
/// even from a device that refuses RPM, which two objects sharing the
/// caller's memory of the refusal learn once; nothing for a missing
/// object or a type without texts.
#[tokio::test(start_paused = true)]
async fn one_objects_state_texts_fall_back_to_none() {
    let (client, server) = lab_on(refuses_rpm).await;
    let to = Destination::Device(LAB);
    let no_rpm = Cell::new(false);
    let texts = to
        .state_texts(&client, oid(ObjectType::BINARY_VALUE, 2), &no_rpm)
        .await
        .unwrap();
    assert_eq!(texts.active.as_deref(), Some("Open"));
    assert_eq!(texts.inactive.as_deref(), Some("Shut"));
    assert_eq!(rejections(client.transport()), 1);
    let texts = to
        .state_texts(&client, oid(ObjectType::BINARY_VALUE, 1), &no_rpm)
        .await
        .unwrap();
    assert_eq!(texts.active.as_deref(), Some("Running"));
    assert_eq!(rejections(client.transport()), 1, "one RPM in all");
    for object in [
        oid(ObjectType::BINARY_VALUE, 9),
        oid(ObjectType::MULTI_STATE_VALUE, 9),
        oid(ObjectType::ANALOG_INPUT, 1),
    ] {
        let texts = to.state_texts(&client, object, &no_rpm).await;
        assert_eq!(texts.ok(), Some(StateTexts::default()), "{object:?}");
    }
    stop(client, server).await;
}

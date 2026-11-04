//! Configuration objects created over the wire on a running server start
//! working without a restart (#2208): a created Schedule writes its value as
//! soon as it exists and at each minute's pass after, follows a created
//! Calendar's changes and its deletion, and a created Trend Log logs. The
//! clock is paused; the Device clock reads Monday 5 October 2026, 09:00.

use super::create_config_types_wire_tests::{OCTOBER_5, reference};
use super::mutation_tests::{SOURCE, oid, route, value};
use super::*;
use crate::schedule::tests::SettableClock;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::constructed::encode_special_event;
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::{
    CreateObjectAck, CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier,
};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::constructed::{BACnetSpecialEvent, BACnetTimeValue, SpecialEventPeriod};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::Time;

use PropertyIdentifier as P;

/// The Schedules' and Trend Logs' target.
fn av1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 1)
}

async fn running() -> BACnetServer<TestTransport> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "AV-1", EngineeringUnits::NO_UNITS).unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::start(ServerConfig::default(), db, TestTransport::new())
        .await
        .unwrap();
    server
        .db
        .write()
        .await
        .set_clock_reader(Some(SettableClock::at(2026, 10, 5, 9, 0)));
    server
}

/// Dispatch one confirmed request to `server`, as the transport would, and
/// return the reply APDU's bytes.
async fn dispatch(
    server: &BACnetServer<TestTransport>,
    service: ConfirmedServiceChoice,
    bytes: Bytes,
) -> Bytes {
    let (tx, rx) = oneshot::channel();
    BACnetServer::dispatch(
        &server.test_dispatch_context(),
        SOURCE,
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 5,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: bytes,
        }),
        ReceivedApdu {
            router_response: None,
            direct_response: None,
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: route(),
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: vec![],
            provenance: TransportProvenance::unverified(),
            reply_tx: Some(tx),
        },
    )
    .await;
    let npdu = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("handler completed")
        .expect("a reply");
    bacnet_encoding::npdu::decode_npdu(npdu).unwrap().payload
}

async fn create(
    server: &BACnetServer<TestTransport>,
    kind: ObjectType,
    values: Vec<(PropertyIdentifier, Vec<u8>)>,
) -> ObjectIdentifier {
    let mut bytes = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Type(kind),
        list_of_initial_values: values
            .into_iter()
            .map(|(property_identifier, value)| BACnetPropertyValue {
                property_identifier,
                property_array_index: None,
                value,
                priority: None,
            })
            .collect(),
    }
    .encode(&mut bytes)
    .unwrap();
    let create = ConfirmedServiceChoice::CREATE_OBJECT;
    let reply = dispatch(server, create, bytes.freeze()).await;
    assert_eq!(&reply[..3], &[0x30, 5, create.to_raw()], "{reply:?}");
    CreateObjectAck::decode(&reply[3..])
        .unwrap()
        .object_identifier
}

async fn write(
    server: &BACnetServer<TestTransport>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
) {
    let mut bytes = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value: value,
        priority: None,
    }
    .encode(&mut bytes)
    .unwrap();
    let service = ConfirmedServiceChoice::WRITE_PROPERTY;
    let reply = dispatch(server, service, bytes.freeze()).await;
    assert_eq!(&reply[..], &[0x20, 5, service.to_raw()]);
}

async fn delete(server: &BACnetServer<TestTransport>, object: ObjectIdentifier) {
    let mut bytes = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: object,
    }
    .encode(&mut bytes);
    let service = ConfirmedServiceChoice::DELETE_OBJECT;
    let reply = dispatch(server, service, bytes.freeze()).await;
    assert_eq!(&reply[..], &[0x20, 5, service.to_raw()]);
}

async fn read(
    server: &BACnetServer<TestTransport>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    server
        .db
        .read()
        .await
        .get(&object)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

/// AV-1's priority-12 slot, where the Schedules write.
async fn slot(server: &BACnetServer<TestTransport>) -> PropertyValue {
    read(server, av1(), P::PRIORITY_ARRAY, Some(12)).await
}

/// One minute's Schedule pass.
async fn next_pass() {
    tokio::time::sleep(Duration::from_secs(61)).await;
}

/// A Schedule writing `default` to AV-1 at priority 12.
fn schedule_values(default: f32) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    vec![
        (P::SCHEDULE_DEFAULT, value(PropertyValue::Real(default))),
        (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, reference(av1())),
        (P::PRIORITY_FOR_WRITING, value(PropertyValue::Unsigned(12))),
    ]
}

#[tokio::test(start_paused = true)]
async fn a_created_schedule_writes_at_once_and_at_each_pass() {
    let mut server = running().await;
    let schedule = create(&server, ObjectType::SCHEDULE, schedule_values(5.0)).await;
    // Written with the creation, not at the next minute's pass.
    assert_eq!(slot(&server).await, PropertyValue::Real(5.0));
    // The minute's pass evaluates it too: a change the application makes
    // reaches AV-1 there.
    server
        .db
        .write()
        .await
        .get_mut(&schedule)
        .unwrap()
        .write_property(P::SCHEDULE_DEFAULT, None, PropertyValue::Real(7.0), None)
        .unwrap();
    assert_eq!(slot(&server).await, PropertyValue::Real(5.0));
    next_pass().await;
    assert_eq!(slot(&server).await, PropertyValue::Real(7.0));

    // One created bare writes nowhere until a client configures it.
    let bare = create(&server, ObjectType::SCHEDULE, vec![]).await;
    assert_eq!(
        read(&server, bare, P::PRESENT_VALUE, None).await,
        PropertyValue::Null
    );
    next_pass().await;
    assert_eq!(slot(&server).await, PropertyValue::Real(7.0));
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_created_calendar_reaches_a_created_schedule_until_deleted() {
    let mut server = running().await;
    let calendar = create(
        &server,
        ObjectType::CALENDAR,
        vec![(P::DATE_LIST, OCTOBER_5.to_vec())],
    )
    .await;
    // On the calendar's days, 9.0 from midnight.
    let mut exception = BytesMut::new();
    encode_special_event(
        &mut exception,
        &BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarReference(calendar),
            list_of_time_values: vec![BACnetTimeValue {
                time: Time {
                    hour: 0,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                value: PropertyValue::Real(9.0),
            }],
            event_priority: 1,
        },
    )
    .unwrap();
    let mut values = schedule_values(5.0);
    values.push((P::EXCEPTION_SCHEDULE, exception.to_vec()));
    create(&server, ObjectType::SCHEDULE, values).await;
    assert_eq!(slot(&server).await, PropertyValue::Real(9.0));

    // The calendar no longer holds today: the next pass falls back to the
    // default.
    write(&server, calendar, P::DATE_LIST, vec![]).await;
    next_pass().await;
    assert_eq!(slot(&server).await, PropertyValue::Real(5.0));
    write(&server, calendar, P::DATE_LIST, OCTOBER_5.to_vec()).await;
    next_pass().await;
    assert_eq!(slot(&server).await, PropertyValue::Real(9.0));
    // A deleted calendar is in effect on no day.
    delete(&server, calendar).await;
    next_pass().await;
    assert_eq!(slot(&server).await, PropertyValue::Real(5.0));
    server.stop().await.unwrap();
}

async fn total_records(server: &BACnetServer<TestTransport>, log: ObjectIdentifier) -> u64 {
    match read(server, log, P::TOTAL_RECORD_COUNT, None).await {
        PropertyValue::Unsigned(count) => count,
        other => panic!("{other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_created_trend_log_logs_with_no_restart() {
    let mut server = running().await;
    // Every second.
    let interval = (P::LOG_INTERVAL, value(PropertyValue::Unsigned(100)));
    let log = create(
        &server,
        ObjectType::TREND_LOG,
        vec![
            (P::LOG_DEVICE_OBJECT_PROPERTY, reference(av1())),
            interval.clone(),
        ],
    )
    .await;
    let before = total_records(&server, log).await;
    tokio::time::sleep(Duration::from_millis(3_050)).await;
    assert!(total_records(&server, log).await >= before + 3);

    // One created bare logs nothing until a client gives it a property.
    let bare = create(&server, ObjectType::TREND_LOG, vec![]).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(total_records(&server, bare).await, 0);
    write(
        &server,
        bare,
        P::LOG_DEVICE_OBJECT_PROPERTY,
        reference(av1()),
    )
    .await;
    write(&server, bare, interval.0, interval.1).await;
    let before = total_records(&server, bare).await;
    tokio::time::sleep(Duration::from_millis(3_050)).await;
    assert!(total_records(&server, bare).await >= before + 3);
    server.stop().await.unwrap();
}

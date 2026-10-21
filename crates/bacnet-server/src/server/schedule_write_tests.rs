//! Schedule writes on a running server (#1057): WriteProperty,
//! WritePropertyMultiple and `write_local` of Weekly_Schedule,
//! Exception_Schedule and Effective_Period, each followed at once by the
//! schedule pass for that Schedule and COV for the target it commands. Also
//! Present_Value written while Out_Of_Service is TRUE, which reaches the
//! target the same way (#1055), and the error for an event priority out of
//! range (#1087).
//!
//! The harness clock reads Tuesday 29 September 2026, 15:00. SCH-5 commands
//! AV-1's Present_Value at priority 16 and defaults to 10.0; the first tick at
//! start-up writes that default. Every check below runs within milliseconds
//! of the write, long before the next 60-second tick.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::ObjectType;

pub(super) fn sch5() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::SCHEDULE, 5).unwrap()
}

pub(super) fn schedule(db: &mut ObjectDatabase) {
    let mut schedule = ScheduleObject::new(5, "SCH-5", PropertyValue::Real(10.0)).unwrap();
    schedule
        .add_object_property_reference(BACnetObjectPropertyReference::new(av1(), PV.to_raw()))
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
}

/// Tuesday's daily schedule: `value` from 15:00.
pub(super) fn from_three(value: f32) -> Vec<u8> {
    [&[0x0E, 0xB4, 15, 0, 0, 0][..], &real(value), &[0x0F]].concat()
}

/// A special event in effect every day: `value` from midnight, priority 1.
fn every_day(value: f32) -> Vec<u8> {
    [
        &[0x0E, 0x2B, 0xFF, 0xFF, 0xFF, 0x0F, 0x2E, 0xB4, 0, 0, 0, 0][..],
        &real(value),
        &[0x2F, 0x39, 1],
    ]
    .concat()
}

pub(super) async fn start() -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), schedule).await;
    h.subscribe_cov().await;
    // The initial report carries the default the start-up tick wrote.
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(10.0)
    );
    h
}

pub(super) async fn read(
    h: &Harness,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// AV-1's Present_Value, and the value its COV notification reports.
pub(super) async fn assert_commanded(h: &Harness, value: f32) {
    assert_eq!(read(h, av1(), PV).await, PropertyValue::Real(value));
    assert_eq!(read(h, sch5(), PV).await, PropertyValue::Real(value));
    let notification = h.cov_notification().await;
    assert_eq!(notification.monitored_object_identifier, av1());
    assert_eq!(notification.list_of_values[0].value, real(value));
}

pub(super) async fn write_property(
    h: &mut Harness,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Vec<u8>,
) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: sch5(),
        property_identifier: property,
        property_array_index: index,
        property_value: value,
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    response(h).await
}

/// WritePropertyMultiple of `properties` on SCH-5, in order.
pub(super) async fn write_property_multiple(
    h: &mut Harness,
    properties: Vec<(PropertyIdentifier, Vec<u8>)>,
) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: sch5(),
            list_of_properties: properties
                .into_iter()
                .map(|(property, value)| BACnetPropertyValue {
                    property_identifier: property,
                    property_array_index: None,
                    value,
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
        .await;
    response(h).await
}

#[tokio::test(start_paused = true)]
async fn write_property_of_a_weekly_day_commands_the_target_at_once() {
    let mut h = start().await;
    write_property(
        &mut h,
        PropertyIdentifier::WEEKLY_SCHEDULE,
        Some(2),
        from_three(21.5),
    )
    .await
    .unwrap();
    assert_commanded(&h, 21.5).await;

    // A whole write that empties every day hands back to Schedule_Default.
    write_property(
        &mut h,
        PropertyIdentifier::WEEKLY_SCHEDULE,
        None,
        [0x0E, 0x0F].repeat(7),
    )
    .await
    .unwrap();
    assert_commanded(&h, 10.0).await;
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn a_refused_write_changes_neither_the_schedule_nor_the_target() {
    let mut h = start().await;
    let before = read(&h, sch5(), PropertyIdentifier::WEEKLY_SCHEDULE).await;
    // 15:00 twice in one day.
    let duplicate = [
        &[0x0E, 0xB4, 15, 0, 0, 0][..],
        &real(21.5),
        &[0xB4, 15, 0, 0, 0],
        &real(19.0),
        &[0x0F],
    ]
    .concat();
    assert_eq!(
        write_property(
            &mut h,
            PropertyIdentifier::WEEKLY_SCHEDULE,
            Some(2),
            duplicate,
        )
        .await,
        Err(ErrorCode::DUPLICATE_ENTRY)
    );
    assert_eq!(
        read(&h, sch5(), PropertyIdentifier::WEEKLY_SCHEDULE).await,
        before
    );
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(10.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn write_property_multiple_of_exceptions_and_period_commands_the_target_at_once() {
    let mut h = start().await;
    write_property_multiple(
        &mut h,
        vec![
            (PropertyIdentifier::EXCEPTION_SCHEDULE, every_day(5.0)),
            // September 2026, which holds today.
            (
                PropertyIdentifier::EFFECTIVE_PERIOD,
                vec![0xA4, 126, 9, 1, 2, 0xA4, 126, 9, 30, 3],
            ),
        ],
    )
    .await
    .unwrap();
    assert_commanded(&h, 5.0).await;

    // An Effective_Period that ends yesterday: the Schedule goes inactive,
    // Present_Value keeps its last value and nothing is written.
    write_property(
        &mut h,
        PropertyIdentifier::EFFECTIVE_PERIOD,
        None,
        vec![0xA4, 126, 9, 1, 2, 0xA4, 126, 9, 28, 1],
    )
    .await
    .unwrap();
    assert_eq!(read(&h, sch5(), PV).await, PropertyValue::Real(5.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn write_local_of_a_weekly_day_commands_the_target_at_once() {
    let h = start().await;
    h.server
        .write_local(
            &sch5(),
            PropertyIdentifier::WEEKLY_SCHEDULE,
            Some(2),
            PropertyValue::ApplicationData(from_three(18.0)),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_commanded(&h, 18.0).await;
}

// --- Present_Value while Out_Of_Service (#1055) -------------------------------

const OUT_OF_SERVICE: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;
const TRUE: &[u8] = &[0x11];
const FALSE: &[u8] = &[0x10];

#[tokio::test(start_paused = true)]
async fn write_property_of_present_value_follows_out_of_service() {
    let mut h = start().await;
    assert_eq!(
        write_property(&mut h, PV, None, real(30.0)).await,
        Err(ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(read(&h, sch5(), PV).await, PropertyValue::Real(10.0));
    h.no_notification().await;

    // Going out of service commands nothing; a written value commands the
    // target at once.
    write_property(&mut h, OUT_OF_SERVICE, None, TRUE.to_vec())
        .await
        .unwrap();
    h.no_notification().await;
    write_property(&mut h, PV, None, real(30.0)).await.unwrap();
    assert_commanded(&h, 30.0).await;

    // Neither a content write nor the 60-second tick replaces it.
    write_property(
        &mut h,
        PropertyIdentifier::WEEKLY_SCHEDULE,
        Some(2),
        from_three(21.5),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(read(&h, sch5(), PV).await, PropertyValue::Real(30.0));
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(30.0));
    h.no_notification().await;

    // NULL relinquishes priority 16: AV-1 falls to Relinquish_Default.
    write_property(&mut h, PV, None, vec![0x00]).await.unwrap();
    assert_eq!(read(&h, sch5(), PV).await, PropertyValue::Null);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );

    // Back in service, the calculation takes over at once.
    write_property(&mut h, OUT_OF_SERVICE, None, FALSE.to_vec())
        .await
        .unwrap();
    assert_commanded(&h, 21.5).await;
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn write_property_multiple_of_present_value_around_out_of_service() {
    let mut h = start().await;
    assert_eq!(
        write_property_multiple(&mut h, vec![(PV, real(25.0))]).await,
        Err(ErrorCode::WRITE_ACCESS_DENIED)
    );
    write_property_multiple(
        &mut h,
        vec![(OUT_OF_SERVICE, TRUE.to_vec()), (PV, real(25.0))],
    )
    .await
    .unwrap();
    assert_commanded(&h, 25.0).await;

    // A value equal to the calculated one, then back in service: the written
    // value goes out, and the calculation, agreeing, adds nothing.
    write_property_multiple(
        &mut h,
        vec![(PV, real(10.0)), (OUT_OF_SERVICE, FALSE.to_vec())],
    )
    .await
    .unwrap();
    assert_commanded(&h, 10.0).await;
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn write_local_of_present_value_follows_out_of_service() {
    let h = start().await;
    let sch5 = sch5();
    let write = |property: PropertyIdentifier, value: PropertyValue| {
        h.server.write_local(
            &sch5,
            property,
            None,
            value,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
    };
    let refused = write(PV, PropertyValue::Real(18.0)).await.unwrap_err();
    assert!(
        matches!(refused, Error::Protocol { code, .. }
            if code == ErrorCode::WRITE_ACCESS_DENIED),
        "{refused:?}"
    );
    write(OUT_OF_SERVICE, PropertyValue::Boolean(true))
        .await
        .unwrap();
    write(PV, PropertyValue::Real(18.0)).await.unwrap();
    assert_commanded(&h, 18.0).await;
    write(OUT_OF_SERVICE, PropertyValue::Boolean(false))
        .await
        .unwrap();
    assert_commanded(&h, 10.0).await;
}

// --- Event priority out of range (#1087) --------------------------------------

#[tokio::test(start_paused = true)]
async fn an_event_priority_out_of_range_is_value_out_of_range_over_the_wire() {
    let mut h = start().await;
    let before = read(&h, sch5(), PropertyIdentifier::EXCEPTION_SCHEDULE).await;
    let with_priority = |encoded: &[u8]| {
        let mut event = every_day(5.0);
        event.truncate(event.len() - 2);
        [event.as_slice(), encoded].concat()
    };
    for encoded in [&[0x39, 0][..], &[0x39, 17], &[0x3A, 0x01, 0x2C]] {
        assert_eq!(
            write_property(
                &mut h,
                PropertyIdentifier::EXCEPTION_SCHEDULE,
                None,
                with_priority(encoded),
            )
            .await,
            Err(ErrorCode::VALUE_OUT_OF_RANGE),
            "{encoded:02X?}"
        );
        assert_eq!(
            write_property_multiple(
                &mut h,
                vec![(
                    PropertyIdentifier::EXCEPTION_SCHEDULE,
                    with_priority(encoded)
                )],
            )
            .await,
            Err(ErrorCode::VALUE_OUT_OF_RANGE),
            "{encoded:02X?}"
        );
    }
    assert_eq!(
        read(&h, sch5(), PropertyIdentifier::EXCEPTION_SCHEDULE).await,
        before
    );
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(10.0));
    h.no_notification().await;
}

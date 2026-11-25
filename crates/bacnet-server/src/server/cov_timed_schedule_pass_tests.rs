//! A Schedule's pass that runs under a WritePropertyMultiple's guard and
//! changes an object the request's own attempts already captured (#2582).
//!
//! AV-1 alarms above 80 and SCH-1 commands one of its properties at priority
//! 16. One context timestamps AV-1's Present_Value and Status_Flags and is
//! reported at once (Max_Notification_Delay 0); the Device clock reads
//! 15:00:20, so the rows carry that time unless a test moves it. The pass's
//! change is one more write of the request: it keeps a row of its own when it
//! moves a value, and the object's transition never adds a row after the
//! request's, nor replaces one the request did not queue.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::primitives::Time;

const DEFAULT: PropertyIdentifier = PropertyIdentifier::SCHEDULE_DEFAULT;
const OOS: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;
/// Status_Flags IN_ALARM and OUT_OF_SERVICE, as the wire's last octet
/// carries them.
const IN_ALARM: u8 = 0x80;
const OUT_OF_SERVICE: u8 = 0x10;

fn sch1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap()
}

/// One write of a WritePropertyMultiple: object, property, value, priority.
type Write = (
    ObjectIdentifier,
    PropertyIdentifier,
    PropertyValue,
    Option<u8>,
);

/// A row as the report carries it: property, value, Time of Change.
type Row = (PropertyIdentifier, Vec<u8>, Option<Time>);

fn pv(value: f32) -> Row {
    pv_at(value, 20)
}

fn sf(bits: u8) -> Row {
    sf_at(bits, 20)
}

fn pv_at(value: f32, second: u8) -> Row {
    (PV, real(value), Some(time(second)))
}

fn sf_at(bits: u8, second: u8) -> Row {
    (SF, vec![0x82, 0x04, bits], Some(time(second)))
}

fn schedule(value: f32) -> Write {
    (sch1(), DEFAULT, PropertyValue::Real(value), None)
}

fn target(value: f32, priority: Option<u8>) -> Write {
    (av1(), PV, PropertyValue::Real(value), priority)
}

/// A server whose SCH-1 commands AV-1's `property`, by default `default`,
/// with the context subscribed with Max_Notification_Delay `delay`, its
/// initial report taken and the clock at :20.
async fn start(property: PropertyIdentifier, default: PropertyValue, delay: u32) -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        high_limit_alarm(db);
        let mut schedule = ScheduleObject::new(1, "SCH-1", default).unwrap();
        let reference = BACnetObjectPropertyReference::new(av1(), property.to_raw());
        schedule.add_object_property_reference(reference).unwrap();
        db.add(Box::new(schedule)).unwrap();
    })
    .await;
    let specs = vec![(av1(), vec![(PV, true), (SF, true)])];
    h.subscribe_with_delay(false, specs, delay).await;
    h.notification().await;
    h.set_clock(20);
    h
}

/// [`start`] with SCH-1 on AV-1's Present_Value, by default 0.
async fn on_present_value() -> Harness {
    start(PV, PropertyValue::Real(0.0), 0).await
}

fn wpm_body(writes: &[Write]) -> BytesMut {
    let list_of_write_access_specs = writes
        .iter()
        .map(|(object, property, value, priority)| {
            let mut encoded = BytesMut::new();
            bacnet_encoding::primitives::encode_property_value(&mut encoded, value).unwrap();
            WriteAccessSpecification {
                object_identifier: *object,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: *property,
                    property_array_index: None,
                    value: encoded.to_vec(),
                    priority: *priority,
                }],
            }
        })
        .collect();
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs,
    }
    .encode(&mut body)
    .unwrap();
    body
}

/// The rows of the next report, checking it is the only one: nothing
/// follows it, the envelope names :20 and no other object is reported.
async fn the_report(h: &Harness) -> Vec<Row> {
    let report = h.notification().await;
    assert_eq!(envelope(&report).map(|(_, time)| time), Some(time(20)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let another = h.frames.lock().unwrap().iter().any(|apdu| match apdu {
        Apdu::UnconfirmedRequest(request) => {
            request.service_choice
                == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE
        }
        _ => false,
    });
    assert!(!another, "one report");
    rows(&report)
}

/// Send a WritePropertyMultiple of `writes`; its answer, and the rows of the
/// one report it causes.
async fn wpm(h: &mut Harness, writes: &[Write]) -> (Result<(), ErrorPdu>, Vec<Row>) {
    h.request(
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm_body(writes),
    )
    .await;
    let answer = error_response(h).await;
    (answer, the_report(h).await)
}

async fn read(
    h: &Harness,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let db = h.server.database().read().await;
    db.get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// The report: SCH-1 written to 95, which the pass writes at priority 16,
/// and AV-1 written to 90 at priority 8, which wins. On dev the pass
/// committed AV-1's IN_ALARM and added its own row after the attempt's, so
/// the report went (90, normal) then (90, in alarm). Now one row, in alarm,
/// as when AV-1 alone is written, whichever entry comes first.
#[tokio::test(start_paused = true)]
async fn a_schedule_and_its_target_written_together_send_one_settled_row() {
    for writes in [
        [schedule(95.0), target(90.0, Some(8))],
        [target(90.0, Some(8)), schedule(95.0)],
    ] {
        let mut h = on_present_value().await;
        let (answer, rows) = wpm(&mut h, &writes).await;
        assert_eq!(answer, Ok(()));
        assert_eq!(rows, [pv(90.0), sf(IN_ALARM)], "{writes:?}");
        assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(90.0));
        h.server.stop().await.unwrap();
    }
}

/// AV-1 written to 90 at priority 16, then SCH-1 to 95: the pass writes the
/// same slot after the attempts, so 95 wins. The request's write and the
/// pass's are two writes with a row each, the transition folded into the
/// second, as for two attempts on AV-1 (#856, #2535). So the first row
/// carries the Status_Flags from before the transition, by the one row per
/// write rule, as two attempts writing 90 then 95 do
/// (`cov_timed_multiple_tests`). The same as dev.
#[tokio::test(start_paused = true)]
async fn a_value_the_pass_moves_keeps_the_requests_row() {
    let mut h = on_present_value().await;
    let (answer, rows) = wpm(&mut h, &[target(90.0, None), schedule(95.0)]).await;
    assert_eq!(answer, Ok(()));
    assert_eq!(rows, [pv(90.0), sf(0), pv(95.0), sf(IN_ALARM)]);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(95.0));
    h.server.stop().await.unwrap();
}

/// Two attempts on AV-1 before the pass, 50 then 90 at priority 8, with
/// SCH-1's 95 beneath them: the transition 90 causes takes the place of the
/// newest row, 90's, and 50's row stays the first write's.
#[tokio::test(start_paused = true)]
async fn the_newest_of_the_requests_rows_is_the_one_replaced() {
    let mut h = on_present_value().await;
    let writes = [target(50.0, Some(8)), target(90.0, Some(8)), schedule(95.0)];
    let (answer, rows) = wpm(&mut h, &writes).await;
    assert_eq!(answer, Ok(()));
    assert_eq!(rows, [pv(50.0), sf(0), pv(90.0), sf(IN_ALARM)]);
    h.server.stop().await.unwrap();
}

/// AV-1 written to 50 at priority 8 at :15 while reports fail, so its row
/// waits (Max_Notification_Delay 10 s); then, sending again, a
/// WritePropertyMultiple of `second` at :20. The rows of the one report
/// that follows.
async fn after_a_pending_row(second: &[Write]) -> Vec<Row> {
    let mut h = start(PV, PropertyValue::Real(0.0), 10).await;
    h.fail_notifications.store(true, Ordering::Release);
    h.set_clock(15);
    h.write_pv(50.0, 15).await;
    assert_eq!(response(&h).await, Ok(()));
    h.settle().await;
    *h.after_ack.lock().unwrap() = None;
    h.fail_notifications.store(false, Ordering::Release);
    h.set_clock(20);
    let (answer, rows) = wpm(&mut h, second).await;
    assert_eq!(answer, Ok(()));
    h.server.stop().await.unwrap();
    rows
}

/// A row an earlier request left pending is never what a later request's
/// capture after the transition replaces. Writing AV-1's value, the second
/// request's own row is the newest and is replaced; writing only its
/// High_Limit, it queues none, so AV-1's transition adds a row after the
/// first request's.
#[tokio::test(start_paused = true)]
async fn a_row_an_earlier_request_left_pending_is_never_replaced() {
    let rows = after_a_pending_row(&[schedule(95.0), target(90.0, Some(8))]).await;
    assert_eq!(
        rows,
        [pv_at(50.0, 15), sf_at(0, 15), pv(90.0), sf(IN_ALARM)]
    );
    let limit = (
        av1(),
        PropertyIdentifier::HIGH_LIMIT,
        PropertyValue::Real(40.0),
        None,
    );
    let rows = after_a_pending_row(&[schedule(95.0), limit]).await;
    assert_eq!(
        rows,
        [pv_at(50.0, 15), sf_at(0, 15), pv(50.0), sf(IN_ALARM)]
    );
}

/// A pass that commits no transition: the pass's 50 replaces the request's
/// 90 at priority 16 (two rows), or the pass writes beneath the request's
/// 50 at priority 8 (one row). Both as dev.
#[tokio::test(start_paused = true)]
async fn a_pass_without_a_transition_leaves_the_rows_as_they_were() {
    let mut h = on_present_value().await;
    let (_, rows) = wpm(&mut h, &[target(90.0, None), schedule(50.0)]).await;
    assert_eq!(rows, [pv(90.0), pv(50.0), sf(0)]);
    h.server.stop().await.unwrap();
    let mut h = on_present_value().await;
    let (_, rows) = wpm(&mut h, &[schedule(95.0), target(50.0, Some(8))]).await;
    assert_eq!(rows, [pv(50.0), sf(0)]);
    h.server.stop().await.unwrap();
}

/// SCH-1 commands AV-1's Out_Of_Service: the request writes AV-1 to 90,
/// and the pass takes it out of service. The pass's change moves
/// Status_Flags before the transition does; both land in its one row, and
/// Status_Flags' own reference gets no row showing OUT_OF_SERVICE alone.
/// As dev, and as one request writing AV-1's value then Out_Of_Service.
#[tokio::test(start_paused = true)]
async fn a_pass_that_moves_status_flags_keeps_one_row_per_write() {
    let mut h = start(OOS, PropertyValue::Boolean(false), 0).await;
    let writes = [
        (sch1(), DEFAULT, PropertyValue::Boolean(true), None),
        target(90.0, Some(8)),
    ];
    let (answer, rows) = wpm(&mut h, &writes).await;
    assert_eq!(answer, Ok(()));
    let settled = [sf(0), pv(90.0), sf(IN_ALARM | OUT_OF_SERVICE)];
    assert_eq!(rows, settled);
    assert_eq!(read(&h, av1(), OOS).await, PropertyValue::Boolean(true));
    h.server.stop().await.unwrap();
    let mut h = on_present_value().await;
    let writes = [
        target(90.0, Some(8)),
        (av1(), OOS, PropertyValue::Boolean(true), None),
    ];
    assert_eq!(wpm(&mut h, &writes).await.1, settled);
    h.server.stop().await.unwrap();
}

/// A later entry fails after SCH-1 and AV-1 were written: the answer names
/// it, nothing is rolled back (Clause 15.10), and the pass still runs for
/// the written prefix. The report is the successful request's one row; on
/// dev it was the same two rows as the successful request's.
#[tokio::test(start_paused = true)]
async fn a_failed_wpm_keeps_its_prefix_and_sends_one_settled_row() {
    let mut h = on_present_value().await;
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap();
    let writes = [
        schedule(95.0),
        target(90.0, Some(8)),
        (missing, PV, PropertyValue::Real(1.0), None),
    ];
    let (answer, rows) = wpm(&mut h, &writes).await;
    let error = answer.expect_err("the third entry fails");
    assert_eq!(error.error_code, ErrorCode::UNKNOWN_OBJECT);
    assert_eq!(rows, [pv(90.0), sf(IN_ALARM)]);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(90.0));
    assert_eq!(read(&h, sch1(), DEFAULT).await, PropertyValue::Real(95.0));
    h.server.stop().await.unwrap();
}

/// Neither alone, nor a WriteProperty or a local write of SCH-1, nor a
/// second request, records more than one row a change.
#[tokio::test(start_paused = true)]
async fn a_write_of_one_of_them_sends_one_row() {
    let mut h = on_present_value().await;
    let (_, rows) = wpm(&mut h, &[target(90.0, Some(8))]).await;
    assert_eq!(rows, [pv(90.0), sf(IN_ALARM)]);
    h.server.stop().await.unwrap();
    let mut h = on_present_value().await;
    let (_, rows) = wpm(&mut h, &[schedule(95.0)]).await;
    assert_eq!(rows, [pv(95.0), sf(IN_ALARM)]);
    h.server.stop().await.unwrap();

    // A WriteProperty of SCH-1, then a second request writing AV-1.
    let mut h = on_present_value().await;
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut value, &PropertyValue::Real(95.0))
        .unwrap();
    let mut body = BytesMut::new();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: sch1(),
        property_identifier: DEFAULT,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    assert_eq!(the_report(&h).await, [pv(95.0), sf(IN_ALARM)]);
    h.write_pv(90.0, 20).await;
    assert_eq!(the_report(&h).await, [pv(90.0), sf(IN_ALARM)]);
    h.server.stop().await.unwrap();

    let mut h = on_present_value().await;
    let (source, value) = (
        crate::LocalCommandSource::ServerDevice,
        PropertyValue::Real(95.0),
    );
    let schedule = sch1();
    let write = h
        .server
        .write_local(&schedule, DEFAULT, None, value, None, source);
    write.await.unwrap();
    assert_eq!(the_report(&h).await, [pv(95.0), sf(IN_ALARM)]);
    h.server.stop().await.unwrap();
}

/// A second context, process 857, takes the same references untimestamped:
/// it is reported from the object after the request, as before, while the
/// timestamped one gets its one settled row.
#[tokio::test(start_paused = true)]
async fn timestamped_and_untimestamped_contexts_both_report_the_settled_target() {
    let mut h = on_present_value().await;
    let specs = vec![(av1(), vec![(PV, false), (SF, false)])];
    h.subscribe_process(857, false, specs, Some(0)).await;
    h.notification().await;
    h.request(
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm_body(&[schedule(95.0), target(90.0, Some(8))]),
    )
    .await;
    assert_eq!(error_response(&h).await, Ok(()));
    let mut reports = [h.notification().await, h.notification().await];
    reports.sort_by_key(|report| report.subscriber_process_identifier);
    assert_eq!(rows(&reports[0]), [pv(90.0), sf(IN_ALARM)]);
    let untimed = |(property, value, _): Row| (property, value, None);
    let settled = [untimed(pv(90.0)), untimed(sf(IN_ALARM))];
    assert_eq!(rows(&reports[1]), settled);
    h.server.stop().await.unwrap();
}

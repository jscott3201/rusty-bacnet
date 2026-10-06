//! Received notifications in the Event Logs that collect them (#1346), and
//! BUFFER_READY reports from the three log types (#1347), on a running
//! server.
use super::*;
use crate::server::RECEIVED_EVENT_LOG_RATE;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_types::constructed::{BACnetLogMultipleRecord, BACnetLogRecord, LogData, LogDatum};
use bacnet_types::enums::{LoggingType, NotifyType};

/// Another device on this link.
const OTHER: [u8; 6] = [10, 0, 0, 6, 0xBA, 0xC0];

/// `PEER`'s alarm with Process Identifier `process`, so each one sent can be
/// told apart in the log.
fn alarm_numbered(process: u32) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: process,
        ..received_alarm()
    }
}

/// Deliver an UnconfirmedEventNotification from `mac`, broadcast on the
/// link when `broadcast`.
async fn receive_from(
    h: &Harness,
    mac: [u8; 6],
    broadcast: bool,
    notification: &EventNotificationRequest,
) {
    let mut service_request = BytesMut::new();
    encode_event_notification(notification, &mut service_request).unwrap();
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    h.tx.send(ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&mac),
        link_layer_group: broadcast,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    })
    .await
    .unwrap();
}

/// The record a collecting log holds for `notification`, received at
/// `second`: the notification as it decoded, Process Identifier included.
fn arrived(second: u8, notification: &EventNotificationRequest) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: at(second).local_date,
        time: time(second),
        log_datum: EventLogDatum::Notification(notification.clone()),
    }
}

/// EL-1 collects received notifications and reports every `threshold`
/// records through Notification Class 0 to `PEER`; EL-2 doesn't collect.
fn collecting(db: &mut ObjectDatabase, threshold: u32) {
    alarm(db, to_peer());
    let mut collector = EventLogObject::new(1, "EL-1", 32).unwrap();
    collector.set_log_received_notifications(true);
    collector.set_notification_threshold(threshold);
    db.add(Box::new(collector)).unwrap();
    logs(db, &[2], 32);
}

async fn unsigned(h: &Harness, log: ObjectIdentifier, property: PropertyIdentifier) -> u64 {
    let db = h.server.database().read().await;
    match db.get(&log).unwrap().read_property(property, None).unwrap() {
        PropertyValue::Unsigned(value) => value,
        other => panic!("{property:?} read {other:?}"),
    }
}

/// Unicast, broadcast and confirmed notifications all go in a log that
/// collects them, as they decoded, and count toward its threshold; a log
/// without the opt-in takes none of them.
#[tokio::test(start_paused = true)]
async fn a_collecting_log_records_what_arrives_and_reports_it() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| collecting(db, 3)).await;
    h.set_clock(41);
    receive_from(&h, PEER, false, &alarm_numbered(1)).await;
    h.settle().await;
    h.set_clock(42);
    receive_from(&h, OTHER, true, &alarm_numbered(2)).await;
    h.settle().await;
    h.set_clock(43);
    let mut body = BytesMut::new();
    encode_event_notification(&alarm_numbered(3), &mut body).unwrap();
    h.request(ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION, body)
        .await;
    response(&h).await.unwrap();

    let report = sent_notification(&h).await;
    assert_eq!(report.event_object_identifier, el(1));
    assert_eq!(
        report.event_values,
        Some(NotificationParameters::BufferReady {
            buffer_property: BACnetDeviceObjectPropertyReference {
                object_identifier: el(1),
                property_identifier: PropertyIdentifier::LOG_BUFFER.to_raw(),
                property_array_index: None,
                device_identifier: Some(report.initiating_device_identifier),
            },
            previous_notification: 0,
            current_notification: 3,
        })
    );
    assert_eq!(
        read_log(&mut h, el(1)).await,
        [
            arrived(41, &alarm_numbered(1)),
            arrived(42, &alarm_numbered(2)),
            arrived(43, &alarm_numbered(3)),
        ]
    );
    assert!(read_log(&mut h, el(2)).await.is_empty());
    h.server.stop().await.unwrap();
}

/// One source floods: its allowance goes in, the rest is counted and not
/// logged, and the next second gives it its allowance again. Another source
/// keeps its own allowance throughout.
#[tokio::test(start_paused = true)]
async fn a_flood_from_one_source_is_held_to_its_allowance() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| collecting(db, 0)).await;
    h.set_clock(41);
    let rate = RECEIVED_EVENT_LOG_RATE;
    for process in 0..rate + 3 {
        receive_from(&h, PEER, true, &alarm_numbered(process)).await;
    }
    h.settle().await;
    receive_from(&h, OTHER, true, &alarm_numbered(100)).await;
    h.settle().await;
    assert_eq!(
        h.server.event_notification_counters().received_not_logged,
        3
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    h.set_clock(42);
    receive_from(&h, PEER, true, &alarm_numbered(200)).await;
    h.settle().await;

    let mut expected: Vec<_> = (0..rate)
        .map(|process| arrived(41, &alarm_numbered(process)))
        .collect();
    expected.push(arrived(41, &alarm_numbered(100)));
    expected.push(arrived(42, &alarm_numbered(200)));
    assert_eq!(read_log(&mut h, el(1)).await, expected);
    assert_eq!(
        h.server.event_notification_counters().received_not_logged,
        3
    );
    h.server.stop().await.unwrap();
}

/// The receive path reads the allowance's window on tokio's clock (#1550),
/// so it closes exactly one second after it opened: a notification 1 ms
/// before then is still over the allowance, and one at the second is logged.
#[tokio::test(start_paused = true)]
async fn a_source_allowance_reopens_exactly_one_window_later() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| collecting(db, 0)).await;
    h.set_clock(41);
    let opened = tokio::time::Instant::now();
    let rate = RECEIVED_EVENT_LOG_RATE;
    for process in 0..rate {
        receive_from(&h, PEER, true, &alarm_numbered(process)).await;
    }
    h.settle().await;
    // `settle` lets a millisecond pass; step to the instant each send needs.
    let advance_to =
        |at: tokio::time::Instant| tokio::time::advance(at - tokio::time::Instant::now());

    advance_to(opened + Duration::from_millis(999)).await;
    receive_from(&h, PEER, true, &alarm_numbered(100)).await;
    h.settle().await;
    let not_logged = || h.server.event_notification_counters().received_not_logged;
    assert_eq!(not_logged(), 1, "1 ms before the window ends");

    advance_to(opened + Duration::from_secs(1)).await;
    receive_from(&h, PEER, true, &alarm_numbered(200)).await;
    h.settle().await;
    assert_eq!(not_logged(), 1, "at the window's end");

    let mut expected: Vec<_> = (0..rate)
        .map(|process| arrived(41, &alarm_numbered(process)))
        .collect();
    expected.push(arrived(41, &alarm_numbered(200)));
    assert_eq!(read_log(&mut h, el(1)).await, expected);
    h.server.stop().await.unwrap();
}

/// With Notification_Threshold 2, the device's own alarm and return to
/// normal make one BUFFER_READY report to `PEER`, carrying both counts. The
/// report goes in no log and doesn't count toward the next one, which comes
/// after two more records.
#[tokio::test(start_paused = true)]
async fn every_two_records_make_one_report_that_adds_no_record() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| collecting(db, 2)).await;
    h.set_clock(40);
    h.write_local(90.0).await;
    let alarm = sent_notification(&h).await;
    h.set_clock(41);
    h.write_local(10.0).await;
    let normal = sent_notification(&h).await;
    let report = sent_notification(&h).await;
    run_passes(&h, 3).await;

    assert_eq!(
        (report.event_type, report.from_state, report.to_state),
        (
            EventType::BUFFER_READY,
            EventState::NORMAL,
            EventState::NORMAL
        )
    );
    assert_eq!(report.notify_type, NotifyType::EVENT);
    assert_eq!(report.process_identifier, PROCESS);
    assert!(matches!(
        report.event_values,
        Some(NotificationParameters::BufferReady {
            previous_notification: 0,
            current_notification: 2,
            ..
        })
    ));
    assert!(!notification_pending(&h), "one report for two records");
    for log in [el(1), el(2)] {
        assert_eq!(
            read_log(&mut h, log).await,
            [logged(40, &alarm), logged(41, &normal)]
        );
    }
    let since = PropertyIdentifier::RECORDS_SINCE_NOTIFICATION;
    assert_eq!(unsigned(&h, el(1), since).await, 0);
    assert_eq!(
        unsigned(&h, el(1), PropertyIdentifier::LAST_NOTIFY_RECORD).await,
        2
    );

    h.set_clock(50);
    h.write_local(90.0).await;
    sent_notification(&h).await;
    run_passes(&h, 2).await;
    assert!(!notification_pending(&h), "one record is not enough");
    h.set_clock(51);
    h.write_local(10.0).await;
    sent_notification(&h).await;
    let next = sent_notification(&h).await;
    assert!(matches!(
        next.event_values,
        Some(NotificationParameters::BufferReady {
            previous_notification: 2,
            current_notification: 4,
            ..
        })
    ));
    h.server.stop().await.unwrap();
}

/// A confirmed notification sent again with the same invoke ID, as a sender
/// that missed the acknowledgment does, is acknowledged twice and logged
/// once. The window that recognizes the repeat is on tokio's clock (#1556),
/// so a runner stall between the two can't end it.
#[tokio::test(start_paused = true)]
async fn a_retransmitted_confirmed_notification_is_logged_once() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| collecting(db, 0)).await;
    h.set_clock(41);
    let mut body = BytesMut::new();
    encode_event_notification(&alarm_numbered(1), &mut body).unwrap();
    h.request(
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        body.clone(),
    )
    .await;
    response(&h).await.unwrap();
    h.set_clock(42);
    h.invoke_id = h.invoke_id.wrapping_sub(1);
    h.request(ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION, body)
        .await;
    response(&h).await.unwrap();

    assert_eq!(
        read_log(&mut h, el(1)).await,
        [arrived(41, &alarm_numbered(1))]
    );
    h.server.stop().await.unwrap();
}

fn tl1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

fn tlm1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG_MULTIPLE, 1).unwrap()
}

/// The process identifier Notification Class 5 names for `PEER`.
const CLASS_5_PROCESS: u32 = 11;

/// TL-1 (TRIGGERED, so the poller adds nothing) reports every two records
/// through Notification Class 0; TLM-1 every record through Notification
/// Class 5, whose destination is `PEER` as process [`CLASS_5_PROCESS`]. EL-1
/// logs the device's notifications.
fn trend_logs(db: &mut ObjectDatabase) {
    db.add(Box::new(to_peer())).unwrap();
    let mut class = NotificationClass::new(5, "NC-5").unwrap();
    class
        .add_destination(destination(
            address_recipient(0, &PEER),
            CLASS_5_PROCESS,
            false,
        ))
        .unwrap();
    db.add(Box::new(class)).unwrap();
    let mut trend = TrendLogObject::new(1, "TL-1", 8).unwrap();
    trend.set_logging_type(LoggingType::TRIGGERED).unwrap();
    trend.set_notification_threshold(2);
    db.add(Box::new(trend)).unwrap();
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    multiple.set_notification_threshold(1);
    multiple.set_notification_class(5);
    db.add(Box::new(multiple)).unwrap();
    logs(db, &[1], 8);
}

/// The BUFFER_READY event values a report on `log`'s buffer in `device`
/// carries.
fn buffer_ready(
    log: ObjectIdentifier,
    device: ObjectIdentifier,
    previous: u32,
    current: u32,
) -> NotificationParameters {
    NotificationParameters::BufferReady {
        buffer_property: BACnetDeviceObjectPropertyReference {
            object_identifier: log,
            property_identifier: PropertyIdentifier::LOG_BUFFER.to_raw(),
            property_array_index: None,
            device_identifier: Some(device),
        },
        previous_notification: previous,
        current_notification: current,
    }
}

/// A Trend Log's and a Trend Log Multiple's reports go to the recipients of
/// each one's own Notification Class, naming its Log_Buffer in this device
/// with both counts. The Event Log logs both, and neither counts toward the
/// next report of the log it is about.
#[tokio::test(start_paused = true)]
async fn trend_log_reports_follow_their_class_and_are_logged() {
    let mut h = Harness::start_with(ServerConfig::default(), trend_logs).await;
    h.set_clock(40);
    {
        let mut db = h.server.database().write().await;
        for _ in 0..2 {
            db.get_mut(&tl1())
                .unwrap()
                .add_trend_record(BACnetLogRecord {
                    date: at(40).local_date,
                    time: time(40),
                    log_datum: LogDatum::UnsignedValue(7),
                    status_flags: None,
                })
                .unwrap();
        }
        db.get_mut(&tlm1())
            .unwrap()
            .add_trend_multiple_record(BACnetLogMultipleRecord {
                date: at(40).local_date,
                time: time(40),
                log_data: LogData::Values(Vec::new()),
            })
            .unwrap();
    }
    run_passes(&h, 1).await;
    let mut reports = vec![sent_notification(&h).await, sent_notification(&h).await];
    reports.sort_by_key(|report| report.event_object_identifier.object_type().to_raw());
    let [trend, multiple] = <[_; 2]>::try_from(reports).unwrap();
    let device = trend.initiating_device_identifier;
    assert_eq!(device.object_type(), ObjectType::DEVICE);
    assert_eq!(
        (
            trend.event_object_identifier,
            trend.event_type,
            trend.to_state
        ),
        (tl1(), EventType::BUFFER_READY, EventState::NORMAL)
    );
    assert_eq!(trend.process_identifier, PROCESS);
    assert_eq!(trend.event_values, Some(buffer_ready(tl1(), device, 0, 2)));
    assert_eq!(multiple.event_object_identifier, tlm1());
    assert_eq!(multiple.notification_class, 5);
    assert_eq!(multiple.process_identifier, CLASS_5_PROCESS);
    assert_eq!(
        multiple.event_values,
        Some(buffer_ready(tlm1(), device, 0, 1))
    );

    run_passes(&h, 3).await;
    assert!(!notification_pending(&h), "one report each");
    let mut logged_reports = read_log(&mut h, el(1)).await;
    logged_reports.sort_by_key(|record| match &record.log_datum {
        EventLogDatum::Notification(n) => n.event_object_identifier.object_type().to_raw(),
        _ => u32::MAX,
    });
    assert_eq!(logged_reports, [logged(40, &trend), logged(40, &multiple)]);
    let db = h.server.database().read().await;
    for (log, total) in [(tl1(), 2), (tlm1(), 1)] {
        let read = |property| db.get(&log).unwrap().read_property(property, None).unwrap();
        assert_eq!(
            read(PropertyIdentifier::TOTAL_RECORD_COUNT),
            PropertyValue::Unsigned(total)
        );
        assert_eq!(
            read(PropertyIdentifier::RECORDS_SINCE_NOTIFICATION),
            PropertyValue::Unsigned(0)
        );
    }
    drop(db);
    h.server.stop().await.unwrap();
}

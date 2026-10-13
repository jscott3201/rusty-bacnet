//! `bacnet read-range` decodes the records of a log buffer by the log object's
//! type (#1274), against a loopback server.
#[allow(dead_code)]
mod support;

use std::net::Ipv4Addr;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum,
    EventNotificationRequest, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{EventState, EventType, NotifyType, ObjectType};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, StatusFlags, Time};
use serde_json::json;
use support::run;

const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 3,
    day_of_week: 6,
};

fn time(hour: u8) -> Time {
    Time {
        hour,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn alarm() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: oid(ObjectType::DEVICE, 1),
        event_object_identifier: oid(ObjectType::ANALOG_INPUT, 3),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("too hot".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: None,
    }
}

/// A server on loopback holding Trend Log 1, Event Log 1 and Trend Log
/// Multiple 1, each with records.
async fn server() -> BACnetServer<BipTransport> {
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 1,
        name: "read-range".into(),
        ..DeviceConfig::default()
    })
    .unwrap();

    let mut trend = TrendLogObject::new(1, "TL-1", 10).unwrap();
    trend
        .add_record(BACnetLogRecord {
            date: DATE,
            time: time(8),
            log_datum: LogDatum::RealValue(72.5),
            status_flags: Some(StatusFlags::IN_ALARM),
        })
        .unwrap();
    trend
        .add_record(BACnetLogRecord {
            date: DATE,
            time: time(9),
            log_datum: LogDatum::LogStatus(LogStatus::BUFFER_PURGED),
            status_flags: None,
        })
        .unwrap();

    let mut events = EventLogObject::new(1, "EL-1", 10).unwrap();
    for (hour, log_datum) in [
        (8, EventLogDatum::Notification(alarm())),
        (9, EventLogDatum::TimeChange(-1.5)),
    ] {
        events
            .add_record(BACnetEventLogRecord {
                date: DATE,
                time: time(hour),
                log_datum,
            })
            .unwrap();
    }

    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 10).unwrap();
    multiple
        .add_record(BACnetLogMultipleRecord {
            date: DATE,
            time: time(8),
            log_data: LogData::Values(vec![LogValue::RealValue(21.5), LogValue::NullValue]),
        })
        .unwrap();

    // Trend Log 2: 100 records a second apart from 08:00:00, each holding
    // its own sequence number, enough that a 40-record page needs three.
    let mut long = TrendLogObject::new(2, "TL-2", 200).unwrap();
    for sequence in 1..=100u8 {
        long.add_record(BACnetLogRecord {
            date: DATE,
            time: Time {
                hour: 8,
                minute: (sequence - 1) / 60,
                second: (sequence - 1) % 60,
                hundredths: 0,
            },
            log_datum: LogDatum::UnsignedValue(u64::from(sequence)),
            status_flags: None,
        })
        .unwrap();
    }

    let objects: Vec<Box<dyn BACnetObject>> = vec![
        Box::new(trend),
        Box::new(events),
        Box::new(multiple),
        Box::new(long),
    ];
    let mut list = vec![device.object_identifier()];
    list.extend(objects.iter().map(|object| object.object_identifier()));
    device.set_object_list(list);

    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    for object in objects {
        db.add(object).unwrap();
    }
    BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap()
}

/// The server's `ip:port`, as the CLI takes a target.
fn target(server: &BACnetServer<BipTransport>) -> String {
    let mac = server.local_mac();
    let port = u16::from_be_bytes([mac[4], mac[5]]);
    format!("{}:{port}", Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]))
}

/// Run `read-range` against `target` for `object` with output `format`,
/// returning its stdout.
async fn read_range(target: &str, object: &str, format: &str) -> String {
    read_range_with(target, object, format, &[]).await
}

/// `read_range` with extra flags after the object.
async fn read_range_with(target: &str, object: &str, format: &str, flags: &[&str]) -> String {
    let mut args = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        format,
        "read-range",
        target,
        object,
    ];
    args.extend_from_slice(flags);
    let output = run(args).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// The unsigned values of `records`: Trend Log 2's sequence numbers.
fn values(json: &serde_json::Value) -> Vec<u64> {
    json["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["datum"].as_str().unwrap().parse().unwrap())
        .collect()
}

#[tokio::test]
async fn read_range_prints_decoded_log_records() {
    let mut server = server().await;
    let target = target(&server);

    let trend: serde_json::Value =
        serde_json::from_str(&read_range(&target, "trend-log:1", "json").await).unwrap();
    assert_eq!(
        trend,
        json!({
            "object": "TREND_LOG:1",
            "property": "LOG_BUFFER",
            "item_count": 2,
            "result_flags": {"first_item": true, "last_item": true, "more_items": false},
            "first_sequence_number": null,
            "violations": [],
            "records": [
                {
                    "timestamp": "2026-10-03 08:00:00.00",
                    "datum": "72.5",
                    "status_flags": "IN_ALARM",
                },
                {
                    "timestamp": "2026-10-03 09:00:00.00",
                    "datum": "log-status BUFFER_PURGED",
                },
            ],
        })
    );

    let events: serde_json::Value =
        serde_json::from_str(&read_range(&target, "event-log:1", "json").await).unwrap();
    assert_eq!(
        events,
        json!({
            "object": "EVENT_LOG:1",
            "property": "LOG_BUFFER",
            "item_count": 2,
            "result_flags": {"first_item": true, "last_item": true, "more_items": false},
            "first_sequence_number": null,
            "violations": [],
            "records": [
                {
                    "timestamp": "2026-10-03 08:00:00.00",
                    "datum": "ALARM OUT_OF_RANGE ANALOG_INPUT:3 NORMAL -> HIGH_LIMIT \"too hot\"",
                },
                {
                    "timestamp": "2026-10-03 09:00:00.00",
                    "datum": "time-change -1.5 s",
                },
            ],
        })
    );

    let multiple: serde_json::Value =
        serde_json::from_str(&read_range(&target, "trend-log-multiple:1", "json").await).unwrap();
    assert_eq!(
        multiple["records"],
        json!([{ "timestamp": "2026-10-03 08:00:00.00", "datum": "[21.5, null]" }])
    );

    // The table shows the same records, one row each, with nothing left as
    // hex; only the Trend Log's has a status flags column.
    let table = read_range(&target, "event-log:1", "table").await;
    assert!(table
        .starts_with("ReadRange EVENT_LOG:1  LOG_BUFFER  count=2  flags=FIRST_ITEM,LAST_ITEM\n"));
    for cell in [
        "2026-10-03 08:00:00.00",
        "ALARM OUT_OF_RANGE ANALOG_INPUT:3 NORMAL -> HIGH_LIMIT \"too hot\"",
        "time-change -1.5 s",
    ] {
        assert!(table.contains(cell), "{cell}: {table}");
    }
    assert!(!table.contains("Status flags"), "{table}");
    assert!(!table.contains("[raw]"), "{table}");
    let table = read_range(&target, "trend-log:1", "table").await;
    assert!(table.contains("Status flags"), "{table}");
    assert!(table.contains("IN_ALARM"), "{table}");

    server.stop().await.unwrap();
}

/// A range, its flags and first sequence number, and `--all` paging a log
/// that needs several pages, ending with the checkpoint (#1532).
#[tokio::test]
async fn read_range_takes_a_range_shows_its_flags_and_pages_a_whole_log() {
    let mut server = server().await;
    let target = target(&server);
    let read = |format: &'static str, flags: &'static [&'static str]| {
        let target = target.clone();
        async move { read_range_with(&target, "trend-log:2", format, flags).await }
    };
    let json = |text: String| serde_json::from_str::<serde_json::Value>(&text).unwrap();

    let by_position = json(read("json", &["--position", "1", "--count", "5"]).await);
    assert_eq!(values(&by_position), [1, 2, 3, 4, 5]);
    assert_eq!(
        by_position["result_flags"],
        json!({"first_item": true, "last_item": false, "more_items": false})
    );
    assert_eq!(by_position["first_sequence_number"], json!(null));

    let by_sequence = json(read("json", &["--sequence", "50", "--count", "3"]).await);
    assert_eq!(values(&by_sequence), [50, 51, 52]);
    assert_eq!(by_sequence["first_sequence_number"], json!(50));

    let backward = json(read("json", &["--sequence", "50", "--count", "-2"]).await);
    assert_eq!(values(&backward), [49, 50]);

    // Records a second apart from 08:00:00: after 08:00:30 is record 32.
    let by_time = json(read("json", &["--time", "2026-10-03T08:00:30", "--count", "2"]).await);
    assert_eq!(values(&by_time), [32, 33]);
    assert_eq!(by_time["first_sequence_number"], json!(32));

    let all = json(read("json", &["--all", "--count", "40"]).await);
    assert_eq!(values(&all), (1..=100).collect::<Vec<u64>>());
    assert_eq!(all["item_count"], json!(100));
    assert_eq!(all["pages"], json!(3));
    assert_eq!(all["next"], json!("--sequence 101"));
    assert_eq!(all["gaps"], json!([]));
    assert_eq!(all["first_sequence_number"], json!(1));
    assert_eq!(all["result_flags"]["last_item"], json!(true));

    let resumed = json(read("json", &["--all", "--sequence", "91", "--count", "40"]).await);
    assert_eq!(values(&resumed), (91..=100).collect::<Vec<u64>>());

    let table = read("table", &["--sequence", "50", "--count", "3"]).await;
    assert!(
        table.starts_with("ReadRange TREND_LOG:2  LOG_BUFFER  count=3  flags=none  first-seq=50\n"),
        "{table}"
    );
    let table = read("table", &["--all", "--count", "40"]).await;
    assert!(table.contains("pages=3  next: --sequence 101"), "{table}");

    // Pacing changes the timing, not the records.
    let paced = run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        "json",
        "--min-interval-ms",
        "5",
        "read-range",
        &target,
        "trend-log:2",
        "--all",
        "--count",
        "40",
    ])
    .await;
    assert!(paced.status.success());
    assert_eq!(
        values(&json(String::from_utf8(paced.stdout).unwrap())).len(),
        100
    );

    for (flags, diagnostic) in [
        (&["--count", "5"][..], "--count needs"),
        (&["--all", "--count", "-5"][..], "--all reads forward"),
        (
            &["--position", "1", "--sequence", "2"][..],
            "cannot be used with",
        ),
        (&["--time", "2026-13-01T00:00"][..], "--time"),
    ] {
        let mut args = vec![
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "read-range",
            &target,
            "trend-log:2",
        ];
        args.extend_from_slice(flags);
        let output = run(args).await;
        assert!(!output.status.success(), "{flags:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(diagnostic), "{flags:?}: {stderr}");
    }

    server.stop().await.unwrap();
}

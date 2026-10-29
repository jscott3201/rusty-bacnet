//! `bacnet read-range` decodes the records of a log buffer by the log object's
//! type (#1274), against a loopback server.
#[allow(dead_code)]
mod support;

use std::net::Ipv4Addr;

use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventLogRecord, BACnetLogMultipleRecord,
    BACnetLogRecord, BACnetPropertyStates, EventLogDatum, EventNotificationRequest, LogData,
    LogDatum, LogValue, NotificationParameters,
};
use bacnet_types::enums::{EventState, EventType, NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{
    BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, StatusFlags, Time,
};
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
        identity: bacnet_objects::DeviceIdentity::default().with_name("read-range"),
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

    // Event Log 2: notifications carrying their event values (#2073).
    let mut valued = EventLogObject::new(2, "EL-2", 10).unwrap();
    for (hour, notification) in [
        (
            8,
            EventNotificationRequest {
                event_values: Some(NotificationParameters::OutOfRange {
                    exceeding_value: 101.5,
                    status_flags: StatusFlags::IN_ALARM,
                    deadband: 1.0,
                    exceeded_limit: 100.0,
                }),
                ..alarm()
            },
        ),
        (
            9,
            EventNotificationRequest {
                event_type: EventType::CHANGE_OF_STATE,
                message_text: None,
                from_state: EventState::NORMAL,
                to_state: EventState::OFFNORMAL,
                event_values: Some(NotificationParameters::ChangeOfState {
                    new_state: BACnetPropertyStates::BinaryValue(1),
                    status_flags: StatusFlags::IN_ALARM | StatusFlags::OUT_OF_SERVICE,
                }),
                ..alarm()
            },
        ),
    ] {
        valued
            .add_record(BACnetEventLogRecord {
                date: DATE,
                time: time(hour),
                log_datum: EventLogDatum::Notification(notification),
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
        Box::new(valued),
        Box::new(multiple),
        Box::new(long),
        Box::new(event_state_log()),
        Box::new(status_flags_log()),
        Box::new(members_log()),
        Box::new(state_log(5, ObjectType::MULTI_STATE_VALUE)),
        Box::new(state_log(6, ObjectType::BINARY_VALUE)),
    ];
    let objects: Vec<Box<dyn BACnetObject>> = objects.into_iter().chain(state_objects()).collect();
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

/// A reference to `property` of `object_type` instance `instance`.
fn sampling(
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(oid(object_type, instance), property.to_raw())
}

/// Trend Log 3 samples Analog Input 1's Event_State: two enumerations and,
/// mismatched, a REAL (#2091).
fn event_state_log() -> TrendLogObject {
    let mut log = TrendLogObject::new(3, "TL-3", 10).unwrap();
    log.set_log_device_object_property(Some(sampling(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::EVENT_STATE,
    )))
    .unwrap();
    for (hour, log_datum) in [
        (8, LogDatum::EnumValue(0)),
        (9, LogDatum::EnumValue(3)),
        (10, LogDatum::RealValue(1.5)),
    ] {
        log.add_record(BACnetLogRecord {
            date: DATE,
            time: time(hour),
            log_datum,
            status_flags: None,
        })
        .unwrap();
    }
    log
}

/// Status flags as a logged bit string.
fn flags_datum(bits: u8) -> LogDatum {
    LogDatum::BitstringValue {
        unused_bits: 4,
        data: vec![bits << 4],
    }
}

/// Trend Log 4 samples Analog Input 1's Status_Flags.
fn status_flags_log() -> TrendLogObject {
    let mut log = TrendLogObject::new(4, "TL-4", 10).unwrap();
    log.set_log_device_object_property(Some(sampling(
        ObjectType::ANALOG_INPUT,
        1,
        PropertyIdentifier::STATUS_FLAGS,
    )))
    .unwrap();
    for (hour, bits) in [(8, 0b1000), (9, 0)] {
        log.add_record(BACnetLogRecord {
            date: DATE,
            time: time(hour),
            log_datum: flags_datum(bits),
            status_flags: None,
        })
        .unwrap();
    }
    log
}

/// Trend Log Multiple 2: Analog Input 1's Event_State, an empty member and
/// Binary Input 1's Present_Value, each named by its own reference.
fn members_log() -> TrendLogMultipleObject {
    let mut log = TrendLogMultipleObject::new(2, "TLM-2", 10).unwrap();
    for reference in [
        sampling(ObjectType::ANALOG_INPUT, 1, PropertyIdentifier::EVENT_STATE),
        sampling(
            ObjectType::ANALOG_INPUT,
            ObjectIdentifier::WILDCARD_INSTANCE,
            PropertyIdentifier::PRESENT_VALUE,
        ),
        sampling(
            ObjectType::BINARY_INPUT,
            1,
            PropertyIdentifier::PRESENT_VALUE,
        ),
    ] {
        log.add_property_reference(reference).unwrap();
    }
    log.add_record(BACnetLogMultipleRecord {
        date: DATE,
        time: time(8),
        log_data: LogData::Values(vec![
            LogValue::EnumValue(2),
            LogValue::EnumValue(2),
            LogValue::EnumValue(1),
        ]),
    })
    .unwrap();
    log
}

/// Multi-State Value 1, its State_Text `Off`, `Low` and `High`, and Binary
/// Value 1, its texts `Running` and `Stopped` (#2122).
fn state_objects() -> Vec<Box<dyn BACnetObject>> {
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    let mut mode = MultiStateValueObject::new(1, "Fan Speed", 3).unwrap();
    mode.write_property(
        PropertyIdentifier::STATE_TEXT,
        None,
        PropertyValue::List(vec![text("Off"), text("Low"), text("High")]),
        None,
    )
    .unwrap();
    let mut fan = BinaryValueObject::new(1, "Fan").unwrap();
    for (property, value) in [
        (PropertyIdentifier::ACTIVE_TEXT, "Running"),
        (PropertyIdentifier::INACTIVE_TEXT, "Stopped"),
    ] {
        fan.write_property(property, None, text(value), None)
            .unwrap();
    }
    vec![Box::new(mode), Box::new(fan)]
}

/// Trend Log `instance` samples the Present_Value of `object_type` 1: a
/// multi-state value's states 1 to 4 (4 past its State_Text), or a binary
/// value's active and inactive.
fn state_log(instance: u32, object_type: ObjectType) -> TrendLogObject {
    let mut log = TrendLogObject::new(instance, format!("TL-{instance}"), 10).unwrap();
    log.set_log_device_object_property(Some(sampling(
        object_type,
        1,
        PropertyIdentifier::PRESENT_VALUE,
    )))
    .unwrap();
    let data: Vec<LogDatum> = if object_type == ObjectType::BINARY_VALUE {
        vec![LogDatum::EnumValue(1), LogDatum::EnumValue(0)]
    } else {
        (1..=4).map(LogDatum::UnsignedValue).collect()
    };
    for (hour, log_datum) in (8..).zip(data) {
        log.add_record(BACnetLogRecord {
            date: DATE,
            time: time(hour),
            log_datum,
            status_flags: None,
        })
        .unwrap();
    }
    log
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
    assert!(
        table.starts_with(
            "ReadRange EVENT_LOG:1  LOG_BUFFER  count=2  flags=FIRST_ITEM,LAST_ITEM\n"
        )
    );
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

/// An Event Log's notifications show their event values for each event
/// type, in the table and as typed fields in JSON (#2073).
#[tokio::test]
async fn read_range_shows_event_values_per_event_type() {
    let mut server = server().await;
    let target = target(&server);

    let out_of_range = "ALARM OUT_OF_RANGE ANALOG_INPUT:3 NORMAL -> HIGH_LIMIT \"too hot\" \
                        {exceeding value 101.5, flags in-alarm, deadband 1, limit 100}";
    let change_of_state = "ALARM CHANGE_OF_STATE ANALOG_INPUT:3 NORMAL -> OFFNORMAL \
                           {state active, flags in-alarm | out-of-service}";
    let table = read_range(&target, "event-log:2", "table").await;
    for cell in [out_of_range, change_of_state] {
        assert!(table.contains(cell), "{cell}: {table}");
    }

    let events: serde_json::Value =
        serde_json::from_str(&read_range(&target, "event-log:2", "json").await).unwrap();
    assert_eq!(
        events["records"],
        json!([
            {
                "timestamp": "2026-10-03 08:00:00.00",
                "datum": out_of_range,
                "event_values": {
                    "element": "notification-parameters",
                    "text": "out-of-range: exceeding value 101.5, flags in-alarm, deadband 1, \
                             limit 100",
                    "fields": {
                        "event_type": {"value": 5, "name": "out-of-range"},
                        "exceeding_value": 101.5,
                        "status_flags": {"bits": "1000", "names": ["in-alarm"]},
                        "deadband": 1.0,
                        "exceeded_limit": 100.0,
                    },
                },
            },
            {
                "timestamp": "2026-10-03 09:00:00.00",
                "datum": change_of_state,
                "event_values": {
                    "element": "notification-parameters",
                    "text": "change-of-state: state active, flags in-alarm | out-of-service",
                    "fields": {
                        "event_type": {"value": 1, "name": "change-of-state"},
                        "new_state": {"element": "property-state", "text": "active"},
                        "status_flags": {
                            "bits": "1001",
                            "names": ["in-alarm", "out-of-service"],
                        },
                    },
                },
            },
        ])
    );

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

/// A Trend Log's enumerations and bits are named by the property its
/// Log_DeviceObjectProperty names, in the table and in JSON; a value the
/// property doesn't name, a log with no reference and an empty member
/// print as logged (#2091).
#[tokio::test]
async fn read_range_names_logged_values_by_the_monitored_property() {
    let mut server = server().await;
    let target = target(&server);
    let json = |text: String| serde_json::from_str::<serde_json::Value>(&text).unwrap();

    let event_state = json(read_range(&target, "trend-log:3", "json").await);
    assert_eq!(
        event_state["records"],
        json!([
            { "timestamp": "2026-10-03 08:00:00.00", "datum": "normal" },
            { "timestamp": "2026-10-03 09:00:00.00", "datum": "high-limit" },
            { "timestamp": "2026-10-03 10:00:00.00", "datum": "1.5" },
        ])
    );
    assert_eq!(event_state["item_count"], json!(3));
    let table = read_range(&target, "trend-log:3", "table").await;
    for cell in ["normal", "high-limit"] {
        assert!(table.contains(cell), "{cell}: {table}");
    }
    assert!(!table.contains("enum "), "{table}");

    // --all reads the reference once, before the pages, and names every
    // page's records.
    let paged =
        json(read_range_with(&target, "trend-log:4", "json", &["--all", "--count", "1"]).await);
    assert_eq!(paged["pages"], json!(2), "{paged}");
    assert_eq!(
        paged["records"],
        json!([
            { "timestamp": "2026-10-03 08:00:00.00", "datum": "in-alarm" },
            { "timestamp": "2026-10-03 09:00:00.00", "datum": "none" },
        ])
    );

    // Binary Input 1 doesn't exist, so the read of its state texts is
    // refused and its member keeps `active` (#2122).
    let members = json(read_range(&target, "trend-log-multiple:2", "json").await);
    assert_eq!(
        members["records"],
        json!([{ "timestamp": "2026-10-03 08:00:00.00", "datum": "[offnormal, enum 2, active]" }])
    );

    // Trend Log 2 has no reference: its values print as logged.
    let unnamed = json(
        read_range_with(
            &target,
            "trend-log:2",
            "json",
            &["--position", "1", "--count", "1"],
        )
        .await,
    );
    assert_eq!(unnamed["records"][0]["datum"], json!("1"));

    server.stop().await.unwrap();
}

/// A log of a multi-state value's Present_Value shows each state with its
/// State_Text entry, and one past the array as its number; a binary
/// value's log shows its Active_Text and Inactive_Text (#2122).
#[tokio::test]
async fn read_range_names_logged_states_with_the_objects_state_texts() {
    let mut server = server().await;
    let target = target(&server);
    let datums = |text: String| -> Vec<String> {
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["datum"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        datums(read_range(&target, "trend-log:5", "json").await),
        ["1 (Off)", "2 (Low)", "3 (High)", "4"]
    );
    let paged = read_range_with(&target, "trend-log:6", "json", &["--all", "--count", "1"]).await;
    assert_eq!(datums(paged), ["active (Running)", "inactive (Stopped)"]);
    let table = read_range(&target, "trend-log:5", "table").await;
    for cell in ["1 (Off)", "2 (Low)", "3 (High)"] {
        assert!(table.contains(cell), "{cell}: {table}");
    }
    server.stop().await.unwrap();
}

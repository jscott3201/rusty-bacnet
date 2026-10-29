//! Real CLI processes reading a device's alarms decoded (#2077):
//! `alarms` (GetEventInformation, paged), `alarm-summary` and
//! `enrollment-summary`, as tables and as JSON.
#[allow(dead_code)]
mod support;

use std::net::Ipv4Addr;

use bacnet_objects::notification_class::NotificationClass;
use bacnet_server::server::{BACnetServer, GetEventInformationBudget};
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{EventState, NotifyType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};
use support::alarm::FixedAlarm;
use support::lab::{lab_database, lab_identity};

/// The lab device with three fixed alarms (one an event, not an alarm)
/// and AI:1's fault cleared, answering GetEventInformation `page` objects
/// at a time.
async fn server(page: usize) -> (BACnetServer<BipTransport>, String) {
    let mut db = lab_database(1234);
    db.get_mut(&ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap())
        .unwrap()
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
            None,
        )
        .unwrap();
    // AI:2 reports to class 5, so the enrollment filters tell it apart.
    db.get_mut(&ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap())
        .unwrap()
        .write_property(
            PropertyIdentifier::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(5),
            None,
        )
        .unwrap();
    // The other intrinsic reporting objects name class 0.
    db.add(Box::new(NotificationClass::new(0, "Default").unwrap()))
        .unwrap();
    let mut class = NotificationClass::new(5, "Plant").unwrap();
    class.priority = [3, 5, 200];
    db.add(Box::new(class)).unwrap();
    for alarm in [
        FixedAlarm::new(1, EventState::HIGH_LIMIT, 0b011, NotifyType::ALARM),
        FixedAlarm::new(2, EventState::FAULT, 0b111, NotifyType::ALARM),
        FixedAlarm::new(3, EventState::NORMAL, 0b110, NotifyType::EVENT),
    ] {
        db.add(Box::new(alarm)).unwrap();
    }
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(db)
        .get_event_information_budget(GetEventInformationBudget {
            max_returned_summaries: page,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let target = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    (server, target)
}

async fn cli(target: &str, format: &str, args: &[&str]) -> String {
    let mut all = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        format,
    ];
    all.push(args[0]);
    all.push(target);
    all.extend(&args[1..]);
    let output = support::run(all).await;
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

async fn json(target: &str, args: &[&str]) -> Value {
    serde_json::from_str(&cli(target, "json", args).await).unwrap()
}

fn object(instance: u32) -> Value {
    json!({"type": "analog-value", "type_number": 2, "instance": instance})
}

/// The summary `alarms` prints for FixedAlarm `instance`.
fn summary(instance: u32, state: Value, acked: Value, notify: Value) -> Value {
    json!({
        "object": object(instance),
        "event_state": state,
        "acknowledged_transitions": acked,
        "event_enable": {"bits": "111", "names": ["to-offnormal", "to-fault", "to-normal"]},
        "notify_type": notify,
        "event_timestamps": {
            "to_offnormal": {"element": "time-stamp", "text": "2026-10-25 Sun 14:30:05"},
            "to_fault": {"element": "time-stamp", "text": "seq 42"},
            "to_normal": {"element": "time-stamp", "text": "07:00"}
        },
        "event_priorities": {"to_offnormal": 3, "to_fault": 5, "to_normal": 200}
    })
}

fn all_summaries() -> Vec<Value> {
    let alarm = json!({"value": 0, "name": "alarm"});
    vec![
        summary(
            1,
            json!({"value": 3, "name": "high-limit"}),
            json!({"bits": "011", "names": ["to-fault", "to-normal"]}),
            alarm.clone(),
        ),
        summary(
            2,
            json!({"value": 1, "name": "fault"}),
            json!({"bits": "111", "names": ["to-offnormal", "to-fault", "to-normal"]}),
            alarm,
        ),
        // Normal, but its to-normal transition is unacknowledged.
        summary(
            3,
            json!({"value": 0, "name": "normal"}),
            json!({"bits": "110", "names": ["to-offnormal", "to-fault"]}),
            json!({"value": 1, "name": "event"}),
        ),
    ]
}

/// A CLI run that must fail, and its stderr.
async fn refused(target: &str, args: &[&str]) -> String {
    let mut all = vec!["--interface", "127.0.0.1", "--port", "0", args[0], target];
    all.extend(&args[1..]);
    let output = support::run(all).await;
    assert!(!output.status.success(), "{args:?} succeeded");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn alarms_are_decoded_in_json_and_the_table() {
    let (mut server, target) = server(256).await;
    assert_eq!(
        json(&target, &["alarms"]).await,
        json!({"summaries": all_summaries(), "more_events": false})
    );
    let table = cli(&target, "table", &["alarms"]).await;
    let header: Vec<&str> = table
        .lines()
        .find(|line| line.starts_with("| Object"))
        .unwrap_or_else(|| panic!("no header: {table}"))
        .trim_matches('|')
        .split(" | ")
        .map(str::trim)
        .collect();
    assert_eq!(
        header,
        [
            "Object",
            "State",
            "Acked",
            "Enabled",
            "Notify",
            "To-offnormal",
            "To-fault",
            "To-normal",
            "Priorities"
        ],
        "{table}"
    );
    let rows: Vec<Vec<&str>> = table
        .lines()
        .filter(|line| line.starts_with("| analog-value"))
        .map(|line| line.trim_matches('|').split(" | ").map(str::trim).collect())
        .collect();
    let times = ["2026-10-25 Sun 14:30:05", "seq 42", "07:00", "3/5/200"];
    let all = "to-offnormal, to-fault, to-normal";
    let expected: Vec<Vec<&str>> = [
        [
            "analog-value 1",
            "high-limit",
            "to-fault, to-normal",
            all,
            "alarm",
        ],
        ["analog-value 2", "fault", all, all, "alarm"],
        [
            "analog-value 3",
            "normal",
            "to-offnormal, to-fault",
            all,
            "event",
        ],
    ]
    .into_iter()
    .map(|start| start.into_iter().chain(times).collect())
    .collect();
    assert_eq!(rows, expected, "{table}");
    assert!(
        table
            .lines()
            .any(|line| line == "3 object(s) with an active event or an unacknowledged transition"),
        "{table}"
    );
    assert!(!table.contains("raw"), "{table}");
    server.stop().await.unwrap();
}

#[tokio::test]
async fn alarms_follow_more_events_to_the_end_or_the_cap() {
    // One object an answer: three answers, each after the last object.
    let (mut server, target) = server(1).await;
    assert_eq!(
        json(&target, &["alarms"]).await,
        json!({"summaries": all_summaries(), "more_events": false})
    );
    assert_eq!(
        json(&target, &["alarms", "--max", "2"]).await,
        json!({"summaries": all_summaries()[..2], "more_events": true})
    );
    let table = cli(&target, "table", &["alarms", "--max", "2"]).await;
    assert!(
        table.contains("2 object(s) with an active event or an unacknowledged transition"),
        "{table}"
    );
    assert!(
        table.contains("The device holds more; stopped at --max 2."),
        "{table}"
    );
    refused(&target, &["alarms", "--max", "0"]).await;
    server.stop().await.unwrap();
}

#[tokio::test]
async fn alarm_summary_lists_the_alarms_named() {
    let (mut server, target) = server(256).await;
    // The device lists them in its own order.
    let mut answer = json(&target, &["alarm-summary"]).await;
    answer
        .as_array_mut()
        .unwrap()
        .sort_by_key(|entry| entry["object"]["instance"].as_u64());
    assert_eq!(
        answer,
        json!([
            {
                "object": object(1),
                "alarm_state": {"value": 3, "name": "high-limit"},
                "acknowledged_transitions": {"bits": "011", "names": ["to-fault", "to-normal"]}
            },
            {
                "object": object(2),
                "alarm_state": {"value": 1, "name": "fault"},
                "acknowledged_transitions": {
                    "bits": "111",
                    "names": ["to-offnormal", "to-fault", "to-normal"]
                }
            }
        ])
    );
    let table = cli(&target, "table", &["alarm-summary"]).await;
    let mut lines: Vec<&str> = table.lines().collect();
    // The two rows in the device's order.
    let rows = [
        "| analog-value 1 | high-limit | to-fault, to-normal               |",
        "| analog-value 2 | fault      | to-offnormal, to-fault, to-normal |",
    ];
    if lines.get(3) == Some(&rows[1]) {
        lines.swap(3, 5);
    }
    assert_eq!(
        lines,
        [
            "+----------------+------------+-----------------------------------+",
            "| Object         | State      | Acked                             |",
            "+=================================================================+",
            rows[0],
            "|----------------+------------+-----------------------------------|",
            rows[1],
            "+----------------+------------+-----------------------------------+",
            "2 object(s) in alarm",
        ],
        "{table}"
    );
    server.stop().await.unwrap();
}

/// The objects of an `enrollment-summary` answer as (type, instance),
/// sorted: the device lists them in its own order.
fn enrolled(answer: &Value) -> Vec<(String, u64)> {
    let mut objects: Vec<(String, u64)> = answer
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["object"]["type"].as_str().unwrap().to_string(),
                entry["object"]["instance"].as_u64().unwrap(),
            )
        })
        .collect();
    objects.sort();
    objects
}

#[tokio::test]
async fn enrollment_summary_names_entries_and_sends_the_filters() {
    let (mut server, target) = server(256).await;
    let answer = json(
        &target,
        &["enrollment-summary", "--event-type", "out-of-range"],
    )
    .await;
    assert_eq!(
        enrolled(&answer),
        [
            ("analog-input".to_string(), 1),
            ("analog-input".to_string(), 2),
            ("analog-output".to_string(), 1)
        ]
    );
    let entry = |instance: u64| {
        answer
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| {
                entry["object"]["type"] == "analog-input" && entry["object"]["instance"] == instance
            })
            .cloned()
            .unwrap()
    };
    assert_eq!(
        entry(1),
        json!({
            "object": {"type": "analog-input", "type_number": 0, "instance": 1},
            "event_type": {"value": 5, "name": "out-of-range"},
            "event_state": {"value": 0, "name": "normal"},
            "priority": 255,
            "notification_class": 0
        })
    );
    // AI:2 reports to class 5, whose to-normal priority is 200.
    assert_eq!(entry(2)["priority"], json!(200));
    assert_eq!(entry(2)["notification_class"], json!(5));

    // Every lab enrollment is normal and fully acknowledged; all but AI:2
    // are in class 0 at priority 255.
    let all = enrolled(&json(&target, &["enrollment-summary"]).await);
    assert_eq!(all.len(), 7, "{all:?}");
    let ai2 = vec![("analog-input".to_string(), 2)];
    let others: Vec<_> = all.iter().filter(|o| **o != ai2[0]).cloned().collect();
    let none = Vec::new();
    for (flags, expected) in [
        (&["--acked", "acked"][..], &all),
        (&["--acked", "not-acked"], &none),
        (&["--state", "normal"], &all),
        (&["--state", "active"], &none),
        (&["--priority", "255"], &others),
        (&["--priority", "200"], &ai2),
        (&["--priority", "100-210"], &ai2),
        (&["--priority", "0-199"], &none),
        (&["--class", "0"], &others),
        (&["--class", "5"], &ai2),
        (&["--class", "5", "--priority", "255"], &none),
    ] {
        let mut args = vec!["enrollment-summary"];
        args.extend(flags);
        assert_eq!(
            &enrolled(&json(&target, &args).await),
            expected,
            "{flags:?}"
        );
    }
    // Names in either spelling.
    let states = json(
        &target,
        &["enrollment-summary", "--event-type", "CHANGE_OF_STATE"],
    )
    .await;
    assert_eq!(enrolled(&states).len(), 3, "{states}");

    let table = cli(&target, "table", &["enrollment-summary", "--class", "7"]).await;
    assert_eq!(table.trim_end(), "No event enrollments match.");
    let table = cli(&target, "table", &["enrollment-summary", "--class", "5"]).await;
    assert!(
        table
            .contains("| analog-input 2 | out-of-range | normal | 200      | 5                  |"),
        "{table}"
    );
    let table = cli(
        &target,
        "table",
        &["enrollment-summary", "--event-type", "buffer-ready"],
    )
    .await;
    assert!(
        table.contains("| trend-log 1 | buffer-ready | normal | 255      | 0                  |"),
        "{table}"
    );
    assert!(table.contains("1 enrollment(s)"), "{table}");

    let stderr = refused(&target, &["enrollment-summary", "--event-type", "bogus"]).await;
    assert!(stderr.contains("unknown event type 'bogus'"), "{stderr}");
    let stderr = refused(&target, &["enrollment-summary", "--priority", "9-3"]).await;
    assert!(stderr.contains("runs backward"), "{stderr}");
    server.stop().await.unwrap();
}

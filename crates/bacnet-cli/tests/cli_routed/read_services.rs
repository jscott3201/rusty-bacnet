//! Read-only routed services keep their local JSON shape.
use super::*;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{EventState, NotifyType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use crate::support::alarm::FixedAlarm;

/// The lab objects refer to Notification Class 0 by default.
pub(super) fn database() -> ObjectDatabase {
    let mut database = lab_database(INSTANCE);
    database
        .add(Box::new(
            NotificationClass::new(0, "Default alarms").unwrap(),
        ))
        .unwrap();
    // This service fixture needs a stable empty event list, rather than the
    // shared lab's intentionally faulted AI1 and its live event timestamp.
    database
        .get_mut(&ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap())
        .unwrap()
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
            None,
        )
        .unwrap();
    let log = ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap();
    database.remove(&log).unwrap().unwrap();
    let mut trend = TrendLogObject::new(1, "Zone Temp Log", 100).unwrap();
    for sequence in 1..=3u8 {
        trend
            .add_record(BACnetLogRecord {
                date: Date {
                    year: 126,
                    month: 10,
                    day: 3,
                    day_of_week: 6,
                },
                time: Time {
                    hour: 8,
                    minute: 0,
                    second: sequence - 1,
                    hundredths: 0,
                },
                log_datum: LogDatum::UnsignedValue(u64::from(sequence)),
                status_flags: None,
            })
            .unwrap();
    }
    database.add(Box::new(trend)).unwrap();
    database
}

async fn query(
    port: Option<u16>,
    target: &str,
    service: &str,
    arguments: &[&str],
) -> std::process::Output {
    let mut cmd = command();
    cmd.args(["--interface", "127.0.0.1", "--port", "0", "--json"]);
    if let Some(port) = port {
        cmd.args(["--foreign-bbmd", &format!("127.0.0.1:{port}")]);
    }
    cmd.arg(service).arg(target).args(arguments);
    Process::spawn(&mut cmd).output().await
}

fn json_output(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

async fn direct_site() -> (BACnetServer<BipTransport>, String) {
    direct_site_with(database()).await
}

async fn direct_site_with(database: ObjectDatabase) -> (BACnetServer<BipTransport>, String) {
    let direct = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(INSTANCE))
        .database(database)
        .build()
        .await
        .unwrap();
    let mac = direct.local_mac();
    let address = format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]));
    (direct, address)
}

/// The service fixture with one active alarm, whose timestamps are fixed
/// so the direct and routed answers can match.
fn alarm_database() -> ObjectDatabase {
    let mut database = database();
    let mut alarm = FixedAlarm::new(1, EventState::HIGH_LIMIT, 0b011, NotifyType::ALARM);
    alarm.class = 0;
    database.add(Box::new(alarm)).unwrap();
    database
}

#[tokio::test]
async fn routed_alarm_commands_match_the_direct_response_for_both_target_forms() {
    let (mut router, mut remote, port) = site_with_database(alarm_database()).await;
    let (mut direct, address) = direct_site_with(alarm_database()).await;
    let alarm = json!({"type": "analog-value", "type_number": 2, "instance": 1});
    for (service, arguments) in [
        ("alarms", &[][..]),
        ("alarm-summary", &[]),
        ("enrollment-summary", &["--event-type", "buffer-ready"]),
    ] {
        let expected = json_output(&query(None, &address, service, arguments).await);
        match service {
            "alarms" => {
                assert_eq!(expected["more_events"], json!(false));
                let summaries = expected["summaries"].as_array().unwrap();
                assert_eq!(summaries.len(), 1, "{expected}");
                assert_eq!(summaries[0]["object"], alarm);
                assert_eq!(summaries[0]["event_state"]["name"], "high-limit");
                assert_eq!(
                    summaries[0]["event_timestamps"]["to_fault"]["text"],
                    "seq 42"
                );
            }
            "alarm-summary" => {
                assert_eq!(expected.as_array().map(Vec::len), Some(1), "{expected}");
                assert_eq!(expected[0]["object"], alarm);
            }
            _ => assert_eq!(
                expected[0]["object"],
                json!({"type": "trend-log", "type_number": 20, "instance": 1}),
                "{expected}"
            ),
        }
        for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
            assert_eq!(
                json_output(&query(Some(port), &target, service, arguments).await),
                expected,
                "{service} {target}"
            );
        }
    }
    direct.stop().await.unwrap();
    remote.stop().await.unwrap();
    router.stop().await.unwrap();
}

/// Independently expected records and complete output schema for this fixture.
fn expected_range(first: u8, last: u8, paged: bool) -> Value {
    let mut expected = json!({
        "object": "TREND_LOG:1",
        "property": "LOG_BUFFER",
        "item_count": last - first + 1,
        "result_flags": { "first_item": !paged && first == 1, "last_item": last == 3, "more_items": false },
        "first_sequence_number": first,
        "violations": [],
        "records": (first..=last).map(|sequence| json!({
            "timestamp": format!("2026-10-03 08:00:0{}.00", sequence - 1),
            "datum": sequence.to_string(),
        })).collect::<Vec<_>>(),
    });
    if paged {
        expected["pages"] = json!(last - first + 1);
        expected["next"] = json!("--sequence 4");
        expected["gaps"] = json!([]);
    }
    expected
}

#[tokio::test]
async fn routed_read_range_keeps_strict_and_lenient_page_output() {
    let (mut router, mut remote, port) = site().await;
    let (mut direct, address) = direct_site().await;
    for flags in [
        vec!["trend-log:1", "--sequence", "1", "--count", "2"],
        vec!["trend-log:1", "--sequence", "1", "--count", "2", "--strict"],
    ] {
        let expected = expected_range(1, 2, false);
        assert_eq!(
            json_output(&query(None, &address, "read-range", &flags).await),
            expected
        );
        for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
            assert_eq!(
                json_output(&query(Some(port), &target, "read-range", &flags).await),
                expected,
                "{target}"
            );
        }
    }
    direct.stop().await.unwrap();
    remote.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn routed_read_range_all_reads_metadata_and_each_page_then_resumes() {
    let (mut router, mut remote, port) = site().await;
    let (mut direct, address) = direct_site().await;
    for (flags, first) in [
        (vec!["trend-log:1", "--all", "--count", "1"], 1),
        (
            vec!["trend-log:1", "--all", "--sequence", "2", "--count", "1"],
            2,
        ),
    ] {
        let expected = expected_range(first, 3, true);
        assert_eq!(
            json_output(&query(None, &address, "read-range", &flags).await),
            expected
        );
        for target in [format!("{DNET}:{INSTANCE}"), format!("{DNET}:0x05")] {
            assert_eq!(
                json_output(&query(Some(port), &target, "read-range", &flags).await),
                expected,
                "{target}"
            );
        }
    }
    direct.stop().await.unwrap();
    remote.stop().await.unwrap();
    router.stop().await.unwrap();
}

#[tokio::test]
async fn routed_read_services_report_malformed_and_unresolved_targets() {
    for (service, arguments) in [
        ("alarms", &[][..]),
        ("read-range", &["trend-log:1"][..]),
        ("read-range", &["trend-log:1", "--all"][..]),
    ] {
        for (target, diagnostic) in [
            ("2:0x", "invalid MAC in routed target"),
            ("1235", "Device 1235 not found"),
        ] {
            failure(&query(None, target, service, arguments).await, diagnostic);
        }
    }
}

//! `bacnet_read_log_records` against an Event Log and an Audit Log in the
//! lab (#2103): paging across ReadRange answers cut short by the device
//! (MORE_ITEMS) and across calls, the per-type summary against a
//! brute-force count, and device text in envelopes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_objects::event_log::EventLogObject;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditNotification,
    BACnetEventLogRecord, BACnetRecipient, EventLogDatum, EventNotificationRequest,
    NotificationParameters,
};
use bacnet_types::enums::{
    AuditOperation, ErrorClass, ErrorCode, EventState, EventType, NotifyType, ObjectType,
    PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, StatusFlags};
use serde_json::{Value, json};

use super::site::{failed, ok, trend_time};
use super::{LAB, Lab, hostile};
use crate::Policy;

/// Records in event-log:1.
const EVENT_RECORDS: u32 = 300;

/// Records in audit-log:1.
const AUDIT_RECORDS: u32 = 250;

/// The record holding the attacker's text, in each log.
const HOSTILE_AT: u32 = 3;

fn object(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Event-log:1's record `i`: every 50th a log status change and every
/// 50th (offset) a clock change; the rest notifications, out-of-range ones
/// (with event values) and change-of-state ones in turn, entering normal,
/// offnormal and high-limit in turn, every tenth an acknowledgment.
fn event(i: u32) -> EventLogDatum {
    match i % 50 {
        49 => return EventLogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
        25 => return EventLogDatum::TimeChange(1.5),
        _ => {}
    }
    let out_of_range = i.is_multiple_of(2);
    EventLogDatum::Notification(EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: object(ObjectType::DEVICE, LAB),
        event_object_identifier: object(ObjectType::ANALOG_INPUT, 1),
        timestamp: BACnetTimeStamp::SequenceNumber(u16::try_from(i).unwrap()),
        notification_class: 0,
        priority: 100,
        event_type: if out_of_range {
            EventType::OUT_OF_RANGE
        } else {
            EventType::CHANGE_OF_STATE
        },
        message_text: Some(if i == HOSTILE_AT {
            hostile()
        } else {
            format!("event {i}")
        }),
        notify_type: if i % 10 == 7 {
            NotifyType::ACK_NOTIFICATION
        } else {
            NotifyType::ALARM
        },
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: [
            EventState::NORMAL,
            EventState::OFFNORMAL,
            EventState::HIGH_LIMIT,
        ][usize::try_from(i % 3).unwrap()],
        event_values: out_of_range.then_some(NotificationParameters::OutOfRange {
            exceeding_value: f32::from(u16::try_from(i).unwrap()),
            status_flags: StatusFlags::IN_ALARM,
            deadband: 1.0,
            exceeded_limit: 100.0,
        }),
    })
}

/// The operations of audit-log:1's notifications, in turn.
const OPERATIONS: [AuditOperation; 4] = [
    AuditOperation::WRITE,
    AuditOperation::CREATE,
    AuditOperation::DELETE,
    AuditOperation::DEVICE_RESET,
];

/// Audit-log:1's record `i`: every 60th a log status change, the rest
/// audit notifications by [`OPERATIONS`] in turn, every seventh failed.
fn audit(i: u32) -> BACnetAuditLogDatum {
    if i % 60 == 59 {
        return BACnetAuditLogDatum::LogStatus(LogStatus::LOG_DISABLED);
    }
    BACnetAuditLogDatum::AuditNotification(BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(object(ObjectType::DEVICE, 1)),
        source_object: None,
        operation: OPERATIONS[usize::try_from(i % 4).unwrap()],
        source_comment: (i == HOSTILE_AT).then(hostile),
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(object(ObjectType::DEVICE, LAB)),
        target_object: Some(object(ObjectType::ANALOG_VALUE, 1)),
        target_property: Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }),
        target_priority: Some(8),
        // REAL 21.5, application tagged.
        target_value: Some(vec![0x44, 0x41, 0xAC, 0x00, 0x00]),
        current_value: None,
        result: i
            .is_multiple_of(7)
            .then_some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    })
}

/// An Audit Log's storage, in memory.
#[derive(Default)]
struct Memory(Mutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for Memory {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

impl Lab {
    /// Add event-log:1 and audit-log:1 to the lab device.
    pub(crate) async fn add_logs(&self) {
        let mut events = EventLogObject::new(1, "Alarm History", EVENT_RECORDS + 10).unwrap();
        for i in 0..EVENT_RECORDS {
            let (date, time) = trend_time(i);
            events
                .add_record(BACnetEventLogRecord {
                    date,
                    time,
                    log_datum: event(i),
                })
                .unwrap();
        }
        let mut audits = AuditLogObject::new(
            1,
            "Audit Trail",
            AUDIT_RECORDS + 10,
            Arc::new(Memory::default()),
        )
        .unwrap();
        for i in 0..AUDIT_RECORDS {
            audits
                .add_record(BACnetAuditLogRecord {
                    timestamp: trend_time(i),
                    datum: audit(i),
                })
                .unwrap();
        }
        let mut db = self.server.database().write().await;
        db.add(Box::new(events)).unwrap();
        db.add(Box::new(audits)).unwrap();
    }
}

/// Every record of `object`, read `max_records` a call; the records and
/// the number of calls.
async fn read_all(lab: &mut Lab, object: &str, max_records: u32) -> (Vec<Value>, u32, u64) {
    let args = json!({ "device": LAB, "object": object, "max_records": max_records });
    let (mut records, mut calls, mut more_items) = (Vec::new(), 0, 0);
    let mut cursor: Option<String> = None;
    loop {
        let mut call = args.clone();
        if let Some(cursor) = &cursor {
            call["cursor"] = json!(cursor);
        }
        let page = lab.agent.call("bacnet_read_log_records", call).await;
        calls += 1;
        let body = ok(&page).clone();
        more_items += body["more_items"].as_u64().unwrap();
        records.extend(body["records"].as_array().unwrap().iter().cloned());
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => {
                assert_eq!(body["done"], json!(true), "{body}");
                break;
            }
        }
        assert!(calls < 10, "the paging ends");
    }
    (records, calls, more_items)
}

/// The record holding the attacker's text keeps it in an envelope: no
/// hidden character, the URL defanged, capped.
fn assert_enveloped(text: &Value) {
    let shown = text["untrusted"]
        .as_str()
        .unwrap_or_else(|| panic!("{text}"));
    assert!(shown.contains("hxxps://evil.example/x"), "{shown}");
    assert!(
        !shown.contains(['\u{202E}', '\u{2028}', '\u{E0049}']),
        "{shown}"
    );
    assert_eq!(text["truncated"], json!(true), "{text}");
    assert!(text["source"].as_str().unwrap().ends_with("log-buffer"));
}

/// Acceptance: an Event Log pages across ReadRange answers the device cut
/// short and across calls, every record once and in order; notifications
/// come decoded with their event values, the message in an envelope; and
/// the summary counts states, event types and notify types as a brute
/// force does.
#[tokio::test(start_paused = true)]
async fn an_event_log_pages_across_answers_and_calls() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.add_logs().await;
    lab.agent.initialize().await;
    let (records, calls, more_items) = read_all(&mut lab, "event-log:1", 120).await;
    // Three calls at least: more when a call's structured content fills up
    // before its 120 records, as a notification's fields are many.
    assert!(calls >= 3, "{calls}");
    assert!(more_items >= 1, "answers cut short by the device");
    let seqs: Vec<u64> = records.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=u64::from(EVENT_RECORDS)).collect::<Vec<_>>());
    let first = &records[0]["notification"];
    assert_eq!(records[0]["kind"], json!("notification"));
    assert_eq!(first["event_type"]["name"], json!("out-of-range"));
    assert_eq!(first["to_state"]["name"], json!("normal"));
    assert_eq!(first["message_text"]["untrusted"], json!("event 0"));
    assert_eq!(
        first["event_values"]["fields"]["exceeding_value"],
        json!(0.0)
    );
    assert!(
        first["event_values"]["text"]["untrusted"].is_string(),
        "{first}"
    );
    assert_eq!(records[1]["flagged"], json!(true), "into offnormal");
    assert_eq!(records[25]["kind"], json!("time-change"));
    assert_eq!(
        records[49]["log_status"]["names"],
        json!(["log-interrupted"])
    );
    let ack = &records[7]["notification"];
    assert_eq!(ack["notify_type"]["name"], json!("ack-notification"));
    assert_eq!(ack["from_state"], Value::Null);
    assert_enveloped(
        &records[usize::try_from(HOSTILE_AT).unwrap()]["notification"]["message_text"],
    );

    let summary = lab
        .agent
        .call(
            "bacnet_read_log_records",
            json!({ "device": LAB, "object": "event-log:1", "summary": true }),
        )
        .await;
    let body = ok(&summary);
    assert_eq!(body["records"], json!([]));
    assert!(body["pages"].as_u64().unwrap() >= 3, "{body}");
    let mut states: BTreeMap<String, u64> = BTreeMap::new();
    let mut types: BTreeMap<String, u64> = BTreeMap::new();
    let (mut notifications, mut flagged) = (0, 0);
    for i in 0..EVENT_RECORDS {
        match event(i) {
            EventLogDatum::Notification(n) => {
                notifications += 1;
                let state = ["normal", "offnormal", "high-limit"][usize::try_from(i % 3).unwrap()];
                *states.entry(state.into()).or_default() += 1;
                let kind = if i % 2 == 0 {
                    "out-of-range"
                } else {
                    "change-of-state"
                };
                *types.entry(kind.into()).or_default() += 1;
                flagged += u64::from(
                    n.notify_type != NotifyType::ACK_NOTIFICATION
                        && n.to_state != EventState::NORMAL,
                );
            }
            EventLogDatum::LogStatus(_) => flagged += 1,
            EventLogDatum::TimeChange(_) => {}
        }
    }
    let summary = &body["summary"];
    assert_eq!(summary["records"], json!(EVENT_RECORDS));
    assert_eq!(summary["notifications"], json!(notifications));
    assert_eq!(summary["log_status"], json!(EVENT_RECORDS / 50));
    assert_eq!(summary["time_changes"], json!(EVENT_RECORDS / 50));
    assert_eq!(summary["flagged"], json!(flagged));
    assert_eq!(summary["by_to_state"], json!(states));
    assert_eq!(summary["by_event_type"], json!(types));
    assert_eq!(
        summary["by_notify_type"]["ack-notification"],
        json!(EVENT_RECORDS / 10)
    );
    assert_eq!(summary["first_time"], json!("2026-10-25 00:00:00.00"));
    lab.stop().await;
}

/// Acceptance: an Audit Log pages the same way; notifications come with
/// the operation named, the value written decoded and a failure's error;
/// the source comment arrives in an envelope; and the summary counts
/// operations and failures as a brute force does.
#[tokio::test(start_paused = true)]
async fn an_audit_log_pages_and_counts_operations() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.add_logs().await;
    lab.agent.initialize().await;
    let (records, calls, more_items) = read_all(&mut lab, "audit-log:1", 100).await;
    assert_eq!(calls, 3);
    assert!(more_items >= 1, "answers cut short by the device");
    let seqs: Vec<u64> = records.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=u64::from(AUDIT_RECORDS)).collect::<Vec<_>>());
    let first = &records[0]["audit"];
    assert_eq!(records[0]["kind"], json!("audit"));
    assert_eq!(first["operation"], json!({ "value": 1, "name": "write" }));
    assert_eq!(first["target_property"]["name"], json!("present-value"));
    assert_eq!(first["target_value"], json!(21.5));
    assert_eq!(
        first["result"]["error_code"]["name"],
        json!("write-access-denied")
    );
    assert_eq!(records[0]["flagged"], json!(true), "a failed operation");
    assert_eq!(records[1]["audit"]["result"], Value::Null);
    assert_eq!(records[59]["kind"], json!("log-status"));
    assert_enveloped(&records[usize::try_from(HOSTILE_AT).unwrap()]["audit"]["source_comment"]);

    let summary = lab
        .agent
        .call(
            "bacnet_read_log_records",
            json!({ "device": LAB, "object": "audit-log:1", "summary": true }),
        )
        .await;
    let summary = &ok(&summary)["summary"];
    let mut operations: BTreeMap<String, u64> = BTreeMap::new();
    let (mut audits, mut failed_ops) = (0, 0);
    for i in 0..AUDIT_RECORDS {
        if let BACnetAuditLogDatum::AuditNotification(n) = audit(i) {
            audits += 1;
            let name =
                ["write", "create", "delete", "device-reset"][usize::try_from(i % 4).unwrap()];
            *operations.entry(name.into()).or_default() += 1;
            failed_ops += u64::from(n.result.is_some());
        }
    }
    assert_eq!(summary["records"], json!(AUDIT_RECORDS));
    assert_eq!(summary["audits"], json!(audits));
    assert_eq!(summary["failed"], json!(failed_ops));
    assert_eq!(summary["by_operation"], json!(operations));
    assert_eq!(summary["log_status"], json!(AUDIT_RECORDS / 60));
    assert!(summary.get("by_to_state").is_none(), "{summary}");
    lab.stop().await;
}

/// The tool reads event and audit logs only; a trend log is the trend
/// tool's, and a window can start from a time.
#[tokio::test(start_paused = true)]
async fn other_objects_are_refused_and_windows_start_at_a_time() {
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.add_logs().await;
    lab.agent.initialize().await;
    for object in ["trend-log:1", "ai:1"] {
        let refused = lab
            .agent
            .call(
                "bacnet_read_log_records",
                json!({ "device": LAB, "object": object }),
            )
            .await;
        assert_eq!(failed(&refused)["field"], json!("object"), "{object}");
    }
    let window = lab
        .agent
        .call(
            "bacnet_read_log_records",
            json!({ "device": LAB, "object": "event-log:1",
                    "from": "2026-10-25T01:00", "to": "2026-10-25T01:09" }),
        )
        .await;
    let body = ok(&window);
    let times: Vec<&str> = body["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["time"].as_str().unwrap())
        .collect();
    assert_eq!(times.len(), 9, "{times:?}");
    assert_eq!(times[0], "2026-10-25 01:01:00.00");
    assert_eq!(body["done"], json!(true));
    lab.stop().await;
}

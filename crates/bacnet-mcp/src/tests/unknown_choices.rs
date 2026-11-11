//! Records under a tag their CHOICE doesn't assign (#2474), as a newer
//! device might hold them: a ReadRange of each log kind carries them, the
//! client keeps them as received, and the tools show each as an
//! `unknown-choice` element (its tag and octets) beside the records around
//! it, which still read.

use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum,
    EventNotificationRequest, LogData, LogDatum, LogValue, NotificationParameters, OpaqueChoice,
};
use bacnet_types::enums::{EventState, EventType, NotifyType, ObjectType};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};
use serde_json::{Value, json};

use super::site::{ok, trend_time};
use super::{LAB, Lab};
use crate::Policy;

fn opaque(tag: u8, constructed: bool, data: &[u8]) -> OpaqueChoice {
    OpaqueChoice {
        tag,
        constructed,
        data: data.to_vec(),
    }
}

/// An alarm from analog-input:1 whose event values are under tag 30.
fn later_alarm() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, LAB).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(9),
        notification_class: 0,
        priority: 100,
        event_type: EventType::from_raw(30),
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::OFFNORMAL,
        event_values: Some(NotificationParameters::Opaque {
            tag: 30,
            data: vec![0x09, 0x01],
        }),
    }
}

impl Lab {
    /// Add event-log:9, trend-log:9 and trend-log-multiple:9, each with an
    /// unknown record between two known ones.
    async fn add_unknown_choice_logs(&self) {
        let stamp = |i| trend_time(i);
        let mut events = EventLogObject::new(9, "Newer Events", 10).unwrap();
        for (i, log_datum) in [
            EventLogDatum::TimeChange(1.5),
            EventLogDatum::Opaque(opaque(3, true, &[0x09, 0x01])),
            EventLogDatum::Notification(later_alarm()),
        ]
        .into_iter()
        .enumerate()
        {
            let (date, time) = stamp(u32::try_from(i).unwrap());
            events
                .add_record(BACnetEventLogRecord {
                    date,
                    time,
                    log_datum,
                })
                .unwrap();
        }
        let mut trend = TrendLogObject::new(9, "Newer Trend", 10).unwrap();
        for (i, log_datum) in [
            LogDatum::RealValue(20.5),
            LogDatum::Opaque(opaque(11, false, &[0x01, 0x02])),
            LogDatum::RealValue(21.5),
        ]
        .into_iter()
        .enumerate()
        {
            let (date, time) = stamp(u32::try_from(i).unwrap());
            trend
                .add_record(BACnetLogRecord {
                    date,
                    time,
                    log_datum,
                    status_flags: None,
                })
                .unwrap();
        }
        let mut multiple = TrendLogMultipleObject::new(9, "Newer Multiple", 10).unwrap();
        for (i, log_data) in [
            LogData::Values(vec![
                LogValue::RealValue(1.0),
                LogValue::Opaque(opaque(9, true, &[])),
            ]),
            LogData::Opaque(opaque(3, false, &[0x07])),
        ]
        .into_iter()
        .enumerate()
        {
            let (date, time) = stamp(u32::try_from(i).unwrap());
            multiple
                .add_record(BACnetLogMultipleRecord {
                    date,
                    time,
                    log_data,
                })
                .unwrap();
        }
        let mut db = self.server.database().write().await;
        db.add(Box::new(events)).unwrap();
        db.add(Box::new(trend)).unwrap();
        db.add(Box::new(multiple)).unwrap();
    }
}

/// The records of `object` through `tool`, in one call.
async fn records(lab: &mut Lab, tool: &str, object: &str) -> Vec<Value> {
    let result = lab
        .agent
        .call(tool, json!({ "device": LAB, "object": object }))
        .await;
    let body = ok(&result);
    assert_eq!(body["done"], json!(true), "{body}");
    body["records"].as_array().unwrap().clone()
}

/// The `unknown-choice` element's fields, with its text taken out of the
/// envelope it comes in.
fn choice(value: &Value) -> (String, Value) {
    let text = value["text"]["untrusted"]
        .as_str()
        .or_else(|| value["text"].as_str())
        .unwrap_or_else(|| panic!("{value}"))
        .to_string();
    assert_eq!(value["element"], json!("unknown-choice"), "{value}");
    (text, value["fields"].clone())
}

#[tokio::test(start_paused = true)]
async fn records_of_an_unknown_kind_read_through_readrange_as_unknown_choices() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.add_unknown_choice_logs().await;
    lab.agent.initialize().await;

    // Event Log: the unknown record is flagged and shows its tag and body;
    // the notification after it reads, its event values by tag.
    let events = records(&mut lab, "bacnet_read_log_records", "event-log:9").await;
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0]["kind"], json!("time-change"));
    assert_eq!(events[1]["kind"], json!("unknown-choice"));
    assert_eq!(events[1]["flagged"], json!(true));
    let (text, fields) = choice(&events[1]["choice"]);
    assert_eq!(text, "[3] ([0] [raw 01])");
    assert_eq!(fields["tag"], json!(3));
    assert_eq!(fields["constructed"], json!(true));
    assert_eq!(fields["data"], json!({"raw": "0901"}));
    assert_eq!(events[2]["kind"], json!("notification"));
    let values = events[2]["notification"]["event_values"].to_string();
    assert!(values.contains("30: [0] [raw 01]"), "{values}");
    assert!(values.contains(r#""raw":"0901""#), "{values}");

    // Trend Log: the records on either side keep their values.
    let trend = records(&mut lab, "bacnet_read_trend", "trend-log:9").await;
    assert_eq!(trend.len(), 3, "{trend:?}");
    assert_eq!(trend[0]["values"], json!([20.5]));
    assert_eq!(trend[1]["kind"], json!("unknown-choice"));
    assert_eq!(trend[1]["flagged"], json!(true));
    assert_eq!(choice(&trend[1]["choice"]).0, "[11] [raw 01 02]");
    assert_eq!(trend[2]["values"], json!([21.5]));

    // Trend Log Multiple: an unknown member beside a known one, then an
    // unknown log data.
    let multiple = records(&mut lab, "bacnet_read_trend", "trend-log-multiple:9").await;
    assert_eq!(multiple.len(), 2, "{multiple:?}");
    assert_eq!(multiple[0]["kind"], json!("values"));
    assert_eq!(multiple[0]["values"][0], json!(1.0));
    let (text, fields) = choice(&multiple[0]["values"][1]);
    assert_eq!(text, "[9] ()");
    assert_eq!(fields["constructed"], json!(true));
    assert_eq!(multiple[1]["kind"], json!("unknown-choice"));
    assert_eq!(choice(&multiple[1]["choice"]).0, "[3] [raw 07]");
    lab.stop().await;
}

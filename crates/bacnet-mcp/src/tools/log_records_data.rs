//! Event Log and Audit Log records as data, and the summary
//! `bacnet_read_log_records` computes over them (#2103).
//!
//! A record is an event notification (Clause 12.27.13) or an audit
//! notification (Clause 12.64.10), a change of the log's own status, or a
//! clock change. The notifications decode through
//! `bacnet_ops::format::log_records`, whose event values come from the
//! shared notification renderer (#2073); their device text (an event's
//! message, an audit's comments, values that may quote a string) comes
//! back in untrusted envelopes.
//!
//! A record is flagged when it changes the log's status, reports an event
//! into a state other than normal, or records an operation that failed.

use std::collections::BTreeMap;

use bacnet_client::log_reader::LogPage;
use bacnet_ops::format::log_records::{AuditNotificationData, EventNotificationData};
use bacnet_ops::label::kebab;
use bacnet_services::read_range::LogRecords;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditNotification, EventLogDatum, EventNotificationRequest,
};
use bacnet_types::enums::{EventState, NotifyType};
use serde_json::{Value, json};

use super::trend_data::{finite, log_status, stamp_text};
use crate::value::enveloped;

/// One record of an Event Log or an Audit Log.
pub(super) struct Record<'a> {
    pub(super) time: String,
    kind: Kind<'a>,
}

enum Kind<'a> {
    Event(&'a EventNotificationRequest),
    Audit(&'a BACnetAuditNotification),
    LogStatus(LogStatus),
    TimeChange(f32),
}

/// A page's records, oldest first; none for a log of another kind.
pub(super) fn records(page: &LogPage) -> Vec<Record<'_>> {
    match &page.records {
        LogRecords::EventLog(records) => records
            .iter()
            .map(|record| Record {
                time: stamp_text(&record.date, &record.time),
                kind: match &record.log_datum {
                    EventLogDatum::Notification(notification) => Kind::Event(notification),
                    EventLogDatum::LogStatus(status) => Kind::LogStatus(*status),
                    EventLogDatum::TimeChange(seconds) => Kind::TimeChange(*seconds),
                },
            })
            .collect(),
        LogRecords::AuditLog(records) => records
            .iter()
            .map(|record| {
                let (date, time) = &record.timestamp;
                Record {
                    time: stamp_text(date, time),
                    kind: match &record.datum {
                        BACnetAuditLogDatum::AuditNotification(notification) => {
                            Kind::Audit(notification)
                        }
                        BACnetAuditLogDatum::LogStatus(status) => Kind::LogStatus(*status),
                        BACnetAuditLogDatum::TimeChange(seconds) => Kind::TimeChange(*seconds),
                    },
                }
            })
            .collect(),
        LogRecords::TrendLog(_) | LogRecords::TrendLogMultiple(_) => Vec::new(),
    }
}

/// An enumeration's name in kebab case, or its number when it has none.
fn key(name: Option<&str>, raw: impl ToString) -> String {
    name.map_or_else(|| raw.to_string(), kebab)
}

impl Record<'_> {
    /// Whether the record is flagged: a change of the log's status, an
    /// event into a state other than normal, or a failed operation.
    fn flagged(&self) -> bool {
        match self.kind {
            Kind::LogStatus(_) => true,
            Kind::Event(event) => {
                event.notify_type != NotifyType::ACK_NOTIFICATION
                    && event.to_state != EventState::NORMAL
            }
            Kind::Audit(audit) => audit.result.is_some(),
            Kind::TimeChange(_) => false,
        }
    }

    /// The record as JSON: `seq` when the log numbered it, `source` naming
    /// the log for its device text, `cap` capping it.
    pub(super) fn json(&self, seq: Option<u64>, source: &str, cap: usize) -> Value {
        let mut row = json!({ "time": self.time, "flagged": self.flagged() });
        if let Some(seq) = seq {
            row["seq"] = json!(seq);
        }
        match self.kind {
            Kind::Event(event) => {
                row["kind"] = json!("notification");
                row["notification"] = enveloped(&EventNotificationData::new(event), source, cap);
            }
            Kind::Audit(audit) => {
                row["kind"] = json!("audit");
                row["audit"] = enveloped(&AuditNotificationData::new(audit), source, cap);
            }
            Kind::LogStatus(status) => {
                row["kind"] = json!("log-status");
                row["log_status"] = enveloped(&log_status(status), source, cap);
            }
            Kind::TimeChange(seconds) => {
                row["kind"] = json!("time-change");
                row["time_change_s"] = finite(f64::from(seconds));
            }
        }
        row
    }
}

/// What a call read, by kind and by type: event notifications by the state
/// they entered, their event type and notify type; audit notifications by
/// operation, and how many failed.
#[derive(Default)]
pub(super) struct Summary {
    records: u64,
    flagged: u64,
    notifications: u64,
    audits: u64,
    log_status: u64,
    time_changes: u64,
    failed: u64,
    first: Option<String>,
    last: Option<String>,
    by_to_state: BTreeMap<String, u64>,
    by_event_type: BTreeMap<String, u64>,
    by_notify_type: BTreeMap<String, u64>,
    by_operation: BTreeMap<String, u64>,
}

impl Summary {
    /// Count `record`.
    pub(super) fn add(&mut self, record: &Record<'_>) {
        self.records += 1;
        if record.flagged() {
            self.flagged += 1;
        }
        if self.first.is_none() {
            self.first = Some(record.time.clone());
        }
        self.last = Some(record.time.clone());
        match record.kind {
            Kind::Event(event) => {
                self.notifications += 1;
                for (counts, name) in [
                    (
                        &mut self.by_to_state,
                        key(event.to_state.name(), event.to_state.to_raw()),
                    ),
                    (
                        &mut self.by_event_type,
                        key(event.event_type.name(), event.event_type.to_raw()),
                    ),
                    (
                        &mut self.by_notify_type,
                        key(event.notify_type.name(), event.notify_type.to_raw()),
                    ),
                ] {
                    *counts.entry(name).or_default() += 1;
                }
            }
            Kind::Audit(audit) => {
                self.audits += 1;
                let operation = key(audit.operation.name(), audit.operation.to_raw());
                *self.by_operation.entry(operation).or_default() += 1;
                if audit.result.is_some() {
                    self.failed += 1;
                }
            }
            Kind::LogStatus(_) => self.log_status += 1,
            Kind::TimeChange(_) => self.time_changes += 1,
        }
    }

    /// The summary as JSON: the event counts for an Event Log, the audit
    /// counts for an Audit Log (`audit`).
    pub(super) fn json(&self, audit: bool) -> Value {
        let mut summary = json!({
            "records": self.records,
            "flagged": self.flagged,
            "log_status": self.log_status,
            "time_changes": self.time_changes,
            "first_time": self.first,
            "last_time": self.last,
        });
        if audit {
            summary["audits"] = json!(self.audits);
            summary["failed"] = json!(self.failed);
            summary["by_operation"] = json!(self.by_operation);
        } else {
            summary["notifications"] = json!(self.notifications);
            summary["by_to_state"] = json!(self.by_to_state);
            summary["by_event_type"] = json!(self.by_event_type);
            summary["by_notify_type"] = json!(self.by_notify_type);
        }
        summary
    }
}

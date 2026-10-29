//! ReadRange (RR) over a list or log-buffer property.
//!
//! A Trend Log, Event Log, Trend Log Multiple or Audit Log answers a ReadRange
//! of its Log_Buffer with records framed as their Clause 21 productions
//! (Clauses 12.25.14, 12.27.13, 12.30.19 and 12.64.10), not application-tagged
//! values. The record decoder is picked by the object's type, and each record
//! is shown as its timestamp, datum and, for a Trend Log, status flags. Any
//! other property or object type decodes as application values. Whatever
//! doesn't decode is shown as hex.
//!
//! The heading carries the result flags and the first sequence number, so a
//! page that left items out says so; `--all` pages through a whole log and
//! ends with the checkpoint to resume from (#1532).
//!
//! A Trend Log's records don't say which property they sample, so once the
//! device has answered the first ReadRange of its Log_Buffer, the command
//! reads its Log_DeviceObjectProperty and names the logged enumerations and
//! bit strings by the monitored property: one request more per command,
//! however many pages (#2091). When that read fails or names nothing, the
//! values print as logged. A column that samples a binary or multi-state
//! object's state then reads that object's state texts, one request an
//! object, and shows each state with its text, `2 (Low)` (#2122); a
//! refused read keeps `2`.

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogCursor, LogGap};
use bacnet_ops::destination::Destination;
use bacnet_ops::format::notification::EventValues;
use bacnet_ops::format::{Datum, Floats, LogColumns, ValueFormat};
use bacnet_services::read_range::{
    LogRecords, ReadRangeAck, ReadRangeRequest, ReadRangeValidation, ReadRangeViolation,
};
use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditNotification, BACnetEventLogRecord,
    BACnetLogMultipleRecord, BACnetLogRecord, BACnetRecipient, EventLogDatum,
    EventNotificationRequest, LogData, LogDatum,
};
use bacnet_types::enums::{NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use serde::Serialize;

use super::read::{format_items, formatter, hex};
pub use super::read_range_options::RangeOptions;
use crate::output::{OutputFormat, screen};

/// Read a range of items from a list or log-buffer property: one read over
/// the range `range` names, or with `--all` every page of a log.
#[allow(clippy::too_many_arguments)]
pub async fn read_range_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    object_type: ObjectType,
    instance: u32,
    property: PropertyIdentifier,
    index: Option<u32>,
    range: &RangeOptions,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let mut heading = Heading {
        object: format!("{object_type}:{instance}"),
        property: format!("{property}"),
        ..Heading::default()
    };
    if range.all {
        if property != PropertyIdentifier::LOG_BUFFER || index.is_some() {
            return Err("--all pages through a log's Log_Buffer".into());
        }
        // What was read is printed, with the flags that resume the read,
        // before any error that stopped it.
        let (rows, stopped) = read_all(client, to, oid, range, &mut heading).await?;
        print_records(&heading, &rows, format);
        return stopped.map_or(Ok(()), Err);
    }
    let request = ReadRangeRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        range: range.spec()?,
    };
    let validation = if range.strict {
        ReadRangeValidation::Strict
    } else {
        ReadRangeValidation::Lenient
    };
    let reply = to.read_range(client, &request, validation).await?;
    let ack = reply.ack;
    heading.item_count = u64::from(ack.item_count);
    heading.result_flags = ack.result_flags;
    heading.first_sequence_number = ack.first_sequence_number;
    heading.note_violations(&reply.violations);
    // Only once the device has answered: a silent one costs one timeout.
    let mut columns = LogColumns::default();
    if property == PropertyIdentifier::LOG_BUFFER {
        columns = to.log_columns(client, oid).await;
        to.log_state_texts(client, &mut columns).await;
    }
    match log_rows(&ack, &columns) {
        Some(rows) => print_records(&heading, &rows, format),
        None => print_items(&heading, object_type, property, &ack.item_data, format),
    }
    Ok(())
}

/// What stopped an `--all` read part way.
type Stopped = Option<Box<dyn std::error::Error>>;

/// Read every page of `log` from the start `range` names, filling in the
/// heading: the records, the last page's flags, the first sequence number,
/// the rules the device broke, the pages read, any gaps and the checkpoint.
/// An error after the options are checked stops the read but keeps what it
/// read, for the caller to print first. The log's columns, which name its
/// values, are read once, after the first page: a device that doesn't
/// answer costs one timeout, not two.
async fn read_all<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    log: ObjectIdentifier,
    range: &RangeOptions,
    heading: &mut Heading,
) -> Result<(LogRows, Stopped), Box<dyn std::error::Error>> {
    let (mut cursor, page_size) = range.pages()?;
    let mut rows = LogRows::default();
    let mut paged = Paged::default();
    let mut columns = LogColumns::default();
    let stopped = loop {
        let page = match to.read_log_page(client, log, cursor, page_size).await {
            Ok(page) => page,
            Err(error) => break Some(error.into()),
        };
        paged.pages += 1;
        if paged.pages == 1 {
            heading.first_sequence_number = page.first_sequence_number;
            columns = to.log_columns(client, log).await;
            to.log_state_texts(client, &mut columns).await;
        }
        heading.item_count += page.records.len() as u64;
        heading.result_flags = page.result_flags;
        heading.note_violations(&page.violations);
        paged.gaps.extend(page.gap);
        rows.rows.extend(record_rows(&page.records, &columns));
        cursor = page.next;
        if page.done {
            break None;
        }
    };
    paged.next = cursor_flags(cursor);
    heading.paged = Some(paged);
    Ok((rows, stopped))
}

/// The `read-range` flags that resume from `cursor`.
fn cursor_flags(cursor: LogCursor) -> String {
    match cursor {
        LogCursor::Oldest => "--all".into(),
        LogCursor::Sequence(sequence) => format!("--sequence {sequence}"),
        LogCursor::Position(position) => format!("--position {position}"),
        LogCursor::Time(date, time) => format!(
            "--time {:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:02}",
            date.actual_year().unwrap_or_default(),
            date.month,
            date.day,
            time.hour,
            time.minute,
            time.second,
            time.hundredths
        ),
    }
}

/// What every ReadRange output starts with.
#[derive(Default)]
struct Heading {
    object: String,
    property: String,
    /// Items returned; with `--all`, records over every page.
    item_count: u64,
    /// FIRST_ITEM, LAST_ITEM and MORE_ITEMS; with `--all`, the last page's.
    result_flags: (bool, bool, bool),
    /// The first item's sequence number; with `--all`, the first page's.
    first_sequence_number: Option<u64>,
    /// The rules the device's answers broke, each once, in the order met.
    violations: Vec<&'static str>,
    paged: Option<Paged>,
}

/// What an `--all` read adds to the heading.
#[derive(Default)]
struct Paged {
    pages: usize,
    gaps: Vec<LogGap>,
    /// The flags that resume the read later.
    next: String,
}

impl Heading {
    fn note_violations(&mut self, violations: &[ReadRangeViolation]) {
        for rule in violations {
            if !self.violations.contains(&rule.name()) {
                self.violations.push(rule.name());
            }
        }
    }

    /// The first line of the table output.
    fn line(&self) -> String {
        let (first, last, more) = self.result_flags;
        let flags: Vec<&str> = [
            (first, "FIRST_ITEM"),
            (last, "LAST_ITEM"),
            (more, "MORE_ITEMS"),
        ]
        .into_iter()
        .filter_map(|(set, name)| set.then_some(name))
        .collect();
        let flags = if flags.is_empty() {
            "none".to_string()
        } else {
            flags.join(",")
        };
        let mut line = format!(
            "ReadRange {}  {}  count={}  flags={flags}",
            self.object, self.property, self.item_count
        );
        if let Some(first) = self.first_sequence_number {
            line.push_str(&format!("  first-seq={first}"));
        }
        if !self.violations.is_empty() {
            line.push_str(&format!("  violations={}", self.violations.join(",")));
        }
        if let Some(paged) = &self.paged {
            line.push_str(&format!("  pages={}  next: {}", paged.pages, paged.next));
        }
        line
    }

    /// Lines after the table output, one for each gap.
    fn notes(&self) -> Vec<String> {
        self.paged
            .iter()
            .flat_map(|paged| &paged.gaps)
            .map(|gap| match gap.skipped {
                Some(skipped) => format!(
                    "  [gap] {skipped} records before sequence {} are gone (asked for {})",
                    gap.first, gap.expected
                ),
                None => format!(
                    "  [gap] the log's numbering restarted: asked for {}, read from {}",
                    gap.expected, gap.first
                ),
            })
            .collect()
    }

    /// The JSON object every output starts from.
    fn json(&self) -> serde_json::Value {
        let (first_item, last_item, more_items) = self.result_flags;
        let mut json = serde_json::json!({
            "object": self.object,
            "property": self.property,
            "item_count": self.item_count,
            "result_flags": {
                "first_item": first_item,
                "last_item": last_item,
                "more_items": more_items,
            },
            "first_sequence_number": self.first_sequence_number,
            "violations": self.violations,
        });
        if let Some(paged) = &self.paged {
            json["pages"] = paged.pages.into();
            json["next"] = paged.next.clone().into();
            json["gaps"] = paged
                .gaps
                .iter()
                .map(|gap| {
                    serde_json::json!({
                        "expected": gap.expected,
                        "first": gap.first,
                        "skipped": gap.skipped,
                    })
                })
                .collect();
        }
        json
    }
}

/// One log record as the CLI shows it.
#[derive(Debug, PartialEq, Serialize)]
struct LogRecordRow {
    timestamp: String,
    datum: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_flags: Option<String>,
    /// An Event Log notification's values, field by field (#2073).
    #[serde(skip_serializing_if = "Option::is_none")]
    event_values: Option<Datum>,
}

/// The records read from a log buffer, and the hex of whatever followed the
/// last record that decoded.
#[derive(Debug, Default)]
struct LogRows {
    rows: Vec<LogRecordRow>,
    undecoded: Option<String>,
}

/// The rows of a log buffer's records, when `ack` answers a read of the
/// Log_Buffer of a log object: every record that decodes, up to the first
/// that doesn't, whose octets on are kept as hex. `columns` names the
/// logged values.
fn log_rows(ack: &ReadRangeAck, columns: &LogColumns) -> Option<LogRows> {
    Some(match ack.log_records()? {
        Ok(records) => LogRows {
            rows: record_rows(&records, columns),
            undecoded: None,
        },
        Err(error) => LogRows {
            rows: record_rows(&error.decoded, columns),
            undecoded: ack
                .item_data
                .get(error.offset..)
                .filter(|rest| !rest.is_empty())
                .map(hex),
        },
    })
}

fn record_rows(records: &LogRecords, columns: &LogColumns) -> Vec<LogRecordRow> {
    match records {
        LogRecords::TrendLog(records) => records
            .iter()
            .map(|record| trend_log_row(record, columns))
            .collect(),
        LogRecords::EventLog(records) => records.iter().map(event_log_row).collect(),
        LogRecords::TrendLogMultiple(records) => records
            .iter()
            .map(|record| trend_log_multiple_row(record, columns))
            .collect(),
        LogRecords::AuditLog(records) => records.iter().map(audit_log_row).collect(),
    }
}

fn trend_log_row(record: &BACnetLogRecord, columns: &LogColumns) -> LogRecordRow {
    LogRecordRow {
        timestamp: timestamp(record.date, record.time),
        datum: log_datum(&record.log_datum, columns, 0),
        status_flags: record.status_flags.map(status_flags),
        event_values: None,
    }
}

fn event_log_row(record: &BACnetEventLogRecord) -> LogRecordRow {
    let mut event_values = None;
    let datum = match &record.log_datum {
        EventLogDatum::LogStatus(status) => log_status(*status),
        EventLogDatum::Notification(notification) => {
            let mut text = event_notification(notification);
            if let Some(parameters) = &notification.event_values {
                let values = EventValues::new(parameters);
                text.push_str(&format!(" {{{}}}", values.details));
                event_values = Some(values.data);
            }
            text
        }
        EventLogDatum::TimeChange(seconds) => time_change(*seconds),
    };
    LogRecordRow {
        timestamp: timestamp(record.date, record.time),
        datum,
        status_flags: None,
        event_values,
    }
}

/// A Trend Log Multiple record, each member's value named by its own
/// column.
fn trend_log_multiple_row(record: &BACnetLogMultipleRecord, columns: &LogColumns) -> LogRecordRow {
    let datum = match &record.log_data {
        LogData::LogStatus(status) => log_status(*status),
        LogData::Values(values) => {
            let values: Vec<String> = values
                .iter()
                .enumerate()
                .map(|(column, value)| log_datum(&LogDatum::from(value.clone()), columns, column))
                .collect();
            format!("[{}]", values.join(", "))
        }
        LogData::TimeChange(seconds) => time_change(*seconds),
    };
    LogRecordRow {
        timestamp: timestamp(record.date, record.time),
        datum,
        status_flags: None,
        event_values: None,
    }
}

fn audit_log_row(record: &BACnetAuditLogRecord) -> LogRecordRow {
    let datum = match &record.datum {
        BACnetAuditLogDatum::LogStatus(status) => log_status(*status),
        BACnetAuditLogDatum::AuditNotification(notification) => audit_notification(notification),
        BACnetAuditLogDatum::TimeChange(seconds) => time_change(*seconds),
    };
    let (date, time) = record.timestamp;
    LogRecordRow {
        timestamp: timestamp(date, time),
        datum,
        status_flags: None,
        event_values: None,
    }
}

fn timestamp(date: Date, time: Time) -> String {
    format!(
        "{} {}",
        bacnet_ops::format::format_property_value(&PropertyValue::Date(date)),
        bacnet_ops::format::format_property_value(&PropertyValue::Time(time))
    )
}

/// The set flags by name; empty when none is set.
fn status_flags(flags: StatusFlags) -> String {
    if flags.is_empty() {
        String::new()
    } else {
        flags.to_string()
    }
}

fn log_status(status: LogStatus) -> String {
    format!("log-status {status}")
}

fn time_change(seconds: f32) -> String {
    format!("time-change {seconds} s")
}

fn object(oid: ObjectIdentifier) -> String {
    format!("{}:{}", oid.object_type(), oid.instance_number())
}

/// A logged value, through the shared formatter, named for the property
/// column `index` of `columns` samples, and a state by its object's state
/// texts when they were read. With no column named, or a value the
/// property doesn't name, an enumeration shows as `enum 3` and a bit
/// string as its bits.
fn log_datum(datum: &LogDatum, columns: &LogColumns, index: usize) -> String {
    let column = columns.column(index);
    let format = ValueFormat::new(
        ObjectType::TREND_LOG,
        columns.state_texts(index),
        Floats::Exact,
    )
    .whole_lists();
    let value = match datum {
        LogDatum::LogStatus(status) => return log_status(*status),
        LogDatum::Failure {
            error_class,
            error_code,
        } => return format!("failure {error_class}/{error_code}"),
        LogDatum::TimeChange(seconds) => return time_change(*seconds),
        LogDatum::AnyValue(bytes) => return format.logged_octets(column, bytes),
        LogDatum::BooleanValue(value) => PropertyValue::Boolean(*value),
        LogDatum::RealValue(value) => PropertyValue::Real(*value),
        LogDatum::EnumValue(value) => match u32::try_from(*value) {
            Ok(value) => PropertyValue::Enumerated(value),
            Err(_) => return format!("enum {value}"),
        },
        LogDatum::UnsignedValue(value) => PropertyValue::Unsigned(*value),
        LogDatum::SignedValue(value) => match i32::try_from(*value) {
            Ok(value) => PropertyValue::Signed(value),
            Err(_) => return value.to_string(),
        },
        LogDatum::BitstringValue { unused_bits, data } => PropertyValue::BitString {
            unused_bits: *unused_bits,
            data: data.clone(),
        },
        LogDatum::NullValue => PropertyValue::Null,
    };
    format.logged(column, &value)
}

/// Notify type, event type, event object and transition, then the message;
/// the caller adds the event values.
fn event_notification(notification: &EventNotificationRequest) -> String {
    let transition = if notification.notify_type == NotifyType::ACK_NOTIFICATION {
        notification.to_state.to_string()
    } else {
        format!("{} -> {}", notification.from_state, notification.to_state)
    };
    let mut text = format!(
        "{} {} {} {transition}",
        notification.notify_type,
        notification.event_type,
        object(notification.event_object_identifier)
    );
    if let Some(message) = &notification.message_text {
        text.push_str(&format!(" \"{message}\""));
    }
    text
}

/// Operation and target, the device that asked for it, and any error.
fn audit_notification(notification: &BACnetAuditNotification) -> String {
    let mut text = format!(
        "{} {}",
        notification.operation,
        recipient(&notification.target_device)
    );
    if let Some(target) = notification.target_object {
        text.push_str(&format!(" {}", object(target)));
    }
    if let Some(property) = &notification.target_property {
        text.push_str(&format!(" {}", property.property_identifier));
        if let Some(index) = property.property_array_index {
            text.push_str(&format!("[{index}]"));
        }
    }
    text.push_str(&format!(" by {}", recipient(&notification.source_device)));
    if let Some((class, code)) = notification.result {
        text.push_str(&format!(" failed {class}/{code}"));
    }
    text
}

fn recipient(recipient: &BACnetRecipient) -> String {
    match recipient {
        BACnetRecipient::Device(oid) => object(*oid),
        BACnetRecipient::Address(address) => format!(
            "network {} mac [{}]",
            address.network_number,
            hex(&address.mac_address)
        ),
    }
}

fn print_records(heading: &Heading, records: &LogRows, format: OutputFormat) {
    match format {
        OutputFormat::Table => {
            println!("{}", heading.line());
            let flags = records.rows.iter().any(|row| row.status_flags.is_some());
            let mut table = comfy_table::Table::new();
            let mut header = vec!["#", "Timestamp", "Datum"];
            if flags {
                header.push("Status flags");
            }
            table.set_header(header);
            for (number, row) in (1..).zip(&records.rows) {
                // The datum may hold device text: a logged string or an
                // event's message.
                let mut cells = vec![
                    number.to_string(),
                    row.timestamp.clone(),
                    screen(&row.datum),
                ];
                if flags {
                    cells.push(row.status_flags.clone().unwrap_or_default());
                }
                table.add_row(cells);
            }
            println!("{table}");
            if let Some(hex) = &records.undecoded {
                println!("  [raw {hex}]");
            }
            for note in heading.notes() {
                println!("{note}");
            }
        }
        OutputFormat::Json => {
            let mut json = heading.json();
            json["records"] = serde_json::to_value(&records.rows).unwrap_or_default();
            if let Some(hex) = &records.undecoded {
                json["undecoded"] = serde_json::Value::from(hex.as_str());
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
    }
}

/// The items of a read of any other list or array: each as text, and in
/// JSON as typed data beside the text (`decoded`).
fn print_items(
    heading: &Heading,
    object_type: ObjectType,
    property: PropertyIdentifier,
    data: &[u8],
    format: OutputFormat,
) {
    let values = formatter(object_type);
    let (mut items, undecoded) = format_items(&values, property, data);
    match format {
        OutputFormat::Table => {
            println!("{}", heading.line());
            for (number, item) in (1..).zip(&items) {
                println!("  [{number}] {}", screen(item));
            }
            if let Some(raw) = undecoded {
                println!("  {raw}");
            }
        }
        OutputFormat::Json => {
            if let Some(raw) = undecoded {
                items.push(raw);
            }
            let mut json = heading.json();
            json["items"] = items.into();
            json["decoded"] = serde_json::to_value(values.data(property, data)).unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            );
        }
    }
}

#[cfg(test)]
#[path = "read_range_tests.rs"]
mod tests;

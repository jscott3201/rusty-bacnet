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

use bacnet_client::client::BACnetClient;
use bacnet_client::log_reader::{LogCursor, LogGap};
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
use bacnet_types::enums::{ErrorClass, ErrorCode, NotifyType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use serde::Serialize;

use super::read::{decode_and_format, format_application_values, hex};
pub use super::read_range_options::RangeOptions;
use crate::output::{self, OutputFormat};

/// Read a range of items from a list or log-buffer property: one read over
/// the range `range` names, or with `--all` every page of a log.
#[allow(clippy::too_many_arguments)]
pub async fn read_range_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
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
        let (rows, stopped) = read_all(client, mac, oid, range, &mut heading).await?;
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
    let reply = client.read_range_with(mac, &request, validation).await?;
    let ack = reply.ack;
    heading.item_count = u64::from(ack.item_count);
    heading.result_flags = ack.result_flags;
    heading.first_sequence_number = ack.first_sequence_number;
    heading.note_violations(&reply.violations);
    match log_rows(&ack) {
        Some(rows) => print_records(&heading, &rows, format),
        None => print_items(&heading, &ack.item_data, format),
    }
    Ok(())
}

/// What stopped an `--all` read part way.
type Stopped = Option<Box<dyn std::error::Error>>;

/// Read every page of `log` from the start `range` names, filling in the
/// heading: the records, the last page's flags, the first sequence number,
/// the rules the device broke, the pages read, any gaps and the checkpoint.
/// An error after the options are checked stops the read but keeps what it
/// read, for the caller to print first.
async fn read_all<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    log: ObjectIdentifier,
    range: &RangeOptions,
    heading: &mut Heading,
) -> Result<(LogRows, Stopped), Box<dyn std::error::Error>> {
    let (mut cursor, page_size) = range.pages()?;
    let mut rows = LogRows::default();
    let mut paged = Paged::default();
    let stopped = loop {
        let page = match client.read_log_page(mac, log, cursor, page_size).await {
            Ok(page) => page,
            Err(error) => break Some(error.into()),
        };
        paged.pages += 1;
        if paged.pages == 1 {
            heading.first_sequence_number = page.first_sequence_number;
        }
        heading.item_count += page.records.len() as u64;
        heading.result_flags = page.result_flags;
        heading.note_violations(&page.violations);
        paged.gaps.extend(page.gap);
        rows.rows.extend(record_rows(&page.records));
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
/// that doesn't, whose octets on are kept as hex.
fn log_rows(ack: &ReadRangeAck) -> Option<LogRows> {
    Some(match ack.log_records()? {
        Ok(records) => LogRows {
            rows: record_rows(&records),
            undecoded: None,
        },
        Err(error) => LogRows {
            rows: record_rows(&error.decoded),
            undecoded: ack
                .item_data
                .get(error.offset..)
                .filter(|rest| !rest.is_empty())
                .map(hex),
        },
    })
}

fn record_rows(records: &LogRecords) -> Vec<LogRecordRow> {
    match records {
        LogRecords::TrendLog(records) => records.iter().map(trend_log_row).collect(),
        LogRecords::EventLog(records) => records.iter().map(event_log_row).collect(),
        LogRecords::TrendLogMultiple(records) => {
            records.iter().map(trend_log_multiple_row).collect()
        }
        LogRecords::AuditLog(records) => records.iter().map(audit_log_row).collect(),
    }
}

fn trend_log_row(record: &BACnetLogRecord) -> LogRecordRow {
    LogRecordRow {
        timestamp: timestamp(record.date, record.time),
        datum: log_datum(&record.log_datum),
        status_flags: record.status_flags.map(status_flags),
    }
}

fn event_log_row(record: &BACnetEventLogRecord) -> LogRecordRow {
    let datum = match &record.log_datum {
        EventLogDatum::LogStatus(status) => log_status(*status),
        EventLogDatum::Notification(notification) => event_notification(notification),
        EventLogDatum::TimeChange(seconds) => time_change(*seconds),
    };
    LogRecordRow {
        timestamp: timestamp(record.date, record.time),
        datum,
        status_flags: None,
    }
}

fn trend_log_multiple_row(record: &BACnetLogMultipleRecord) -> LogRecordRow {
    let datum = match &record.log_data {
        LogData::LogStatus(status) => log_status(*status),
        LogData::Values(values) => {
            let values: Vec<String> = values
                .iter()
                .map(|value| log_datum(&LogDatum::from(value.clone())))
                .collect();
            format!("[{}]", values.join(", "))
        }
        LogData::TimeChange(seconds) => time_change(*seconds),
    };
    LogRecordRow {
        timestamp: timestamp(record.date, record.time),
        datum,
        status_flags: None,
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
    }
}

fn timestamp(date: Date, time: Time) -> String {
    format!(
        "{} {}",
        output::format_property_value(&PropertyValue::Date(date)),
        output::format_property_value(&PropertyValue::Time(time))
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

fn log_datum(datum: &LogDatum) -> String {
    match datum {
        LogDatum::LogStatus(status) => log_status(*status),
        LogDatum::BooleanValue(value) => value.to_string(),
        LogDatum::RealValue(value) => value.to_string(),
        LogDatum::EnumValue(value) => format!("enumerated({value})"),
        LogDatum::UnsignedValue(value) => value.to_string(),
        LogDatum::SignedValue(value) => value.to_string(),
        LogDatum::BitstringValue { unused_bits, data } => {
            output::format_property_value(&PropertyValue::BitString {
                unused_bits: *unused_bits,
                data: data.clone(),
            })
        }
        LogDatum::NullValue => "null".to_string(),
        LogDatum::Failure {
            error_class,
            error_code,
        } => format!("failure {}", raw_error(*error_class, *error_code)),
        LogDatum::TimeChange(seconds) => time_change(*seconds),
        LogDatum::AnyValue(bytes) => decode_and_format(bytes),
    }
}

/// An error class and code by name, or by number when one doesn't fit the
/// enumeration.
fn raw_error(class: u32, code: u32) -> String {
    let class = u16::try_from(class).map_or(class.to_string(), |raw| {
        ErrorClass::from_raw(raw).to_string()
    });
    let code =
        u16::try_from(code).map_or(code.to_string(), |raw| ErrorCode::from_raw(raw).to_string());
    format!("{class}/{code}")
}

/// Notify type, event type, event object and transition, then the message.
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
                let mut cells = vec![number.to_string(), row.timestamp.clone(), row.datum.clone()];
                if flags {
                    cells.push(row.status_flags.clone().unwrap_or_default());
                }
                table.add_row(cells);
            }
            println!("{table}");
            if let Some(hex) = &records.undecoded {
                println!("  [raw] {hex}");
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

fn print_items(heading: &Heading, data: &[u8], format: OutputFormat) {
    let (mut items, undecoded) = format_application_values(data);
    match format {
        OutputFormat::Table => {
            println!("{}", heading.line());
            for (number, item) in (1..).zip(&items) {
                println!("  [{number}] {item}");
            }
            if let Some(hex) = undecoded {
                println!("  [raw] {hex}");
            }
        }
        OutputFormat::Json => {
            if let Some(hex) = undecoded {
                items.push(format!("[raw: {hex}]"));
            }
            let mut json = heading.json();
            json["items"] = items.into();
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

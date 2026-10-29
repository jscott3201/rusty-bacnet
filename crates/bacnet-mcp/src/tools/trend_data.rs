//! Trend log records as data, and the summary `bacnet_read_trend` computes
//! over them.
//!
//! A record is a sampled value (a typed datum, the CLI's JSON shape), the
//! error that stopped a sample, a change of the log's own status, or a
//! clock change. Records carrying status flags with any flag set, and log
//! status records, are flagged, since either means the samples around them
//! may not be what they seem.
//!
//! Each column's values are named by the property the log samples there,
//! from its Log_DeviceObjectProperty ([`LogColumns`], #2091): an
//! enumeration with its name, a bit string with its set bits' names, and
//! the summary's state counts by name. A column nothing names keeps the
//! numbers.
//!
//! A column that samples a binary or multi-state object's state counts its
//! samples as states, keyed `active`/`inactive` or by state number. The
//! object's own state texts (#2122) are device strings, so they never
//! become keys or splice into a value: the column's `state_texts` maps
//! each of those same keys to its text in an untrusted envelope.

use std::collections::BTreeMap;

use bacnet_ops::format::{Datum, Floats, LogColumns, LoggedProperty, StateTexts, ValueFormat};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetLogMultipleRecord, BACnetLogRecord, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, PropertyValue, StatusFlags, Time};
use serde_json::{Value, json};

use crate::errors::remote_json;
use crate::untrusted::envelope;
use crate::value::{enveloped, object_text};

/// A date and time as `2026-10-25 07:00:00.00`, unspecified fields as 0,
/// so the text sorts as the time does.
pub(super) fn stamp_text(date: &Date, time: &Time) -> String {
    let part = |value: u8| if value == 0xFF { 0 } else { value };
    let year = if date.year == 0xFF {
        0
    } else {
        u16::from(date.year) + 1900
    };
    format!(
        "{year:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:02}",
        part(date.month),
        part(date.day),
        part(time.hour),
        part(time.minute),
        part(time.second),
        part(time.hundredths)
    )
}

/// What one log entry carries, ready for the result and the summary.
pub(super) struct Entry {
    pub(super) time: String,
    pub(super) kind: Kind,
    pub(super) status_flags: Option<StatusFlags>,
}

/// The parts of an entry.
pub(super) enum Kind {
    /// One sampled value or failure per column: one for a Trend Log, one
    /// per member for a Trend Log Multiple.
    Values(Vec<Sample>),
    LogStatus(LogStatus),
    TimeChange(f32),
}

/// One column's sample.
pub(super) enum Sample {
    Value(PropertyValue),
    /// A signed sample, which a log keeps to 64 bits.
    Signed(i64),
    /// A value of another datatype, as its tagged octets.
    Any(Vec<u8>),
    Failure(ErrorClass, ErrorCode),
}

impl Entry {
    /// A Trend Log record.
    pub(super) fn trend(record: &BACnetLogRecord) -> Self {
        let kind = match &record.log_datum {
            LogDatum::LogStatus(status) => Kind::LogStatus(*status),
            LogDatum::TimeChange(seconds) => Kind::TimeChange(*seconds),
            LogDatum::BooleanValue(v) => value(PropertyValue::Boolean(*v)),
            LogDatum::RealValue(v) => value(PropertyValue::Real(*v)),
            LogDatum::EnumValue(v) => value(enumerated(*v)),
            LogDatum::UnsignedValue(v) => value(PropertyValue::Unsigned(*v)),
            LogDatum::SignedValue(v) => Kind::Values(vec![Sample::Signed(*v)]),
            LogDatum::BitstringValue { unused_bits, data } => value(PropertyValue::BitString {
                unused_bits: *unused_bits,
                data: data.clone(),
            }),
            LogDatum::NullValue => value(PropertyValue::Null),
            LogDatum::Failure {
                error_class,
                error_code,
            } => Kind::Values(vec![Sample::Failure(*error_class, *error_code)]),
            LogDatum::AnyValue(octets) => Kind::Values(vec![Sample::Any(octets.clone())]),
        };
        Self {
            time: stamp_text(&record.date, &record.time),
            kind,
            status_flags: record.status_flags,
        }
    }

    /// A Trend Log Multiple record.
    pub(super) fn multiple(record: &BACnetLogMultipleRecord) -> Self {
        let kind = match &record.log_data {
            LogData::LogStatus(status) => Kind::LogStatus(*status),
            LogData::TimeChange(seconds) => Kind::TimeChange(*seconds),
            LogData::Values(values) => Kind::Values(values.iter().map(sample).collect()),
        };
        Self {
            time: stamp_text(&record.date, &record.time),
            kind,
            status_flags: None,
        }
    }

    /// Whether the record is flagged: status flags with any flag set, or a
    /// change of the log's status.
    pub(super) fn flagged(&self) -> bool {
        matches!(self.kind, Kind::LogStatus(_))
            || self.status_flags.is_some_and(|flags| !flags.is_empty())
    }

    /// The record as JSON: `seq` when the log numbered it, `source` naming
    /// the log for any device text in a value, `cap` capping it, and
    /// `columns` naming each column's values.
    pub(super) fn json(
        &self,
        seq: Option<u64>,
        source: &str,
        cap: usize,
        columns: &LogColumns,
    ) -> Value {
        let mut row = json!({ "time": self.time, "flagged": self.flagged() });
        if let Some(seq) = seq {
            row["seq"] = json!(seq);
        }
        if let Some(flags) = self.status_flags {
            row["status_flags"] = enveloped(&status_flags(flags), source, cap);
        }
        match &self.kind {
            Kind::Values(samples) => {
                row["kind"] = json!("values");
                let values: Vec<Value> = samples
                    .iter()
                    .enumerate()
                    .map(|(column, sample)| {
                        sample_json(sample, columns.column(column), source, cap)
                    })
                    .collect();
                row["values"] = json!(values);
            }
            Kind::LogStatus(status) => {
                row["kind"] = json!("log-status");
                row["log_status"] = enveloped(&log_status(*status), source, cap);
            }
            Kind::TimeChange(seconds) => {
                row["kind"] = json!("time-change");
                row["time_change_s"] = finite(f64::from(*seconds));
            }
        }
        row
    }
}

fn value(value: PropertyValue) -> Kind {
    Kind::Values(vec![Sample::Value(value)])
}

/// An enumerated sample: ENUMERATED when it fits 32 bits, else the number.
fn enumerated(raw: u64) -> PropertyValue {
    match u32::try_from(raw) {
        Ok(raw) => PropertyValue::Enumerated(raw),
        Err(_) => PropertyValue::Unsigned(raw),
    }
}

fn sample(value: &LogValue) -> Sample {
    match value {
        LogValue::BooleanValue(v) => Sample::Value(PropertyValue::Boolean(*v)),
        LogValue::RealValue(v) => Sample::Value(PropertyValue::Real(*v)),
        LogValue::EnumValue(v) => Sample::Value(enumerated(*v)),
        LogValue::UnsignedValue(v) => Sample::Value(PropertyValue::Unsigned(*v)),
        LogValue::SignedValue(v) => Sample::Signed(*v),
        LogValue::BitstringValue { unused_bits, data } => Sample::Value(PropertyValue::BitString {
            unused_bits: *unused_bits,
            data: data.clone(),
        }),
        LogValue::NullValue => Sample::Value(PropertyValue::Null),
        LogValue::Failure {
            error_class,
            error_code,
        } => Sample::Failure(*error_class, *error_code),
        LogValue::AnyValue(octets) => Sample::Any(octets.clone()),
    }
}

/// The formatter samples are named with: no state texts, every digit.
fn with_format<R>(f: impl FnOnce(&ValueFormat<'_>) -> R) -> R {
    let texts = StateTexts::default();
    f(&ValueFormat::new(
        ObjectType::TREND_LOG,
        &texts,
        Floats::Exact,
    ))
}

/// A sampled value, named for the property `column` samples.
fn datum(value: &PropertyValue, column: Option<LoggedProperty>) -> Datum {
    with_format(|format| format.logged_data(column, value))
}

fn status_flags(flags: StatusFlags) -> Datum {
    let value = PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags.bits() << 4],
    };
    with_format(|format| format.element_data(PropertyIdentifier::STATUS_FLAGS, &value))
}

/// A log status as named bits.
pub(super) fn log_status(status: LogStatus) -> Datum {
    let names: Vec<String> = [
        (LogStatus::LOG_DISABLED, "log-disabled"),
        (LogStatus::BUFFER_PURGED, "buffer-purged"),
        (LogStatus::LOG_INTERRUPTED, "log-interrupted"),
    ]
    .iter()
    .filter(|(bit, _)| status.contains(*bit))
    .map(|(_, name)| (*name).to_string())
    .collect();
    let bits: String = [
        LogStatus::LOG_DISABLED,
        LogStatus::BUFFER_PURGED,
        LogStatus::LOG_INTERRUPTED,
    ]
    .iter()
    .map(|bit| if status.contains(*bit) { '1' } else { '0' })
    .collect();
    Datum::Bits {
        bits,
        names: Some(names),
    }
}

fn sample_json(sample: &Sample, column: Option<LoggedProperty>, source: &str, cap: usize) -> Value {
    match sample {
        Sample::Value(value) => enveloped(&datum(value, column), source, cap),
        Sample::Signed(value) => json!(value),
        Sample::Any(octets) => {
            let decoded = with_format(|format| format.logged_octets_data(column, octets));
            enveloped(&decoded, source, cap)
        }
        Sample::Failure(class, code) => {
            let remote = RemoteError::Error {
                class: *class,
                code: *code,
            };
            json!({ "error": remote_json(&remote) })
        }
    }
}

/// A number JSON can hold, or null.
pub(super) fn finite(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        Value::Null
    }
}

/// One column's statistics.
#[derive(Debug, Default, Clone)]
struct Column {
    numeric: u64,
    min: f64,
    max: f64,
    sum: f64,
    /// Booleans and enumerations, by value: an enumeration by name when
    /// the column's property names it, and a multi-state object's state
    /// by its number.
    states: BTreeMap<String, u64>,
    failures: u64,
    other: u64,
}

impl Column {
    fn add(&mut self, sample: &Sample, column: Option<LoggedProperty>) {
        // A multi-state object's state is a number that counts as a state.
        if let (Sample::Value(PropertyValue::Unsigned(state)), Some(column)) = (sample, column)
            && column.holds_states()
        {
            *self.states.entry(state.to_string()).or_default() += 1;
            return;
        }
        let number = match sample {
            Sample::Value(PropertyValue::Real(v)) if v.is_finite() => Some(f64::from(*v)),
            Sample::Value(PropertyValue::Double(v)) if v.is_finite() => Some(*v),
            #[allow(clippy::cast_precision_loss)]
            Sample::Value(PropertyValue::Unsigned(v)) => Some(*v as f64),
            #[allow(clippy::cast_precision_loss)]
            Sample::Signed(v) => Some(*v as f64),
            _ => None,
        };
        if let Some(number) = number {
            if self.numeric == 0 {
                self.min = number;
                self.max = number;
            } else {
                self.min = self.min.min(number);
                self.max = self.max.max(number);
            }
            self.sum += number;
            self.numeric += 1;
            return;
        }
        match sample {
            Sample::Value(PropertyValue::Boolean(v)) => {
                *self.states.entry(v.to_string()).or_default() += 1;
            }
            Sample::Value(value @ PropertyValue::Enumerated(v)) => {
                let state = match datum(value, column) {
                    Datum::Enumerated {
                        name: Some(name), ..
                    } => name,
                    _ => v.to_string(),
                };
                *self.states.entry(state).or_default() += 1;
            }
            Sample::Failure(..) => self.failures += 1,
            _ => self.other += 1,
        }
    }

    /// The device's text for each state key the column counted, in an
    /// envelope from `source`: `active`/`inactive` by Active_Text and
    /// Inactive_Text, a state number by its State_Text entry. `None` when
    /// no counted state has a text.
    fn state_texts(&self, texts: &StateTexts, source: &str, cap: usize) -> Option<Value> {
        let labels: serde_json::Map<String, Value> = self
            .states
            .keys()
            .filter_map(|key| {
                let (text, property) = match key.as_str() {
                    "active" => (texts.active.as_ref(), "active-text"),
                    "inactive" => (texts.inactive.as_ref(), "inactive-text"),
                    number => (
                        number
                            .parse::<usize>()
                            .ok()
                            .and_then(|state| state.checked_sub(1))
                            .and_then(|index| texts.states.get(index)),
                        "state-text",
                    ),
                };
                let text = text.filter(|text| !text.is_empty())?;
                Some((
                    key.clone(),
                    envelope(text, format!("{source} {property}"), cap),
                ))
            })
            .collect();
        (!labels.is_empty()).then_some(Value::Object(labels))
    }

    /// The column as JSON; `labels` is its [`Self::state_texts`].
    fn json(&self, index: usize, labels: Option<Value>) -> Value {
        let numeric = (self.numeric > 0).then(|| {
            #[allow(clippy::cast_precision_loss)]
            let mean = self.sum / self.numeric as f64;
            json!({
                "count": self.numeric,
                "min": finite(self.min),
                "max": finite(self.max),
                "mean": finite(mean),
            })
        });
        json!({
            "column": index,
            "numeric": numeric,
            "states": (!self.states.is_empty()).then(|| json!(self.states)),
            "state_texts": labels,
            "failures": self.failures,
            "other": self.other,
        })
    }
}

/// The summary of the records a call read.
#[derive(Debug, Default)]
pub(super) struct Summary {
    records: u64,
    flagged: u64,
    log_status: u64,
    time_changes: u64,
    first: Option<String>,
    last: Option<String>,
    columns: Vec<Column>,
}

impl Summary {
    /// Count `entry`, its values named by `columns`.
    pub(super) fn add(&mut self, entry: &Entry, columns: &LogColumns) {
        self.records += 1;
        if entry.flagged() {
            self.flagged += 1;
        }
        if self.first.is_none() {
            self.first = Some(entry.time.clone());
        }
        self.last = Some(entry.time.clone());
        match &entry.kind {
            Kind::Values(samples) => {
                if self.columns.len() < samples.len() {
                    self.columns.resize(samples.len(), Column::default());
                }
                for (index, (column, sample)) in self.columns.iter_mut().zip(samples).enumerate() {
                    column.add(sample, columns.column(index));
                }
            }
            Kind::LogStatus(_) => self.log_status += 1,
            Kind::TimeChange(_) => self.time_changes += 1,
        }
    }

    /// The summary as JSON: each column's state keys labelled with the
    /// state texts `columns` holds, enveloped as from `device` with their
    /// text capped at `cap`.
    pub(super) fn json(&self, columns: &LogColumns, device: u32, cap: usize) -> Value {
        let columns: Vec<Value> = self
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                let labels = columns.column(index).and_then(|logged| {
                    let object = logged.object;
                    let source = format!(
                        "device {device} {}",
                        object_text(object.object_type(), object.instance_number())
                    );
                    column.state_texts(columns.raw_state_texts(index), &source, cap)
                });
                column.json(index, labels)
            })
            .collect();
        json!({
            "records": self.records,
            "flagged": self.flagged,
            "log_status": self.log_status,
            "time_changes": self.time_changes,
            "first_time": self.first,
            "last_time": self.last,
            "columns": columns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minute: u8) -> (Date, Time) {
        let date = Date {
            year: 126,
            month: 10,
            day: 25,
            day_of_week: 7,
        };
        let time = Time {
            hour: 7,
            minute,
            second: 0,
            hundredths: 0,
        };
        (date, time)
    }

    /// A Trend Log Multiple summarises each member as a column; log
    /// status records are flagged and counted, failures counted, and a
    /// failure's error kept as data.
    #[test]
    fn multiple_logs_summarise_by_column() {
        let record = |minute, log_data| {
            let (date, time) = at(minute);
            BACnetLogMultipleRecord {
                date,
                time,
                log_data,
            }
        };
        let records = [
            record(
                0,
                LogData::Values(vec![LogValue::RealValue(1.5), LogValue::EnumValue(1)]),
            ),
            record(
                1,
                LogData::Values(vec![
                    LogValue::SignedValue(-4),
                    LogValue::Failure {
                        error_class: ErrorClass::DEVICE,
                        error_code: ErrorCode::OPERATIONAL_PROBLEM,
                    },
                ]),
            ),
            record(2, LogData::LogStatus(LogStatus::LOG_INTERRUPTED)),
            record(3, LogData::TimeChange(2.0)),
        ];
        let mut summary = Summary::default();
        let entries: Vec<Entry> = records.iter().map(Entry::multiple).collect();
        for entry in &entries {
            summary.add(entry, &LogColumns::default());
        }
        let json = summary.json(&LogColumns::default(), 1, 256);
        assert_eq!(json["records"], json!(4));
        assert_eq!(json["flagged"], json!(1));
        assert_eq!(json["log_status"], json!(1));
        assert_eq!(json["time_changes"], json!(1));
        assert_eq!(json["first_time"], json!("2026-10-25 07:00:00.00"));
        let first = &json["columns"][0]["numeric"];
        assert_eq!(first["min"], json!(-4.0));
        assert_eq!(first["max"], json!(1.5));
        assert_eq!(first["mean"], json!(-1.25));
        assert_eq!(json["columns"][1]["states"], json!({ "1": 1 }));
        assert_eq!(json["columns"][1]["failures"], json!(1));
        let row = entries[1].json(Some(2), "s", 256, &LogColumns::default());
        assert_eq!(row["values"][0], json!(-4));
        assert_eq!(
            row["values"][1]["error"]["code"],
            json!("operational-problem")
        );
        let status = entries[2].json(None, "s", 256, &LogColumns::default());
        assert_eq!(status["kind"], json!("log-status"));
        assert_eq!(status["flagged"], json!(true));
        assert_eq!(status["log_status"]["names"], json!(["log-interrupted"]));
    }

    /// Each column's values are named by the property it samples, in the
    /// records and in the summary's state counts; a column nothing names,
    /// and a value its property doesn't name, keep the numbers (#2091).
    #[test]
    fn columns_name_values_and_states() {
        let column = |property| {
            Some(LoggedProperty {
                object: bacnet_types::primitives::ObjectIdentifier::new(
                    ObjectType::ANALOG_INPUT,
                    1,
                )
                .unwrap(),
                property,
                index: None,
                device: None,
            })
        };
        let columns: LogColumns = [
            column(PropertyIdentifier::EVENT_STATE),
            column(PropertyIdentifier::STATUS_FLAGS),
            None,
        ]
        .into_iter()
        .collect();
        let (date, time) = at(0);
        let record = |values| BACnetLogMultipleRecord {
            date,
            time,
            log_data: LogData::Values(values),
        };
        let records = [
            record(vec![
                LogValue::EnumValue(2),
                LogValue::BitstringValue {
                    unused_bits: 4,
                    data: vec![0b1000_0000],
                },
                LogValue::EnumValue(2),
            ]),
            record(vec![
                LogValue::EnumValue(0),
                LogValue::EnumValue(2),
                LogValue::EnumValue(2),
            ]),
        ];
        let entries: Vec<Entry> = records.iter().map(Entry::multiple).collect();
        let row = entries[0].json(None, "s", 256, &columns);
        assert_eq!(row["values"][0], json!({ "value": 2, "name": "offnormal" }));
        assert_eq!(
            row["values"][1],
            json!({ "bits": "1000", "names": ["in-alarm"] })
        );
        assert_eq!(row["values"][2], json!({ "value": 2 }));
        let unnamed = entries[0].json(None, "s", 256, &LogColumns::default());
        assert_eq!(unnamed["values"][0], json!({ "value": 2 }));
        assert_eq!(unnamed["values"][1], json!({ "bits": "1000" }));

        let mut summary = Summary::default();
        for entry in &entries {
            summary.add(entry, &columns);
        }
        let json = summary.json(&columns, 1, 256);
        assert_eq!(
            json["columns"][0]["states"],
            json!({ "normal": 1, "offnormal": 1 })
        );
        // Status_Flags names no enumeration; the third column has no name.
        assert_eq!(json["columns"][1]["states"], json!({ "2": 1 }));
        assert_eq!(json["columns"][2]["states"], json!({ "2": 2 }));
    }
}

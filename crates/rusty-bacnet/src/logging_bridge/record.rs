//! An event, captured on the thread that emitted it, ready for `logging`.

use std::time::SystemTime;

use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use tracing::field::{Field, Visit};
use tracing::{Event, Metadata};

/// One event: where it came from, its message and fields, and when.
pub(super) struct Record {
    pub(super) metadata: &'static Metadata<'static>,
    pub(super) message: Option<String>,
    pub(super) fields: Vec<(&'static str, Value)>,
    pub(super) created: SystemTime,
}

impl Record {
    /// Capture `event` now.
    pub(super) fn capture(event: &Event<'_>) -> Self {
        let mut fields = Fields::default();
        event.record(&mut fields);
        Self {
            metadata: event.metadata(),
            message: fields.message,
            fields: fields.fields,
            created: SystemTime::now(),
        }
    }

    /// The record's message: the event's message, then each field as
    /// `key=value`.
    pub(super) fn text(&self) -> String {
        message_text(self.message.as_deref(), &self.fields)
    }
}

/// A field's value, kept as a Python type where one fits.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Value {
    Str(String),
    I64(i64),
    U64(u64),
    F64(f64),
    Bool(bool),
    /// Anything else, by its `Debug` (or, for `%field`, `Display`) form.
    Text(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Str(s) | Self::Text(s) => f.write_str(s),
            Self::I64(n) => write!(f, "{n}"),
            Self::U64(n) => write!(f, "{n}"),
            Self::F64(n) => write!(f, "{n}"),
            Self::Bool(b) => write!(f, "{b}"),
        }
    }
}

impl Value {
    pub(super) fn to_python<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            Self::Str(s) | Self::Text(s) => s.into_bound_py_any(py),
            Self::I64(n) => n.into_bound_py_any(py),
            Self::U64(n) => n.into_bound_py_any(py),
            Self::F64(n) => n.into_bound_py_any(py),
            Self::Bool(b) => b.into_bound_py_any(py),
        }
    }
}

/// Collects an event's message and fields.
#[derive(Default)]
struct Fields {
    message: Option<String>,
    fields: Vec<(&'static str, Value)>,
}

impl Fields {
    fn put(&mut self, field: &Field, value: Value) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push((field.name(), value));
        }
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field, Value::Text(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, Value::Str(value.to_owned()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, Value::I64(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, Value::U64(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, Value::F64(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, Value::Bool(value));
    }
}

/// The event's message, then each field as `key=value`. Values go in as
/// they are, unquoted: `record.bacnet_fields` keeps the structure.
pub(super) fn message_text(message: Option<&str>, fields: &[(&'static str, Value)]) -> String {
    let mut text = message.unwrap_or_default().to_owned();
    for (name, value) in fields {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(name);
        text.push('=');
        text.push_str(&value.to_string());
    }
    text
}

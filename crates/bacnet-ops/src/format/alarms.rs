//! Alarm and event summaries as typed data and as table text (#2077): a
//! GetEventInformation answer, a GetAlarmSummary entry and a
//! GetEnrollmentSummary entry, with states, event types and notify types
//! named, transition bits named, timestamps rendered and objects as
//! objects. The CLI's tables and JSON, the terminal UI and the MCP server
//! share them.
//!
//! Each summary is built from [`Datum`] values, so its JSON uses the value
//! forms the CLI reference documents (`{"value": 3, "name":
//! "high-limit"}`, `{"bits": "011", "names": [...]}`), and its table cells
//! are written from the same data.

use std::collections::HashSet;
use std::future::Future;

use bacnet_services::alarm_event::{EventSummary, GetEventInformationAck};
use bacnet_services::alarm_summary::AlarmSummaryEntry;
use bacnet_services::enrollment_summary::EnrollmentSummaryEntry;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use serde::Serialize;

use crate::label::object_label;

use super::{Datum, Floats, StateTexts, ValueFormat, constructed};

/// One value for each of the three event transitions, in their wire
/// order: to-offnormal, to-fault, to-normal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transitions<T> {
    /// The to-offnormal transition's.
    pub to_offnormal: T,
    /// The to-fault transition's.
    pub to_fault: T,
    /// The to-normal transition's.
    pub to_normal: T,
}

impl<T> Transitions<T> {
    fn from_array([to_offnormal, to_fault, to_normal]: [T; 3]) -> Self {
        Self {
            to_offnormal,
            to_fault,
            to_normal,
        }
    }

    fn as_array(&self) -> [&T; 3] {
        [&self.to_offnormal, &self.to_fault, &self.to_normal]
    }
}

/// One object of a GetEventInformation answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventSummaryData {
    /// The object, a [`Datum::Object`].
    pub object: Datum,
    /// Its Event_State, named.
    pub event_state: Datum,
    /// Its Acked_Transitions, as named bits.
    pub acknowledged_transitions: Datum,
    /// Its Event_Enable, as named bits.
    pub event_enable: Datum,
    /// Its Notify_Type, named.
    pub notify_type: Datum,
    /// The time of each transition's last event, as `time-stamp`
    /// elements.
    pub event_timestamps: Transitions<Datum>,
    /// Each transition's notification priority, a number.
    pub event_priorities: Transitions<Datum>,
}

/// A GetEventInformation answer, or several pages of one joined.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventInformationData {
    /// The objects, in the device's order.
    pub summaries: Vec<EventSummaryData>,
    /// Whether the device holds more than these: its last answer said so,
    /// or a cap stopped the reading early.
    pub more_events: bool,
}

/// One entry of a GetAlarmSummary answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlarmSummaryData {
    /// The object, a [`Datum::Object`].
    pub object: Datum,
    /// Its event state, named.
    pub alarm_state: Datum,
    /// Its Acked_Transitions, as named bits.
    pub acknowledged_transitions: Datum,
}

/// One entry of a GetEnrollmentSummary answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnrollmentSummaryData {
    /// The object, a [`Datum::Object`].
    pub object: Datum,
    /// Its event algorithm, named.
    pub event_type: Datum,
    /// Its event state, named.
    pub event_state: Datum,
    /// The priority of its notifications, a number.
    pub priority: Datum,
    /// Its notification class, a number; [`Datum::Null`] when the device
    /// left it out.
    pub notification_class: Datum,
}

/// The formatter the summaries name values with: no object's state texts,
/// every digit.
fn named<R>(f: impl FnOnce(&ValueFormat<'_>) -> R) -> R {
    let texts = StateTexts::default();
    f(&ValueFormat::new(ObjectType::DEVICE, &texts, Floats::Exact))
}

fn object(format: &ValueFormat<'_>, object: ObjectIdentifier) -> Datum {
    format.element_data(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        &PropertyValue::ObjectIdentifier(object),
    )
}

fn enumerated(format: &ValueFormat<'_>, property: PropertyIdentifier, raw: u32) -> Datum {
    format.element_data(property, &PropertyValue::Enumerated(raw))
}

/// Transition bits as the property `property` names them.
fn transitions(
    format: &ValueFormat<'_>,
    property: PropertyIdentifier,
    bits: EventTransitionBits,
) -> Datum {
    let value = PropertyValue::BitString {
        unused_bits: 5,
        data: vec![bits.to_bacnet()],
    };
    format.element_data(property, &value)
}

pub(super) fn timestamp(stamp: &BACnetTimeStamp) -> Datum {
    Datum::Constructed {
        element: "time-stamp".into(),
        text: constructed::timestamp_text(stamp),
        fields: None,
    }
}

impl EventSummaryData {
    /// The column headings of [`Self::cells`].
    pub const HEADERS: [&'static str; 9] = [
        "Object",
        "State",
        "Acked",
        "Enabled",
        "Notify",
        "To-offnormal",
        "To-fault",
        "To-normal",
        "Priorities",
    ];

    /// One object of a GetEventInformation answer, decoded.
    pub fn new(summary: &EventSummary) -> Self {
        named(|format| Self {
            object: object(format, summary.object_identifier),
            event_state: enumerated(
                format,
                PropertyIdentifier::EVENT_STATE,
                summary.event_state.to_raw(),
            ),
            acknowledged_transitions: transitions(
                format,
                PropertyIdentifier::ACKED_TRANSITIONS,
                summary.acknowledged_transitions,
            ),
            event_enable: transitions(
                format,
                PropertyIdentifier::EVENT_ENABLE,
                summary.event_enable,
            ),
            notify_type: enumerated(
                format,
                PropertyIdentifier::NOTIFY_TYPE,
                summary.notify_type.to_raw(),
            ),
            event_timestamps: Transitions::from_array(
                summary.event_timestamps.each_ref().map(timestamp),
            ),
            event_priorities: Transitions::from_array(
                summary
                    .event_priorities
                    .map(|priority| Datum::Unsigned(u64::from(priority))),
            ),
        })
    }

    /// The table row, under [`Self::HEADERS`]: the priorities as
    /// `to-offnormal/to-fault/to-normal`, `3/5/200`.
    pub fn cells(&self) -> Vec<String> {
        let [offnormal, fault, normal] = self.event_timestamps.as_array().map(cell);
        let priorities = self.event_priorities.as_array().map(cell).join("/");
        vec![
            cell(&self.object),
            cell(&self.event_state),
            cell(&self.acknowledged_transitions),
            cell(&self.event_enable),
            cell(&self.notify_type),
            offnormal,
            fault,
            normal,
            priorities,
        ]
    }
}

impl EventInformationData {
    /// One GetEventInformation answer, decoded.
    pub fn new(ack: &GetEventInformationAck) -> Self {
        Self::from_summaries(&ack.list_of_event_summaries, ack.more_events)
    }

    /// Summaries read over one or more answers, decoded; `more_events`
    /// says whether the device holds more.
    pub fn from_summaries(summaries: &[EventSummary], more_events: bool) -> Self {
        Self {
            summaries: summaries.iter().map(EventSummaryData::new).collect(),
            more_events,
        }
    }
}

impl AlarmSummaryData {
    /// The column headings of [`Self::cells`].
    pub const HEADERS: [&'static str; 3] = ["Object", "State", "Acked"];

    /// One GetAlarmSummary entry, decoded.
    pub fn new(entry: &AlarmSummaryEntry) -> Self {
        named(|format| Self {
            object: object(format, entry.object_identifier),
            alarm_state: enumerated(
                format,
                PropertyIdentifier::EVENT_STATE,
                entry.alarm_state.to_raw(),
            ),
            acknowledged_transitions: transitions(
                format,
                PropertyIdentifier::ACKED_TRANSITIONS,
                entry.acknowledged_transitions,
            ),
        })
    }

    /// The table row, under [`Self::HEADERS`].
    pub fn cells(&self) -> Vec<String> {
        vec![
            cell(&self.object),
            cell(&self.alarm_state),
            cell(&self.acknowledged_transitions),
        ]
    }
}

impl EnrollmentSummaryData {
    /// The column headings of [`Self::cells`].
    pub const HEADERS: [&'static str; 5] = [
        "Object",
        "Event type",
        "State",
        "Priority",
        "Notification class",
    ];

    /// One GetEnrollmentSummary entry, decoded.
    pub fn new(entry: &EnrollmentSummaryEntry) -> Self {
        named(|format| Self {
            object: object(format, entry.object_identifier),
            event_type: enumerated(
                format,
                PropertyIdentifier::EVENT_TYPE,
                entry.event_type.to_raw(),
            ),
            event_state: enumerated(
                format,
                PropertyIdentifier::EVENT_STATE,
                entry.event_state.to_raw(),
            ),
            priority: Datum::Unsigned(u64::from(entry.priority)),
            notification_class: entry
                .notification_class
                .map_or(Datum::Null, |class| Datum::Unsigned(u64::from(class))),
        })
    }

    /// The table row, under [`Self::HEADERS`]; a missing notification
    /// class as `-`.
    pub fn cells(&self) -> Vec<String> {
        vec![
            cell(&self.object),
            cell(&self.event_type),
            cell(&self.event_state),
            cell(&self.priority),
            cell(&self.notification_class),
        ]
    }
}

/// A summary field as a table cell: an object as `analog-input 1`, an
/// enumeration by name (its number when it has none), bits by their set
/// names, comma separated to read apart from the table's bars, or `none`,
/// a timestamp as its text, and an absent value as `-`: the formatter's
/// text for each (a test pins them together).
pub(crate) fn cell(datum: &Datum) -> String {
    match datum {
        Datum::Null => "-".into(),
        Datum::Unsigned(n) => n.to_string(),
        Datum::Object {
            object_type,
            instance,
            ..
        } => format!("{object_type} {instance}"),
        Datum::Enumerated { value, name } => name.clone().unwrap_or_else(|| value.to_string()),
        Datum::Bits {
            names: Some(names), ..
        } if names.is_empty() => "none".into(),
        Datum::Bits {
            names: Some(names), ..
        } => names.join(", "),
        Datum::Bits { bits, .. } => format!("bits({bits})"),
        Datum::Constructed { text, .. } => text.clone(),
        Datum::Boolean(b) => b.to_string(),
        Datum::Signed(n) => n.to_string(),
        Datum::Real(v) => v.to_string(),
        Datum::Double(v) => v.to_string(),
        Datum::NonFinite { float } => float.clone(),
        Datum::Text(text) => text.clone(),
        Datum::Date { date } => date.clone(),
        Datum::Time { time } => time.clone(),
        Datum::Octets { octets } => format!("0x{octets}"),
        Datum::Raw { raw } => format!("[raw {raw}]"),
        Datum::List(items) => items.iter().map(cell).collect::<Vec<_>>().join(", "),
    }
}

/// Every summary a device holds, read one GetEventInformation answer at
/// a time: `fetch` asks for the objects after its argument (`None` for
/// the first answer), and the reading goes on while the device says more
/// remain, up to `max` summaries. It returns the summaries and whether
/// the device holds more than them.
///
/// A device that names an object twice, or says more remain but sends
/// nothing, would show duplicates or page forever, so either is an error.
pub async fn read_event_information<F, Fut>(
    max: usize,
    fetch: F,
) -> Result<(Vec<EventSummary>, bool), Error>
where
    F: FnMut(Option<ObjectIdentifier>) -> Fut,
    Fut: Future<Output = Result<GetEventInformationAck, Error>>,
{
    read_event_information_until(max, || false, fetch).await
}

/// [`read_event_information`], stopping early when `stop` says so: it is
/// asked before every answer after the first, and a stop returns the
/// summaries read so far with "more" set, so the caller can go on after the
/// last of them later (a call out of time, say).
pub async fn read_event_information_until<S, F, Fut>(
    max: usize,
    mut stop: S,
    mut fetch: F,
) -> Result<(Vec<EventSummary>, bool), Error>
where
    S: FnMut() -> bool,
    F: FnMut(Option<ObjectIdentifier>) -> Fut,
    Fut: Future<Output = Result<GetEventInformationAck, Error>>,
{
    let mut summaries: Vec<EventSummary> = Vec::new();
    let mut seen = HashSet::new();
    let mut after = None;
    loop {
        if after.is_some() && stop() {
            return Ok((summaries, true));
        }
        let ack = fetch(after).await?;
        for summary in ack.list_of_event_summaries {
            if !seen.insert(summary.object_identifier.encode()) {
                return Err(Error::Encoding(format!(
                    "GetEventInformation named {} again",
                    object_label(summary.object_identifier)
                )));
            }
            summaries.push(summary);
        }
        if summaries.len() >= max {
            let more = ack.more_events || summaries.len() > max;
            summaries.truncate(max);
            return Ok((summaries, more));
        }
        if !ack.more_events {
            return Ok((summaries, false));
        }
        // Every object is new, so the cursor moves unless the page was
        // empty.
        match summaries.last().map(|summary| summary.object_identifier) {
            Some(last) if after != Some(last) => after = Some(last),
            _ => {
                return Err(Error::decoding(
                    0,
                    "GetEventInformation said more events remain but sent no new object",
                ));
            }
        }
    }
}

#[cfg(test)]
#[path = "alarms_tests.rs"]
mod tests;

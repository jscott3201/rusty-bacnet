//! The alarm commands that read (#2077): `alarms` (GetEventInformation,
//! paged), `alarm-summary` (GetAlarmSummary) and `enrollment-summary`
//! (GetEnrollmentSummary), decoded and named by the shared renderer
//! (`bacnet_ops::format::alarms`).

use bacnet_client::client::BACnetClient;
use bacnet_ops::destination::Destination;
use bacnet_ops::format::alarms::{
    AlarmSummaryData, EnrollmentSummaryData, EventInformationData, EventSummaryData,
    read_event_information,
};
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryRequest, PriorityFilter};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{AcknowledgmentFilter, EnrollmentSummaryEventStateFilter, EventType};
use serde::Serialize;

use crate::output::{OutputFormat, screen};

/// Summaries `alarms` reads before it stops, unless `--max` says otherwise.
pub const DEFAULT_MAX_EVENTS: u32 = 10_000;

/// The value named `word` in a `bacnet_enum` table, in any case, with `-`
/// or `_` between words: `high-limit`, `HIGH_LIMIT`.
fn by_name<T: Copy>(table: &[(&str, T)], word: &str, what: &str) -> Result<T, String> {
    let wanted = word.to_ascii_uppercase().replace('-', "_");
    table
        .iter()
        .find(|(name, _)| *name == wanted)
        .map(|&(_, value)| value)
        .ok_or_else(|| {
            let names: Vec<String> = table
                .iter()
                .map(|(name, _)| bacnet_ops::label::kebab(name))
                .collect();
            format!(
                "unknown {what} '{word}': expected one of {}",
                names.join(", ")
            )
        })
}

/// `--acked`: `all`, `acked` or `not-acked`.
pub fn parse_ack_filter(word: &str) -> Result<AcknowledgmentFilter, String> {
    by_name(
        AcknowledgmentFilter::ALL_NAMED,
        word,
        "acknowledgment filter",
    )
}

/// `--state`: `offnormal`, `fault`, `normal`, `all` or `active`.
pub fn parse_state_filter(word: &str) -> Result<EnrollmentSummaryEventStateFilter, String> {
    by_name(
        EnrollmentSummaryEventStateFilter::ALL_NAMED,
        word,
        "event state filter",
    )
}

/// `--event-type`: an event algorithm by name, `out-of-range`.
pub fn parse_event_type(word: &str) -> Result<EventType, String> {
    by_name(EventType::ALL_NAMED, word, "event type")
}

/// `--priority`: `MIN-MAX`, each 0 to 255, or one priority alone.
pub fn parse_priority_range(text: &str) -> Result<PriorityFilter, String> {
    let number = |part: &str| {
        part.trim()
            .parse::<u8>()
            .map_err(|_| format!("invalid priority '{part}': expected 0 to 255"))
    };
    let (min, max) = match text.split_once('-') {
        Some((min, max)) => (number(min)?, number(max)?),
        None => {
            let one = number(text)?;
            (one, one)
        }
    };
    if min > max {
        return Err(format!("priority range '{text}' runs backward"));
    }
    Ok(PriorityFilter {
        min_priority: min,
        max_priority: max,
    })
}

/// The `enrollment-summary` filters, the ones the service defines.
#[derive(Debug, Clone, clap::Args)]
pub struct EnrollmentFilters {
    /// Acknowledgment: all, acked or not-acked.
    #[arg(long = "acked", value_name = "ACKED", default_value = "all", value_parser = parse_ack_filter)]
    pub acknowledgment: AcknowledgmentFilter,
    /// Event state: offnormal, fault, normal, all or active.
    #[arg(long = "state", value_name = "STATE", value_parser = parse_state_filter)]
    pub event_state: Option<EnrollmentSummaryEventStateFilter>,
    /// Event algorithm, such as out-of-range or change-of-state.
    #[arg(long, value_parser = parse_event_type)]
    pub event_type: Option<EventType>,
    /// Notification priority, MIN-MAX (0 to 255) or one value.
    #[arg(long, value_parser = parse_priority_range)]
    pub priority: Option<PriorityFilter>,
    /// Notification class number.
    #[arg(long = "class", value_name = "CLASS")]
    pub notification_class: Option<u32>,
}

impl EnrollmentFilters {
    fn request(&self) -> GetEnrollmentSummaryRequest {
        GetEnrollmentSummaryRequest {
            acknowledgment_filter: self.acknowledgment,
            enrollment_filter: None,
            event_state_filter: self.event_state,
            event_type_filter: self.event_type,
            priority_filter: self.priority,
            notification_class_filter: self.notification_class,
        }
    }
}

/// A table of `rows` under `headers`, each cell [`screen`]ed.
fn table<const N: usize>(headers: [&str; N], rows: impl Iterator<Item = Vec<String>>) -> String {
    let mut table = comfy_table::Table::new();
    table.set_header(headers.to_vec());
    for row in rows {
        table.add_row(row.iter().map(|cell| screen(cell)).collect::<Vec<_>>());
    }
    table.to_string()
}

fn print_json(value: &impl Serialize) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serialize alarm output")
    );
}

/// The `alarms` text: a table and a count, or a line saying there is
/// nothing to show, and a note when more remain.
pub fn event_information_text(data: &EventInformationData, max: u32) -> String {
    let mut text = if data.summaries.is_empty() {
        "No active events or unacknowledged transitions.".to_string()
    } else {
        format!(
            "{}\n{} object(s) with an active event or an unacknowledged transition",
            table(
                EventSummaryData::HEADERS,
                data.summaries.iter().map(EventSummaryData::cells)
            ),
            data.summaries.len()
        )
    };
    if data.more_events {
        text.push_str(&format!("\nThe device holds more; stopped at --max {max}."));
    }
    text
}

/// Read every GetEventInformation answer of a device, up to `max`
/// summaries, and print them.
pub async fn alarms_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    max: u32,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let cap = usize::try_from(max).unwrap_or(usize::MAX);
    let (summaries, more_events) =
        read_event_information(cap, |after| to.get_event_information(client, after)).await?;
    let data = EventInformationData::from_summaries(&summaries, more_events);
    match format {
        OutputFormat::Table => println!("{}", event_information_text(&data, max)),
        OutputFormat::Json => print_json(&data),
    }
    Ok(())
}

/// The `alarm-summary` text.
pub fn alarm_summary_text(entries: &[AlarmSummaryData]) -> String {
    if entries.is_empty() {
        return "No objects in alarm.".into();
    }
    format!(
        "{}\n{} object(s) in alarm",
        table(
            AlarmSummaryData::HEADERS,
            entries.iter().map(AlarmSummaryData::cells)
        ),
        entries.len()
    )
}

/// GetAlarmSummary: the objects in alarm, printed.
pub async fn alarm_summary_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let ack = to.get_alarm_summary(client).await?;
    let entries: Vec<AlarmSummaryData> = ack.entries.iter().map(AlarmSummaryData::new).collect();
    match format {
        OutputFormat::Table => println!("{}", alarm_summary_text(&entries)),
        OutputFormat::Json => print_json(&entries),
    }
    Ok(())
}

/// The `enrollment-summary` text.
pub fn enrollment_summary_text(entries: &[EnrollmentSummaryData]) -> String {
    if entries.is_empty() {
        return "No event enrollments match.".into();
    }
    format!(
        "{}\n{} enrollment(s)",
        table(
            EnrollmentSummaryData::HEADERS,
            entries.iter().map(EnrollmentSummaryData::cells)
        ),
        entries.len()
    )
}

/// GetEnrollmentSummary with `filters`: the matching event-initiating
/// objects, printed.
pub async fn enrollment_summary_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    filters: &EnrollmentFilters,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let ack = to
        .get_enrollment_summary(client, &filters.request())
        .await?;
    let entries: Vec<EnrollmentSummaryData> =
        ack.entries.iter().map(EnrollmentSummaryData::new).collect();
    match format {
        OutputFormat::Table => println!("{}", enrollment_summary_text(&entries)),
        OutputFormat::Json => print_json(&entries),
    }
    Ok(())
}

#[cfg(test)]
#[path = "alarms_tests.rs"]
mod tests;

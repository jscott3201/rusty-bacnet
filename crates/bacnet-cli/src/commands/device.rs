//! Device management commands: DeviceCommunicationControl, ReinitializeDevice,
//! AcknowledgeAlarm, CreateObject, DeleteObject, TimeSync.

use bacnet_client::client::BACnetClient;
use bacnet_ops::destination::Destination;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{EnableDisable, EventState, ObjectType, ReinitializedState};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};

use crate::output::{self, OutputFormat};

/// Synchronize time with a remote device.
pub async fn time_sync_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    utc: bool,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("system time error: {e}"))?;
    let secs = now.as_secs();

    // Convert epoch seconds to date/time components.
    // Days since 1970-01-01.
    let days = secs / 86400;
    let day_secs = (secs % 86400) as u32;

    let hour = (day_secs / 3600) as u8;
    let minute = ((day_secs % 3600) / 60) as u8;
    let second = (day_secs % 60) as u8;
    let hundredths = ((now.subsec_millis() / 10) % 100) as u8;

    // Civil date from days since epoch (algorithm from Howard Hinnant).
    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    let y = if m <= 2 { y + 1 } else { y };

    // Day of week: 1970-01-01 was Thursday (BACnet: 4).
    let dow = ((days + 3) % 7 + 1) as u8; // 1=Monday..7=Sunday

    let date = bacnet_types::primitives::Date {
        year: (y - 1900) as u8,
        month: m,
        day: d,
        day_of_week: dow,
    };
    let time = bacnet_types::primitives::Time {
        hour,
        minute,
        second,
        hundredths,
    };

    // Both choices retain the existing UTC-derived fields; there is no
    // timezone library here to calculate local civil time.
    to.time_synchronization(client, date, time, utc).await?;
    output::print_success("Time synchronized", format);
    Ok(())
}

/// The DeviceCommunicationControl actions by name, as `control` (`dcc`)
/// and the TUI's `:dcc` take them.
pub(crate) const DCC_ACTIONS: [(&str, EnableDisable); 3] = [
    ("enable", EnableDisable::ENABLE),
    ("disable", EnableDisable::DISABLE),
    ("disable-initiation", EnableDisable::DISABLE_INITIATION),
];

/// The ReinitializeDevice states by name, as `reinit` and the TUI's
/// `:reinit` take them.
pub(crate) const REINIT_STATES: [(&str, ReinitializedState); 8] = [
    ("coldstart", ReinitializedState::COLDSTART),
    ("warmstart", ReinitializedState::WARMSTART),
    ("start-backup", ReinitializedState::START_BACKUP),
    ("end-backup", ReinitializedState::END_BACKUP),
    ("start-restore", ReinitializedState::START_RESTORE),
    ("end-restore", ReinitializedState::END_RESTORE),
    ("abort-restore", ReinitializedState::ABORT_RESTORE),
    ("activate-changes", ReinitializedState::ACTIVATE_CHANGES),
];

/// The value named `word` in `table`, in any case, with `_` for `-`.
fn named<T: Copy>(table: &[(&str, T)], word: &str) -> Option<T> {
    let word = word.to_ascii_lowercase().replace('_', "-");
    table
        .iter()
        .find(|(name, _)| *name == word)
        .map(|(_, value)| *value)
}

/// `'a', 'b', or 'c'`.
fn choices<T>(table: &[(&str, T)]) -> String {
    let names: Vec<String> = table.iter().map(|(name, _)| format!("'{name}'")).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{}, or {last}", rest.join(", ")),
        _ => names.join(""),
    }
}

/// Parse an action string into an `EnableDisable` value.
pub(crate) fn parse_enable_disable(action: &str) -> Result<EnableDisable, String> {
    named(&DCC_ACTIONS, action).ok_or_else(|| {
        format!(
            "unknown action '{action}': expected {}",
            choices(&DCC_ACTIONS)
        )
    })
}

/// Parse a state string into a `ReinitializedState` value.
pub(crate) fn parse_reinit_state(state: &str) -> Result<ReinitializedState, String> {
    named(&REINIT_STATES, state).ok_or_else(|| {
        format!(
            "unknown state '{state}': expected {}",
            choices(&REINIT_STATES)
        )
    })
}

/// Send a DeviceCommunicationControl request.
pub async fn control_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    action: &str,
    duration: Option<u16>,
    password: Option<&str>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let enable_disable = parse_enable_disable(action)?;

    to.device_communication_control(
        client,
        enable_disable,
        duration,
        password.map(|s| s.to_string()),
    )
    .await?;

    output::print_success("OK", format);
    Ok(())
}

/// Send a ReinitializeDevice request.
pub async fn reinit_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    state: &str,
    password: Option<&str>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let reinit_state = parse_reinit_state(state)?;

    to.reinitialize_device(client, reinit_state, password.map(|s| s.to_string()))
        .await?;

    output::print_success("OK", format);
    Ok(())
}

fn build_acknowledge_alarm_request(
    process_id: u32,
    object_type: ObjectType,
    instance: u32,
    event_state: EventState,
    source: &str,
    timestamp: BACnetTimeStamp,
    time_of_acknowledgment: BACnetTimeStamp,
) -> Result<AcknowledgeAlarmRequest, bacnet_types::error::Error> {
    Ok(AcknowledgeAlarmRequest {
        acknowledging_process_identifier: process_id,
        event_object_identifier: ObjectIdentifier::new(object_type, instance)?,
        event_state_acknowledged: event_state,
        timestamp,
        acknowledgment_source: source.to_string(),
        time_of_acknowledgment,
    })
}

/// The alarm being acknowledged and the acknowledgment metadata.
pub struct AcknowledgeAlarmArgs<'a> {
    /// Type of the object whose alarm is acknowledged.
    pub object_type: ObjectType,
    /// Instance number of the object whose alarm is acknowledged.
    pub instance: u32,
    /// Event state being acknowledged.
    pub event_state: EventState,
    /// Free-text source of the acknowledgment.
    pub source: &'a str,
    /// Timestamp of the event transition being acknowledged.
    pub timestamp: BACnetTimeStamp,
    /// Timestamp recorded as the time of acknowledgment.
    pub time_of_acknowledgment: BACnetTimeStamp,
}

/// Acknowledge an alarm on a remote device with exact caller-supplied timestamps.
pub async fn acknowledge_alarm_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    args: AcknowledgeAlarmArgs<'_>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let AcknowledgeAlarmArgs {
        object_type,
        instance,
        event_state,
        source,
        timestamp,
        time_of_acknowledgment,
    } = args;
    // Use PID as process identifier
    let process_id = std::process::id();
    let request = build_acknowledge_alarm_request(
        process_id,
        object_type,
        instance,
        event_state,
        source,
        timestamp,
        time_of_acknowledgment,
    )?;

    to.acknowledge_alarm_request(client, &request).await?;

    output::print_success("Alarm acknowledged", format);
    Ok(())
}

/// Delete an object on a remote device.
pub async fn delete_object_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    object_type: ObjectType,
    instance: u32,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    to.delete_object(client, oid).await?;
    output::print_success(&format!("Deleted {}:{}", object_type, instance), format);
    Ok(())
}

/// Create an object on a remote device.
pub async fn create_object_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    object_type: ObjectType,
    instance: u32,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    use bacnet_services::object_mgmt::ObjectSpecifier;
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let specifier = ObjectSpecifier::Identifier(oid);
    // The client decodes the ACK; one that doesn't decode is an error, sent
    // after the device acted on the request.
    let created = to.create_object(client, specifier, vec![]).await.map_err(
        |error| -> Box<dyn std::error::Error> {
            match error {
                bacnet_types::error::Error::Decoding(_) => format!(
                    "{error}; the CreateObject acknowledgment didn't decode, \
                     but the device may have created the object"
                )
                .into(),
                other => other.into(),
            }
        },
    )?;
    let (message, json) = created_object_report(created);
    match format {
        OutputFormat::Table => output::print_success(&message, format),
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json).expect("serialize create-object result")
        ),
    }
    Ok(())
}

/// Render the object a CreateObject acknowledgment names as a text message
/// and a JSON object: the identifier is what a user needs, since the server
/// may have picked the instance.
fn created_object_report(created: ObjectIdentifier) -> (String, serde_json::Value) {
    let message = format!(
        "Created {}:{}",
        created.object_type(),
        created.instance_number()
    );
    let json = serde_json::json!({
        "status": "ok",
        "message": message,
        "object_type": created.object_type().to_string(),
        "instance": created.instance_number(),
    });
    (message, json)
}

#[cfg(test)]
mod create_object_report_tests {
    use super::*;

    #[test]
    fn report_names_the_new_object() {
        let created = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 100).unwrap();
        let (message, json) = created_object_report(created);
        assert_eq!(message, "Created BINARY_INPUT:100");
        assert_eq!(
            json,
            serde_json::json!({
                "status": "ok",
                "message": "Created BINARY_INPUT:100",
                "object_type": "BINARY_INPUT",
                "instance": 100,
            })
        );
    }
}

#[cfg(test)]
mod acknowledgment_tests {
    use super::*;
    use bacnet_types::primitives::{Date, Time};

    #[test]
    fn request_builder_preserves_both_timestamps_and_all_metadata() {
        let event_timestamp = BACnetTimeStamp::SequenceNumber(65_535);
        let acknowledgment_timestamp = BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 14,
                day: 34,
                day_of_week: 255,
            },
            time: Time {
                hour: 23,
                minute: 59,
                second: 58,
                hundredths: 99,
            },
        };
        let request = build_acknowledge_alarm_request(
            0x1234_5678,
            ObjectType::ANALOG_INPUT,
            77,
            EventState::HIGH_LIMIT,
            "operator-console",
            event_timestamp.clone(),
            acknowledgment_timestamp.clone(),
        )
        .unwrap();

        assert_eq!(request.acknowledging_process_identifier, 0x1234_5678);
        assert_eq!(
            request.event_object_identifier.object_type(),
            ObjectType::ANALOG_INPUT
        );
        assert_eq!(request.event_object_identifier.instance_number(), 77);
        assert_eq!(request.event_state_acknowledged, EventState::HIGH_LIMIT);
        assert_eq!(request.timestamp, event_timestamp);
        assert_eq!(request.acknowledgment_source, "operator-console");
        assert_eq!(request.time_of_acknowledgment, acknowledgment_timestamp);
    }
}

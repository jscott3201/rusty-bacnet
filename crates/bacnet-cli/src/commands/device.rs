//! Device management commands: DeviceCommunicationControl, ReinitializeDevice, GetEventInformation,
//! AcknowledgeAlarm, CreateObject, DeleteObject, TimeSync.

use bacnet_client::client::BACnetClient;
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{EnableDisable, ObjectType, ReinitializedState};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};

use crate::output::{self, OutputFormat};

/// Synchronize time with a remote device.
pub async fn time_sync_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
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

    if utc {
        client.utc_time_synchronization(mac, date, time).await?;
    } else {
        // For local time sync we also use UTC since we don't have
        // a timezone library. Document this limitation.
        client.time_synchronization(mac, date, time).await?;
    }
    output::print_success("Time synchronized", format);
    Ok(())
}

/// Parse an action string into an `EnableDisable` value.
fn parse_enable_disable(action: &str) -> Result<EnableDisable, String> {
    match action.to_ascii_lowercase().as_str() {
        "enable" => Ok(EnableDisable::ENABLE),
        "disable" => Ok(EnableDisable::DISABLE),
        "disable-initiation" | "disable_initiation" => Ok(EnableDisable::DISABLE_INITIATION),
        _ => Err(format!(
            "unknown action '{action}': expected 'enable', 'disable', or 'disable-initiation'"
        )),
    }
}

/// Parse a state string into a `ReinitializedState` value.
fn parse_reinit_state(state: &str) -> Result<ReinitializedState, String> {
    match state.to_ascii_lowercase().as_str() {
        "coldstart" => Ok(ReinitializedState::COLDSTART),
        "warmstart" => Ok(ReinitializedState::WARMSTART),
        "start-backup" | "start_backup" => Ok(ReinitializedState::START_BACKUP),
        "end-backup" | "end_backup" => Ok(ReinitializedState::END_BACKUP),
        "start-restore" | "start_restore" => Ok(ReinitializedState::START_RESTORE),
        "end-restore" | "end_restore" => Ok(ReinitializedState::END_RESTORE),
        "abort-restore" | "abort_restore" => Ok(ReinitializedState::ABORT_RESTORE),
        "activate-changes" | "activate_changes" => Ok(ReinitializedState::ACTIVATE_CHANGES),
        _ => Err(format!(
            "unknown state '{state}': expected 'coldstart', 'warmstart', 'start-backup', \
             'end-backup', 'start-restore', 'end-restore', 'abort-restore', or 'activate-changes'"
        )),
    }
}

/// Send a DeviceCommunicationControl request.
pub async fn control_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    action: &str,
    duration: Option<u16>,
    password: Option<&str>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let enable_disable = parse_enable_disable(action)?;

    client
        .device_communication_control(
            mac,
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
    mac: &[u8],
    state: &str,
    password: Option<&str>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let reinit_state = parse_reinit_state(state)?;

    client
        .reinitialize_device(mac, reinit_state, password.map(|s| s.to_string()))
        .await?;

    output::print_success("OK", format);
    Ok(())
}

/// Get event/alarm information from a device.
pub async fn alarms_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client.get_event_information(mac, None).await?;

    // TODO: Decode GetEventInformation-ACK properly.
    let hex: String = response
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    output::print_success(
        &format!(
            "GetEventInformation response ({} bytes, raw — decoding not yet implemented):\n{hex}",
            response.len()
        ),
        format,
    );
    Ok(())
}

fn build_acknowledge_alarm_request(
    process_id: u32,
    object_type: ObjectType,
    instance: u32,
    event_state: u32,
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

/// Acknowledge an alarm on a remote device with exact caller-supplied timestamps.
#[allow(clippy::too_many_arguments)]
pub async fn acknowledge_alarm_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    event_state: u32,
    source: &str,
    timestamp: BACnetTimeStamp,
    time_of_acknowledgment: BACnetTimeStamp,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
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

    client.acknowledge_alarm_request(mac, &request).await?;

    output::print_success("Alarm acknowledged", format);
    Ok(())
}

/// Delete an object on a remote device.
pub async fn delete_object_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    client.delete_object(mac, oid).await?;
    output::print_success(&format!("Deleted {}:{}", object_type, instance), format);
    Ok(())
}

/// Create an object on a remote device.
pub async fn create_object_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    use bacnet_services::object_mgmt::ObjectSpecifier;
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let specifier = ObjectSpecifier::Identifier(oid);
    let response = client.create_object(mac, specifier, vec![]).await?;
    let hex: String = response
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    output::print_success(&format!("Created object (response: {hex})"), format);
    Ok(())
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
            3,
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
        assert_eq!(request.event_state_acknowledged, 3);
        assert_eq!(request.timestamp, event_timestamp);
        assert_eq!(request.acknowledgment_source, "operator-console");
        assert_eq!(request.time_of_acknowledgment, acknowledgment_timestamp);
    }
}

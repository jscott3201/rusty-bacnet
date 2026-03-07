//! Device management commands: DeviceCommunicationControl, ReinitializeDevice, GetEventInformation,
//! AcknowledgeAlarm, CreateObject, DeleteObject, TimeSync.

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{EnableDisable, ObjectType, ReinitializedState};
use bacnet_types::primitives::ObjectIdentifier;

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

/// Acknowledge an alarm on a remote device.
pub async fn acknowledge_alarm_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    event_state: u32,
    source: &str,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    // Use PID as process identifier
    let process_id = std::process::id();

    client
        .acknowledge_alarm(mac, process_id, oid, event_state, source)
        .await?;

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

//! COV subscription command.

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use crate::output::{self, OutputFormat};

/// Subscribe to COV notifications for an object on a remote device.
pub async fn subscribe_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    mac: &[u8],
    object_type: ObjectType,
    instance: u32,
    lifetime: Option<u32>,
    confirmed: bool,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(object_type, instance)?;
    let process_id = std::process::id(); // Use PID instead of hardcoded 1

    client
        .subscribe_cov(mac, process_id, oid, confirmed, lifetime)
        .await?;

    output::print_success(
        &format!(
            "Subscribed to {}:{} (process_id={}). Watching... (Ctrl+C to stop)",
            object_type, instance, process_id
        ),
        format,
    );

    let mut rx = client.cov_notifications();
    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(notification) => {
                        print_cov_notification(&notification, format);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        output::print_error(&format!("Missed {n} notifications (buffer overflow)"));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        output::print_error("Notification channel closed");
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }

    Ok(())
}

fn print_cov_notification(
    notification: &bacnet_services::cov::COVNotificationRequest,
    format: OutputFormat,
) {
    let obj = format!(
        "{}:{}",
        notification.monitored_object_identifier.object_type(),
        notification.monitored_object_identifier.instance_number()
    );
    let device = format!(
        "device:{}",
        notification.initiating_device_identifier.instance_number()
    );

    // Decode property values
    let mut props = Vec::new();
    for pv in &notification.list_of_values {
        let prop_name = format!("{}", pv.property_identifier);
        let value_str = decode_cov_value(&pv.value);
        props.push((prop_name, value_str));
    }

    match format {
        OutputFormat::Table => {
            let mut parts = vec![format!(
                "[COV] {obj} from {device}, remaining={}s",
                notification.time_remaining
            )];
            for (prop, val) in &props {
                parts.push(format!("  {prop} = {val}"));
            }
            println!("{}", parts.join("\n"));
        }
        OutputFormat::Json => {
            let prop_map: serde_json::Map<String, serde_json::Value> = props
                .into_iter()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect();
            let json = serde_json::json!({
                "type": "cov",
                "object": obj,
                "device": device,
                "time_remaining": notification.time_remaining,
                "values": prop_map,
            });
            println!("{}", serde_json::to_string(&json).unwrap_or_default());
        }
    }
}

fn decode_cov_value(data: &[u8]) -> String {
    let mut offset = 0;
    let mut values = Vec::new();
    while offset < data.len() {
        match decode_application_value(data, offset) {
            Ok((value, next)) => {
                values.push(crate::output::format_property_value(&value));
                offset = next;
            }
            Err(_) => {
                let hex: String = data[offset..]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                values.push(format!("[raw: {hex}]"));
                break;
            }
        }
    }
    values.join(", ")
}

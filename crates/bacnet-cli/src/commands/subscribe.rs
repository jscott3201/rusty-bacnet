//! COV subscription command.

use bacnet_client::client::{BACnetClient, COVNotificationDelivery, ReceivedCOVNotification};
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

fn print_cov_notification(received: &ReceivedCOVNotification, format: OutputFormat) {
    let notification = &received.notification;
    let obj = format!(
        "{}:{}",
        notification.monitored_object_identifier.object_type(),
        notification.monitored_object_identifier.instance_number()
    );
    let device = format!(
        "device:{}",
        notification.initiating_device_identifier.instance_number()
    );
    let delivery = match received.delivery {
        COVNotificationDelivery::Confirmed => "confirmed",
        COVNotificationDelivery::Unconfirmed => "unconfirmed",
    };
    let source_mac = format_mac(&received.source_mac);
    let source_address = received
        .source_address
        .as_ref()
        .map(|address| format_mac(address));
    let routed_source = received
        .source_network
        .zip(source_address.as_ref())
        .map(|(network, address)| format!("{network}:{address}"));

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
                "[COV] {obj} from {device}, source={source_mac}, delivery={delivery}, remaining={}s",
                notification.time_remaining,
            )];
            if let Some(routed_source) = &routed_source {
                parts.push(format!("  routed-source = {routed_source}"));
            }
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
                "delivery": delivery,
                "source_mac": source_mac,
                "source_network": received.source_network,
                "source_address": source_address,
                "time_remaining": notification.time_remaining,
                "values": prop_map,
            });
            println!("{}", serde_json::to_string(&json).unwrap_or_default());
        }
    }
}

/// A MAC as hex, except a B/IPv6 VMAC, which prints as the `bip6:` target
/// the CLI reads back. Six octets stay hex: on BACnet/SC they are a VMAC,
/// not an IPv4 address and port.
fn format_mac(mac: &[u8]) -> String {
    if let Ok(vmac) = <[u8; 3]>::try_from(mac) {
        return bacnet_transport::bip6::format_vmac_text(bacnet_transport::bip6::Bip6Vmac::new(
            vmac,
        ));
    }
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
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

#[cfg(test)]
mod tests {
    use super::format_mac;
    use crate::resolve::{parse_target, Target};

    /// A COV source on B/IPv6 prints as the `bip6:` VMAC a target takes back;
    /// other MACs print as hex.
    #[test]
    fn cov_sources_print_b_ipv6_vmacs_as_targets() {
        let vmac = [0x00, 0x04, 0xD2];
        let printed = format_mac(&vmac);
        assert_eq!(printed, "bip6:0x0004D2");
        match parse_target(&printed) {
            Ok(Target::Mac(mac)) => assert_eq!(mac, vmac),
            _ => panic!("{printed} does not read back"),
        }
        assert_eq!(format_mac(&[2, 0, 0, 0, 0, 1]), "02:00:00:00:00:01");
        assert_eq!(format_mac(&[7]), "07");
    }
}

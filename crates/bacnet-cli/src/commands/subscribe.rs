//! COV subscription command.
//!
//! A subscription is finite by default (300 seconds), renewed while the
//! command watches and cancelled when it stops, so it leaves nothing behind
//! on the device. A lifetime of 0 asks for one that never expires (Clause
//! 13.14.1.1.4); it stays after the CLI exits, which is why the write guard
//! treats it as a write (`bacnet_tool::class`).

use std::future::Future;
use std::time::Duration;

use crate::destination::Destination;
use bacnet_client::client::{
    BACnetClient, COVEvent, COVNotificationDelivery, ReceivedCOVNotification,
};
use bacnet_ops::target::routed_station;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::output::{self, OutputFormat};

/// The lifetime a subscription asks for when `--lifetime` is not given.
pub(crate) const DEFAULT_LIFETIME: u32 = 300;

/// How long before a finite subscription runs out the command renews it.
const RENEWAL_MARGIN: u32 = 30;

/// When to renew a subscription of `lifetime` seconds: `RENEWAL_MARGIN`
/// before it runs out, but never before half of it has passed, and at least
/// a second apart. `None` for an indefinite one (0), which needs none.
pub(crate) fn renewal_interval(lifetime: u32) -> Option<Duration> {
    (lifetime != 0).then(|| {
        let seconds = lifetime
            .saturating_sub(RENEWAL_MARGIN)
            .max(lifetime / 2)
            .max(1);
        Duration::from_secs(u64::from(seconds))
    })
}

/// What to subscribe to.
#[derive(Debug, Clone, Copy)]
pub struct Subscription {
    /// The object's type.
    pub object_type: ObjectType,
    /// The object's instance.
    pub instance: u32,
    /// Seconds; 0 for a subscription that never expires.
    pub lifetime: u32,
    /// Ask for confirmed notifications.
    pub confirmed: bool,
}

/// Ctrl+C, or on Unix also SIGTERM: what ends `subscribe`'s watch. On
/// Unix both handlers are installed when this is called, before the
/// subscription goes out, so an early signal still cancels it.
#[cfg(unix)]
pub(crate) fn stop_signal() -> impl Future<Output = ()> {
    use tokio::signal::unix::{SignalKind, signal};
    let interrupt = signal(SignalKind::interrupt());
    let terminate = signal(SignalKind::terminate());
    async move {
        let (Ok(mut interrupt), Ok(mut terminate)) = (interrupt, terminate) else {
            // No handler: stop at once rather than leave one we can't end.
            return;
        };
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    }
}

/// Ctrl+C, or on Unix also SIGTERM: what ends `subscribe`'s watch.
#[cfg(not(unix))]
pub(crate) async fn stop_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Subscribe to COV notifications for an object on a remote device and
/// print them until `stop` completes. A finite subscription is renewed
/// meanwhile and cancelled at the end, however the watch ends.
///
/// Once the subscription is in place stdout is a stream (#2783): the
/// opening line, then a line a notification, JSON Lines in JSON. A failure
/// after that (a renewal, the cancel, stdout itself) ends the stream with an
/// error line; a refused subscription prints nothing.
pub async fn subscribe_cmd<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    subscription: Subscription,
    format: OutputFormat,
    stop: impl Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Subscription {
        object_type,
        instance,
        lifetime,
        confirmed,
    } = subscription;
    let held = Held {
        process_id: std::process::id(), // Use PID instead of hardcoded 1
        oid: ObjectIdentifier::new(object_type, instance)?,
        confirmed,
        lifetime,
    };

    let rx = client.cov_notifications();
    held.subscribe(client, to).await?;

    let watched = watch(client, to, held, rx, format, stop).await;
    let ended = if lifetime != 0
        && let Err(error) = to.unsubscribe_cov(client, held.process_id, held.oid).await
    {
        let message = format!("cancelling the subscription failed: {error}");
        Err(match watched {
            Ok(_) => message,
            Err(watch) => format!("{watch}; {message}"),
        }
        .into())
    } else {
        watched
    };
    let last = match &ended {
        Ok(note) => note.map(str::to_owned),
        Err(error) => Some(crate::error_text(&**error)),
    };
    if let Some(message) = last {
        output::end_stream(&message, format);
    }
    ended.map(|_| ())
}

/// The subscription the command holds.
#[derive(Debug, Clone, Copy)]
struct Held {
    process_id: u32,
    oid: ObjectIdentifier,
    confirmed: bool,
    /// Seconds; 0 for one that never expires.
    lifetime: u32,
}

impl Held {
    /// Send its SubscribeCOV: the first, or a renewal.
    async fn subscribe<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        to: &Destination,
    ) -> Result<(), bacnet_types::error::Error> {
        // Absent with Issue Confirmed Notifications present is indefinite
        // too (Clause 13.14.2), and is what the CLI has always sent for it.
        let lifetime = (self.lifetime != 0).then_some(self.lifetime);
        to.subscribe_cov(client, self.process_id, self.oid, self.confirmed, lifetime)
            .await
    }
}

/// How a watch ended: `Ok(None)` when stopped; `Ok(Some(note))` on a note
/// stderr already has that leaves the exit status 0; or the error.
type Watched = Result<Option<&'static str>, Box<dyn std::error::Error>>;

/// What stderr says when the client's notifications end.
const CHANNEL_CLOSED: &str = "Notification channel closed";

/// Print the opening line, then each notification until `stop` completes,
/// renewing a finite subscription on the way.
async fn watch<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    held: Held,
    mut rx: broadcast::Receiver<COVEvent>,
    format: OutputFormat,
    stop: impl Future<Output = ()>,
) -> Watched {
    output::print_line(&opening_text(&held, format)).map_err(output::stdout_failed)?;
    let renew_every = renewal_interval(held.lifetime);
    let mut renew_at = renew_every.map(|every| tokio::time::Instant::now() + every);
    tokio::pin!(stop);
    loop {
        let renew = async {
            match renew_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            () = &mut stop => return Ok(None),
            () = renew => {
                if let Err(error) = held.subscribe(client, to).await {
                    return Err(format!("renewing the subscription failed: {error}").into());
                }
                renew_at = renew_every.map(|every| tokio::time::Instant::now() + every);
            }
            result = rx.recv() => match result {
                Ok(COVEvent::Object(notification)) => {
                    output::print_line(&cov_notification_text(&notification, format))
                        .map_err(output::stdout_failed)?;
                }
                // A COV-multiple notification answers a
                // SubscribeCOVPropertyMultiple, which this command never
                // sends: nothing it watches.
                Ok(COVEvent::Multiple(_)) => {}
                Err(RecvError::Lagged(n)) => {
                    output::print_error(&format!("Missed {n} notifications (buffer overflow)"));
                }
                Err(RecvError::Closed) => {
                    output::print_error(CHANNEL_CLOSED);
                    return Ok(Some(CHANNEL_CLOSED));
                }
            },
        }
    }
}

/// The opening line, once the subscription is in place: the status line,
/// or in JSON `{"type": "subscribed", "status": "ok", "message"}`.
fn opening_text(held: &Held, format: OutputFormat) -> String {
    let kept = match held.lifetime {
        0 => "indefinite: it stays after the CLI exits".to_string(),
        lifetime => format!("lifetime {lifetime}s, renewed while watching, cancelled on exit"),
    };
    let message = format!(
        "Subscribed to {}:{} (process_id={}; {kept}). Watching... (Ctrl+C to stop)",
        held.oid.object_type(),
        held.oid.instance_number(),
        held.process_id
    );
    match format {
        OutputFormat::Table => output::success_text(&message, format),
        OutputFormat::Json => output::json_line(&serde_json::json!({
            "type": "subscribed",
            "status": "ok",
            "message": message,
        })),
    }
}

/// A COV notification as printed in `format`. A routed source prints in
/// the target syntax, `2:0x05`, so it can be typed back as a target.
fn cov_notification_text(received: &ReceivedCOVNotification, format: OutputFormat) -> String {
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
    let source_mac = bacnet_ops::target::format_sc_mac(&received.source_mac);
    let routed_source = received
        .source_network
        .zip(received.source_address.as_ref())
        .and_then(|(network, address)| routed_station(network, address));

    // The values through the shared formatter, named for the object.
    let values = super::read::formatter(notification.monitored_object_identifier.object_type());
    let mut props = Vec::new();
    let mut decoded = serde_json::Map::new();
    for pv in &notification.list_of_values {
        let prop_name = format!("{}", pv.property_identifier);
        let value_str = values.value(pv.property_identifier, &pv.value);
        let data = values.data(pv.property_identifier, &pv.value);
        decoded.insert(
            prop_name.clone(),
            serde_json::to_value(data).unwrap_or_default(),
        );
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
                parts.push(format!("  {prop} = {}", crate::output::screen(val)));
            }
            parts.join("\n")
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
                "routed_source": routed_source,
                "time_remaining": notification.time_remaining,
                "values": prop_map,
                "decoded": decoded,
            });
            serde_json::to_string(&json).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::renewal_interval;
    use bacnet_ops::target::format_sc_mac;
    use std::time::Duration;

    /// Renewal comes 30 s before a subscription runs out, never before half
    /// of it, at least a second apart, and never for an indefinite one.
    #[test]
    fn finite_subscriptions_renew_before_they_run_out() {
        let secs = |s| Some(Duration::from_secs(s));
        assert_eq!(renewal_interval(300), secs(270));
        assert_eq!(renewal_interval(40), secs(20));
        assert_eq!(renewal_interval(1), secs(1));
        assert_eq!(renewal_interval(u32::MAX), secs(u64::from(u32::MAX) - 30));
        assert_eq!(renewal_interval(0), None);
    }
    use bacnet_ops::target::{Target, parse_target};

    /// The opening line is one line in either format: the status line, or
    /// one compact JSON object of type `subscribed` that keeps the status
    /// object's keys (#2783).
    #[test]
    fn the_opening_line_is_one_line() {
        use super::{Held, opening_text};
        use crate::output::OutputFormat;
        use bacnet_types::enums::ObjectType;
        use bacnet_types::primitives::ObjectIdentifier;

        let held = |lifetime| Held {
            process_id: 7,
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap(),
            confirmed: false,
            lifetime,
        };
        let line = opening_text(&held(300), OutputFormat::Json);
        assert!(!line.contains('\n'), "{line}");
        let json: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "subscribed",
                "status": "ok",
                "message": "Subscribed to ANALOG_INPUT:2 (process_id=7; lifetime 300s, \
                            renewed while watching, cancelled on exit). Watching... \
                            (Ctrl+C to stop)",
            })
        );
        let table = opening_text(&held(0), OutputFormat::Table);
        assert!(!table.contains('\n'), "{table}");
        assert!(table.contains("indefinite: it stays after the CLI exits"));
        assert!(serde_json::from_str::<serde_json::Value>(&table).is_err());
    }

    /// A COV source on B/IPv6 prints as the `bip6:` VMAC a target takes back;
    /// other MACs print as hex.
    #[test]
    fn cov_sources_print_b_ipv6_vmacs_as_targets() {
        let vmac = [0x00, 0x04, 0xD2];
        let printed = format_sc_mac(&vmac);
        assert_eq!(printed, "bip6:0x0004D2");
        match parse_target(&printed) {
            Ok(Target::Mac(mac)) => assert_eq!(mac, vmac),
            _ => panic!("{printed} does not read back"),
        }
        assert_eq!(format_sc_mac(&[2, 0, 0, 0, 0, 1]), "02:00:00:00:00:01");
        assert_eq!(format_sc_mac(&[7]), "07");
    }

    /// A routed COV source prints as the `NET:0xMAC` target, in the text
    /// and the JSON forms, and parses back to the same station (#1916).
    #[test]
    fn routed_cov_sources_print_as_targets() {
        use bacnet_client::client::{COVNotificationDelivery, ReceivedCOVNotification};
        use bacnet_services::cov::COVNotificationRequest;
        use bacnet_types::MacAddr;
        use bacnet_types::enums::ObjectType;
        use bacnet_types::primitives::ObjectIdentifier;

        use super::cov_notification_text;
        use crate::output::OutputFormat;

        let received = |routed: Option<(u16, &[u8])>| ReceivedCOVNotification {
            notification: COVNotificationRequest {
                subscriber_process_identifier: 1,
                initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2001)
                    .unwrap(),
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                time_remaining: 300,
                list_of_values: Vec::new(),
            },
            source_mac: MacAddr::from_slice(&[10, 0, 1, 1, 0xBA, 0xC0]),
            source_network: routed.map(|(network, _)| network),
            source_address: routed.map(|(_, mac)| MacAddr::from_slice(mac)),
            delivery: COVNotificationDelivery::Unconfirmed,
        };
        let stations: [(u16, &[u8], &str); 2] = [
            (2, &[5], "2:0x05"),
            (3, &[127, 0, 0, 1, 0xC3, 0x50], "3:0x7F000001C350"),
        ];
        for (network, mac, text) in stations {
            let notification = received(Some((network, mac)));
            let table = cov_notification_text(&notification, OutputFormat::Table);
            assert!(
                table.contains(&format!("\n  routed-source = {text}")),
                "{table}"
            );
            let json: serde_json::Value =
                serde_json::from_str(&cov_notification_text(&notification, OutputFormat::Json))
                    .unwrap();
            assert_eq!(json["routed_source"], text);
            assert_eq!(json["source_network"], network);
            assert!(json.get("source_address").is_none());
            assert_eq!(
                parse_target(text),
                Ok(Target::RoutedMac {
                    dnet: network,
                    mac: mac.to_vec(),
                    one_station: false
                })
            );
        }
        // Values are named for the monitored object (#2078): the text, and
        // in JSON the typed form beside it.
        let mut named = received(None);
        named.notification.monitored_object_identifier =
            ObjectIdentifier::new(ObjectType::BINARY_INPUT, 1).unwrap();
        named.notification.list_of_values = vec![
            bacnet_types::constructed::BACnetPropertyValue {
                property_identifier: bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: vec![0x91, 0x01],
                priority: None,
            },
            bacnet_types::constructed::BACnetPropertyValue {
                property_identifier: bacnet_types::enums::PropertyIdentifier::STATUS_FLAGS,
                property_array_index: None,
                value: vec![0x82, 0x04, 0x80],
                priority: None,
            },
        ];
        let table = cov_notification_text(&named, OutputFormat::Table);
        assert!(table.contains("\n  PRESENT_VALUE = active"), "{table}");
        assert!(table.contains("\n  STATUS_FLAGS = in-alarm"), "{table}");
        let json: serde_json::Value =
            serde_json::from_str(&cov_notification_text(&named, OutputFormat::Json)).unwrap();
        assert_eq!(json["values"]["PRESENT_VALUE"], "active");
        assert_eq!(
            json["decoded"]["PRESENT_VALUE"],
            serde_json::json!({"value": 1, "name": "active"})
        );
        assert_eq!(
            json["decoded"]["STATUS_FLAGS"],
            serde_json::json!({"bits": "1000", "names": ["in-alarm"]})
        );

        // A local source has no routed source.
        let local = received(None);
        assert!(!cov_notification_text(&local, OutputFormat::Table).contains("routed-source"));
        let json: serde_json::Value =
            serde_json::from_str(&cov_notification_text(&local, OutputFormat::Json)).unwrap();
        assert!(json["routed_source"].is_null());
    }
}

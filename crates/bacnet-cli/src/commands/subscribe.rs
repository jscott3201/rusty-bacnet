//! COV subscription command.
//!
//! A subscription is finite by default (300 seconds), renewed while the
//! command watches and cancelled when it stops, so it leaves nothing behind
//! on the device. A lifetime of 0 asks for one that never expires (Clause
//! 13.14.1.1.4); it stays after the CLI exits, which is why the write guard
//! treats it as a write (`bacnet_tool::class`).

use std::future::Future;
use std::io::Write;
use std::time::Duration;

use crate::destination::Destination;
use bacnet_client::client::{
    BACnetClient, COVEvent, COVNotificationDelivery, ReceivedCOVNotification,
};
use bacnet_ops::target::{AddressStyle, routed_station};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::output::{self, OutputFormat, Printed};

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
/// after that (a renewal, the cancel, a write to stdout) ends the stream
/// with an error line; a refused subscription prints nothing. A reader that
/// goes away (a closed pipe) stops the watch as Ctrl+C does, once a line
/// finds it gone. Notifications missed because the watch fell behind are a
/// note on stderr and, in JSON, a `missed` line where the gap is (#2787).
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

    let watched = watch(client, to, held, rx, format, &mut std::io::stdout(), stop).await;
    let ended = if lifetime != 0
        && let Err(error) = to.unsubscribe_cov(client, held.process_id, held.oid).await
    {
        let message = format!("cancelling the subscription failed: {error}");
        Err(match watched {
            Ok(()) => message,
            Err(watch) => format!("{watch}; {message}"),
        }
        .into())
    } else {
        watched
    };
    if let Err(error) = &ended {
        output::end_stream(&**error, format);
    }
    ended
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

/// The error the watch ends with when the client's notifications end.
const CHANNEL_CLOSED: &str = "Notification channel closed";

/// Print the opening line, then each notification until `stop` completes
/// or stdout's reader has gone, renewing a finite subscription on the way.
/// The lines go to `out`, which is stdout but for tests.
async fn watch<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    to: &Destination,
    held: Held,
    mut rx: broadcast::Receiver<COVEvent>,
    format: OutputFormat,
    out: &mut impl Write,
    stop: impl Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error>> {
    if output::print_line_to(out, &opening_text(&held, format))? == Printed::ReaderGone {
        return Ok(());
    }
    let style = AddressStyle::of(client.transport());
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
            () = &mut stop => return Ok(()),
            () = renew => {
                if let Err(error) = held.subscribe(client, to).await {
                    return Err(format!("renewing the subscription failed: {error}").into());
                }
                renew_at = renew_every.map(|every| tokio::time::Instant::now() + every);
            }
            result = rx.recv() => {
                if let Some(line) = event_line(result, style, format)?
                    && output::print_line_to(out, &line)? == Printed::ReaderGone
                {
                    return Ok(());
                }
            }
        }
    }
}

/// What a received event prints: a notification's line, or nothing for one
/// the command doesn't watch. Notifications the watch fell too far behind to
/// read (more than the client's buffer, 64, came while it was busy, as with
/// a slow reader of stdout) are a note on stderr and, in JSON, a `missed`
/// line with their exact count, which comes where they would have (#2787).
/// The client's notifications ending is an error that ends the watch.
fn event_line(
    received: Result<COVEvent, RecvError>,
    style: AddressStyle,
    format: OutputFormat,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match received {
        Ok(COVEvent::Object(notification)) => {
            Ok(Some(cov_notification_text(&notification, style, format)))
        }
        // A COV-multiple notification answers a SubscribeCOVPropertyMultiple,
        // which this command never sends: nothing it watches.
        Ok(COVEvent::Multiple(_)) => Ok(None),
        Err(RecvError::Lagged(n)) => Ok(output::missed(
            &format!("Missed {n} notifications (buffer overflow)"),
            n,
            format,
        )),
        Err(RecvError::Closed) => Err(CHANNEL_CLOSED.into()),
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

/// A COV notification as printed in `format`. Its sender, the link peer it
/// came from (the router, for a routed one), prints in `style`, the link's
/// own form (`192.168.1.10:47808` on B/IP) as other commands print a peer;
/// a routed source prints in the target syntax, `2:0x05`. Both read back as
/// targets.
fn cov_notification_text(
    received: &ReceivedCOVNotification,
    style: AddressStyle,
    format: OutputFormat,
) -> String {
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
    let source_mac = style.format(&received.source_mac);
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
            output::json_line(&json)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::renewal_interval;
    use bacnet_ops::target::AddressStyle;
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

    /// A notification from its sender `mac`, local or through a router.
    fn received(
        mac: &[u8],
        routed: Option<(u16, &[u8])>,
    ) -> bacnet_client::client::ReceivedCOVNotification {
        use bacnet_client::client::{COVNotificationDelivery, ReceivedCOVNotification};
        use bacnet_services::cov::COVNotificationRequest;
        use bacnet_types::MacAddr;
        use bacnet_types::enums::ObjectType;
        use bacnet_types::primitives::ObjectIdentifier;

        ReceivedCOVNotification {
            notification: COVNotificationRequest {
                subscriber_process_identifier: 1,
                initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2001)
                    .unwrap(),
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                time_remaining: 300,
                list_of_values: Vec::new(),
            },
            source_mac: MacAddr::from_slice(mac),
            source_network: routed.map(|(network, _)| network),
            source_address: routed.map(|(_, mac)| MacAddr::from_slice(mac)),
            delivery: COVNotificationDelivery::Unconfirmed,
        }
    }

    /// A notification's sender prints in its link's own form, as other
    /// commands print a peer, in the text and in JSON's `source_mac`: B/IP
    /// as `ip:port`, B/IPv6 as its `bip6:` VMAC, MS/TP as `mstp:N`, and a
    /// BACnet/SC VMAC as hex octets (#2783). Each reads back as a target.
    #[test]
    fn a_sender_prints_in_its_link_s_form() {
        use super::cov_notification_text;
        use crate::output::OutputFormat;

        let senders: [(AddressStyle, &[u8], &str); 4] = [
            (
                AddressStyle::Bip,
                &[127, 0, 0, 1, 0xF9, 0x52],
                "127.0.0.1:63826",
            ),
            (AddressStyle::Bip6, &[0x00, 0x04, 0xD2], "bip6:0x0004D2"),
            (AddressStyle::Mstp, &[5], "mstp:5"),
            (AddressStyle::Hex, &[2, 0, 0, 0, 0, 1], "02:00:00:00:00:01"),
        ];
        for (style, mac, text) in senders {
            let notification = received(mac, None);
            let table = cov_notification_text(&notification, style, OutputFormat::Table);
            assert!(table.contains(&format!(", source={text}, ")), "{table}");
            let json: serde_json::Value = serde_json::from_str(&cov_notification_text(
                &notification,
                style,
                OutputFormat::Json,
            ))
            .unwrap();
            assert_eq!(json["source_mac"], text);
            assert!(parse_target(text).is_ok(), "{text} does not read back");
        }
    }

    /// The client's notifications ending ends the watch with an error
    /// (status 1, after the error line, #2783); missed ones are a `missed`
    /// line in JSON and nothing on a table's stdout (#2787), and the watch
    /// goes on.
    #[test]
    fn the_notifications_ending_is_an_error() {
        use super::event_line;
        use crate::output::OutputFormat;
        use tokio::sync::broadcast::error::RecvError;

        let ended = event_line(
            Err(RecvError::Closed),
            AddressStyle::Bip,
            OutputFormat::Json,
        );
        assert_eq!(
            ended.expect_err("an error").to_string(),
            "Notification channel closed"
        );
        let missed =
            |format| event_line(Err(RecvError::Lagged(3)), AddressStyle::Bip, format).unwrap();
        assert_eq!(
            missed(OutputFormat::Json).as_deref(),
            Some(r#"{"count":3,"type":"missed"}"#)
        );
        assert_eq!(missed(OutputFormat::Table), None);
    }

    /// A notification about AI:`instance`, as the client publishes it.
    fn about(instance: u32) -> bacnet_client::client::COVEvent {
        use bacnet_types::enums::ObjectType;
        use bacnet_types::primitives::ObjectIdentifier;

        let mut notification = received(&[2], None);
        notification.notification.monitored_object_identifier =
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap();
        bacnet_client::client::COVEvent::Object(notification)
    }

    /// Until the watch has read everything `sender` sent, or learned that
    /// it lost some.
    async fn read_up(sender: &tokio::sync::broadcast::Sender<bacnet_client::client::COVEvent>) {
        for _ in 0..100 {
            if sender.is_empty() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the watch stopped reading");
    }

    /// `watch`'s stdout in `format` while its buffer, of 2 notifications
    /// here, overflows (#2787): it reads AI:1; AI:2 to AI:5 arrive before
    /// it reads again, so AI:2 and AI:3 are lost; then AI:6 comes, and the
    /// channel closes, which ends the watch. No timers: each step waits
    /// until the watch has read what was sent.
    async fn watched_across_a_gap(format: crate::output::OutputFormat) -> String {
        use super::{Held, watch};
        use crate::destination::Destination;
        use bacnet_client::client::{BACnetClient, ClientConfig};
        use bacnet_ops::DeviceAddress;
        use bacnet_transport::test_transport::TestTransport;
        use bacnet_types::enums::ObjectType;
        use bacnet_types::primitives::ObjectIdentifier;

        let transport = TestTransport::builder().build();
        let mut client = BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap();
        let to = Destination {
            address: DeviceAddress::local(&[2]).unwrap(),
            instance: None,
        };
        let held = Held {
            process_id: 7,
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            confirmed: false,
            // Indefinite: no renewal to send.
            lifetime: 0,
        };
        let (sender, notifications) = tokio::sync::broadcast::channel(2);
        let device = async move {
            sender.send(about(1)).unwrap();
            read_up(&sender).await;
            for instance in 2..=5 {
                sender.send(about(instance)).unwrap();
            }
            read_up(&sender).await;
            sender.send(about(6)).unwrap();
        };
        let mut stdout = Vec::new();
        let stop = std::future::pending();
        let watching = watch(&client, &to, held, notifications, format, &mut stdout, stop);
        let (watched, ()) = tokio::join!(watching, device);
        assert_eq!(
            watched.expect_err("the channel closed").to_string(),
            "Notification channel closed"
        );
        client.stop().await.unwrap();
        String::from_utf8(stdout).unwrap()
    }

    /// A watch that falls behind marks the gap where it is (#2787): in JSON
    /// a `missed` line with the exact count between the notifications on
    /// either side of it, every line one JSON value, and the stream goes on
    /// after it. A table's stdout is the notifications alone (the note is
    /// on stderr).
    #[tokio::test]
    async fn a_gap_in_the_notifications_is_marked_where_it_is() {
        use crate::output::OutputFormat;

        let json = watched_across_a_gap(OutputFormat::Json).await;
        let lines: Vec<serde_json::Value> = json
            .lines()
            .map(|line| serde_json::from_str(line).expect("one JSON value a line"))
            .collect();
        let kinds: Vec<&str> = lines
            .iter()
            .map(|line| line["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            ["subscribed", "cov", "missed", "cov", "cov", "cov"],
            "{json}"
        );
        assert_eq!(lines[2], serde_json::json!({"type": "missed", "count": 2}));
        let objects: Vec<&str> = lines
            .iter()
            .filter_map(|line| line["object"].as_str())
            .collect();
        let read = [
            "ANALOG_INPUT:1",
            "ANALOG_INPUT:4",
            "ANALOG_INPUT:5",
            "ANALOG_INPUT:6",
        ];
        assert_eq!(objects, read, "{json}");

        let table = watched_across_a_gap(OutputFormat::Table).await;
        assert!(!table.to_lowercase().contains("missed"), "{table}");
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 5, "the opening line, 4 notifications: {table}");
        assert!(lines[0].contains("Subscribed to ANALOG_INPUT:1"), "{table}");
        let objects: Vec<&str> = lines[1..]
            .iter()
            .map(|line| line.strip_prefix("[COV] ").expect("a notification"))
            .map(|line| line.split(' ').next().unwrap())
            .collect();
        assert_eq!(objects, read, "{table}");
    }

    /// A routed COV source prints as the `NET:0xMAC` target, in the text
    /// and the JSON forms, and parses back to the same station (#1916).
    #[test]
    fn routed_cov_sources_print_as_targets() {
        use bacnet_types::enums::ObjectType;
        use bacnet_types::primitives::ObjectIdentifier;

        use crate::output::OutputFormat;

        fn text_of(
            notification: &bacnet_client::client::ReceivedCOVNotification,
            format: OutputFormat,
        ) -> String {
            super::cov_notification_text(notification, AddressStyle::Bip, format)
        }
        let router = [10, 0, 1, 1, 0xBA, 0xC0];
        let received = |routed| received(&router, routed);
        let stations: [(u16, &[u8], &str); 2] = [
            (2, &[5], "2:0x05"),
            (3, &[127, 0, 0, 1, 0xC3, 0x50], "3:0x7F000001C350"),
        ];
        for (network, mac, text) in stations {
            let notification = received(Some((network, mac)));
            let table = text_of(&notification, OutputFormat::Table);
            assert!(
                table.contains(&format!("\n  routed-source = {text}")),
                "{table}"
            );
            let json: serde_json::Value =
                serde_json::from_str(&text_of(&notification, OutputFormat::Json)).unwrap();
            assert_eq!(json["source_mac"], "10.0.1.1:47808", "the router");
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
        let table = text_of(&named, OutputFormat::Table);
        assert!(table.contains("\n  PRESENT_VALUE = active"), "{table}");
        assert!(table.contains("\n  STATUS_FLAGS = in-alarm"), "{table}");
        let json: serde_json::Value =
            serde_json::from_str(&text_of(&named, OutputFormat::Json)).unwrap();
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
        assert!(!text_of(&local, OutputFormat::Table).contains("routed-source"));
        let json: serde_json::Value =
            serde_json::from_str(&text_of(&local, OutputFormat::Json)).unwrap();
        assert!(json["routed_source"].is_null());
    }
}

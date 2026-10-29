//! `bacnet_discover`: Who-Is, then the discovery table, paged.

use std::sync::PoisonError;
use std::time::Duration;

use bacnet_client::discovery::DiscoveredDevice;
use bacnet_ops::engine::{Wanted, WhoIsScope};
use bacnet_ops::label::kebab;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::{Spec, next, schema, start};
use crate::context::{Call, Ctx};
use crate::errors::ToolError;
use crate::untrusted::envelope;
use crate::value::row_text;

pub(super) const NAME: &str = "bacnet_discover";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Discover BACnet devices",
    description: "Send a Who-Is (unless send is false) and list the devices that have \
                  answered: instance, address, network, vendor id, APDU size and \
                  segmentation. scope local (the default) asks this network, network asks one \
                  remote network, directed asks one address, global asks every network. A \
                  global Who-Is, or one without a range or wider than 1000 instances, needs \
                  broad discovery in the server's policy, at most one a minute: give a range \
                  (1000-1999) otherwise. A directed Who-Is counts too, unless its address is \
                  a device already discovered (an unknown address may be a subnet broadcast). \
                  When the policy names the devices it may reach, the range must lie inside \
                  them. names: true also reads each listed device's \
                  name. send: false only lists what is known; pages index a live table. \
                  Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: true,
};

/// How long one broad Who-Is holds off the next.
const BROAD_INTERVAL: Duration = Duration::from_secs(60);

/// The widest range of instances a Who-Is may ask without broad
/// discovery, when the policy doesn't name the devices.
pub(crate) const BROAD_WIDTH: u32 = 1000;

fn input() -> Value {
    schema::input(
        json!({
            "send": {
                "type": "boolean",
                "default": true,
                "description": "Send a Who-Is first; false lists the devices already known",
            },
            "scope": { "enum": ["local", "network", "directed", "global"], "default": "local" },
            "network": {
                "type": "integer",
                "minimum": 1,
                "maximum": 65534,
                "description": "The remote network, for scope network",
            },
            "address": {
                "type": "string",
                "description": "The station address, for scope directed (192.168.1.20, mstp:5, ...)",
            },
            "range": {
                "type": "string",
                "pattern": "^[0-9]+(-[0-9]+)?$",
                "description": "Device instances to ask, low-high or one instance",
            },
            "wait_s": { "type": "integer", "minimum": 1, "maximum": 30, "default": 3 },
            "names": {
                "type": "boolean",
                "default": false,
                "description": "Also read each listed device's Object_Name",
            },
            "cursor": schema::cursor(),
        }),
        &[],
    )
}

fn output() -> Value {
    schema::output(json!({
        "devices": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "instance": { "type": "integer" },
                    "address": { "type": "string" },
                    "router": { "type": ["string", "null"] },
                    "network": { "type": "integer" },
                    "vendor_id": { "type": "integer" },
                    "max_apdu": { "type": "integer" },
                    "segmentation": { "type": "string" },
                    "last_seen_s": { "type": "integer" },
                    "name": schema::envelope(),
                },
                "required": ["instance", "address", "network"],
            },
        },
        "total": { "type": "integer" },
        "sent": { "type": "boolean" },
        "scope": { "type": "string" },
    }))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Scope {
    #[default]
    Local,
    Network,
    Directed,
    Global,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    send: Option<bool>,
    #[serde(default)]
    scope: Scope,
    network: Option<u32>,
    address: Option<String>,
    range: Option<String>,
    wait_s: Option<u64>,
    #[serde(default)]
    names: bool,
    cursor: Option<String>,
}

/// A range argument: `low-high` or one instance.
pub(super) fn parse_range(text: &str) -> Result<(u32, u32), ToolError> {
    let bad = || ToolError::invalid("range", format!("'{text}' is not low-high or an instance"));
    let (low, high) = match text.split_once('-') {
        Some((low, high)) => (
            low.parse().map_err(|_| bad())?,
            high.parse().map_err(|_| bad())?,
        ),
        None => {
            let one = text.parse().map_err(|_| bad())?;
            (one, one)
        }
    };
    if low > high || high > ObjectIdentifier::MAX_INSTANCE {
        return Err(bad());
    }
    Ok((low, high))
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let range = args.range.as_deref().map(parse_range).transpose()?;
    let network = args
        .network
        .map(|n| {
            u16::try_from(n)
                .ok()
                .filter(|n| (1..=65534).contains(n))
                .ok_or_else(|| ToolError::invalid("network", "a network is 1 to 65534"))
        })
        .transpose()?;
    let first = start(ctx, NAME, args.cursor.as_deref(), raw)? as usize;
    // A later page lists what the first one found; it sends nothing. A
    // passive server sends nothing either, so it only lists.
    let send = args.send.unwrap_or(ctx.engine.is_some()) && args.cursor.is_none();
    if send && ctx.engine.is_none() {
        return Err(ToolError::refused(
            "passive",
            "the server is passive: it sends no Who-Is and only lists devices already heard, \
             which, with no client, is none; call with send false",
        ));
    }
    let wait = Duration::from_secs(args.wait_s.unwrap_or(3).clamp(1, 30));
    let scope_text = match (args.scope, network, &args.address) {
        (Scope::Local, ..) => "local".to_string(),
        (Scope::Network, Some(network), _) => format!("network {network}"),
        (Scope::Directed, _, Some(address)) => format!("directed {address}"),
        (Scope::Global, ..) => "global".to_string(),
        (Scope::Network, None, _) => {
            return Err(ToolError::invalid(
                "network",
                "scope network needs a network",
            ));
        }
        (Scope::Directed, _, None) => {
            return Err(ToolError::invalid(
                "address",
                "scope directed needs an address",
            ));
        }
    };
    if send {
        who_is(ctx, &args, network, range).await?;
        tokio::time::sleep(wait).await;
    }
    let mut devices = match &ctx.engine {
        Some(engine) => engine.devices().await,
        // Passive: no client, so nothing is known.
        None => Vec::new(),
    };
    devices.retain(|found| {
        let instance = found.object_identifier.instance_number();
        let on = found.source_network.unwrap_or(0);
        let in_scope = match args.scope {
            Scope::Local => found.is_local(),
            Scope::Network => found.source_network == network,
            Scope::Directed | Scope::Global => true,
        };
        in_scope
            && range.is_none_or(|(low, high)| (low..=high).contains(&instance))
            && ctx.policy.allows_network(on)
            && ctx.policy.allows_device(instance)
    });
    devices.sort_by_key(|found| found.object_identifier.instance_number());
    let page = ctx.policy.caps.devices;
    let cap = ctx.policy.caps.text;
    let mut rows = Vec::new();
    let mut resume = None;
    for (at, found) in devices.iter().enumerate().skip(first) {
        if at >= first + page || (args.names && at > first && call.out_of_time()) {
            resume = Some(at);
            break;
        }
        let instance = found.object_identifier.instance_number();
        let mut row = device_row(ctx, found);
        if args.names
            && let Some(engine) = &ctx.engine
        {
            let object = found.object_identifier;
            let name = engine
                .read(
                    instance,
                    object,
                    Wanted::List(&[PropertyIdentifier::OBJECT_NAME]),
                )
                .await
                .ok()
                .and_then(|read| row_text(&read.rows, PropertyIdentifier::OBJECT_NAME));
            let source = format!("device {instance} device:{instance} object-name");
            row["name"] = name.map_or(Value::Null, |name| envelope(&name, source, cap));
        }
        rows.push(row);
    }
    let mut body = json!({
        "devices": rows,
        "total": devices.len(),
        "sent": send,
        "scope": scope_text,
        "truncated": resume.is_some(),
        "next_cursor": resume.map(|at| next(ctx, NAME, raw, at as u64)),
        "limits": ctx.limits(Some(page)),
    });
    if ctx.engine.is_none() {
        body["note"] = json!("passive: nothing is sent, and only devices already heard are listed");
    }
    Ok(body)
}

/// One device of the discovery table as a result row: identifiers and
/// what its I-Am said, no device text.
pub(crate) fn device_row<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    found: &DiscoveredDevice,
) -> Value {
    let routed = found.source_network.zip(found.source_address.as_deref());
    let (address, router) = match &ctx.engine {
        Some(engine) => engine.address_style().locate(&found.mac_address, routed),
        None => (String::new(), None),
    };
    json!({
        "instance": found.object_identifier.instance_number(),
        "address": address,
        "router": router,
        "network": found.source_network.unwrap_or(0),
        "vendor_id": found.vendor_id,
        "max_apdu": found.max_apdu_length,
        "segmentation": kebab(found.segmentation_supported),
        "last_seen_s": found.last_seen.elapsed().as_secs(),
    })
}

/// Send the Who-Is the arguments ask for, if the policy allows it.
async fn who_is<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    args: &Args,
    network: Option<u16>,
    range: Option<(u32, u32)>,
) -> Result<(), ToolError> {
    let engine = ctx.engine()?;
    let scope = match (args.scope, network, &args.address) {
        (Scope::Local, ..) => {
            ctx.allow_network(0)?;
            WhoIsScope::Local
        }
        (Scope::Network, Some(network), _) => {
            ctx.allow_network(network)?;
            WhoIsScope::Network(network)
        }
        (Scope::Directed, _, Some(address)) => {
            ctx.allow_network(0)?;
            WhoIsScope::Directed(address)
        }
        (Scope::Global, ..) => {
            if ctx.policy.networks.is_some() {
                return Err(ToolError::refused(
                    "network-not-allowed",
                    "a global Who-Is reaches every network, and this server's [mcp] networks \
                     limit which it may reach",
                ));
            }
            WhoIsScope::Global
        }
        _ => unreachable!("checked by the caller"),
    };
    // A directed Who-Is's address, checked before anything is charged, and
    // whether the discovery table knows a device there.
    let known_station = match scope {
        WhoIsScope::Directed(address) => {
            let mac = engine.station_mac(address).await.map_err(|e| match e {
                bacnet_types::error::Error::Encoding(message) => {
                    ToolError::invalid("address", message)
                }
                other => ToolError::invalid("address", other.to_string()),
            })?;
            engine
                .devices()
                .await
                .iter()
                .any(|found| found.is_local() && found.mac_address.as_slice() == mac.as_slice())
        }
        _ => false,
    };
    if broad(ctx, args.scope, range, known_station)? {
        broad_allowed(ctx)?;
    }
    let range = range
        .map(|(low, high)| DeviceInstanceRange::new(low, high))
        .transpose()
        .map_err(|e| ToolError::invalid("range", e.to_string()))?;
    engine.who_is(scope, range).await.map_err(|e| match e {
        bacnet_types::error::Error::Encoding(message) => ToolError::invalid("address", message),
        other => ToolError::Failed(other.to_string()),
    })
}

/// Whether a Who-Is is broad, after refusing one that would make devices
/// outside the policy answer.
///
/// - When the policy names its devices, the range must lie inside one of
///   its ranges, whatever the scope.
/// - Broad is about traffic, so in either case a global Who-Is is broad,
///   and so is one without a range or wider than [`BROAD_WIDTH`]
///   instances. The one exception is a directed Who-Is to a station the
///   discovery table already knows as a device (`known_station`): only it
///   answers. An unknown address may be a subnet's broadcast address, which
///   every device on that subnet hears.
fn broad<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    scope: Scope,
    range: Option<(u32, u32)>,
    known_station: bool,
) -> Result<bool, ToolError> {
    if let Some(allowed) = &ctx.policy.devices {
        let inside =
            range.is_some_and(|(low, high)| allowed.iter().any(|&(l, h)| l <= low && high <= h));
        if !inside {
            return Err(ToolError::refused(
                "device-not-allowed",
                "this server's [mcp] devices limit which devices it may reach: give a range \
                 inside them",
            ));
        }
    }
    let wide = range.is_none_or(|(low, high)| high - low >= BROAD_WIDTH);
    let one_station = scope == Scope::Directed && known_station;
    Ok(scope == Scope::Global || (wide && !one_station))
}

/// A broad Who-Is: allowed by the policy, and not within a minute of the
/// last one.
fn broad_allowed<T: TransportPort + 'static>(ctx: &Ctx<T>) -> Result<(), ToolError> {
    if !ctx.policy.broad_discovery {
        return Err(ToolError::refused(
            "broad-discovery",
            "a global Who-Is, or one without a range or wider than 1000 instances, needs \
             broad_discovery = true in the profile's [mcp] table; give a narrower range, such as \
             1000-1999, instead",
        ));
    }
    let mut last = ctx.broad.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    if let Some(at) = *last
        && now.duration_since(at) < BROAD_INTERVAL
    {
        let wait = BROAD_INTERVAL - now.duration_since(at);
        return Err(ToolError::Refused {
            reason: "rate-limited",
            message: "one broad Who-Is a minute".into(),
            retry_after_s: Some(wait.as_secs().max(1)),
        });
    }
    *last = Some(now);
    Ok(())
}

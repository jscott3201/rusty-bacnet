//! `bacnet_network_map`: the networks and routers the client knows, and
//! the devices on each; with `bbmd: true`, the BBMD tables of the local
//! B/IP network and a consistency verdict ([`super::bbmd`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{Spec, bbmd, schema};
use crate::context::{Call, Ctx, ROUTER_CAP};
use crate::errors::ToolError;

pub(super) const NAME: &str = "bacnet_network_map";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Map BACnet networks",
    description: "Map the networks this server may reach: each network with the routers \
                  that reach it, how fast the policy lets requests go there, and the devices \
                  discovered on it; and the routers with the networks each reaches. send: \
                  true first broadcasts a Who-Is-Router-To-Network (for one network, or all) \
                  and waits wait_s for the routers' answers; that is a broadcast every router \
                  answers, so it needs broad discovery in the server's policy and goes at most \
                  once a minute. Without send, the routers are the last ones heard. bbmd: true \
                  also reads the BDT and FDT (BBMD tables) of the BBMDs \
                  in reach (the one this server registers with, any bbmds named by a known \
                  device's or router's ip:port, and the peers their tables list) and returns a \
                  verdict: entries one BBMD has and its peer lacks, mismatched masks, foreign \
                  devices near expiry, and BBMDs that didn't answer or don't support the read. \
                  It needs broad discovery and goes at most once a minute. Sends no table \
                  writes, registrations or network-layer writes. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// How long a router query holds off the next.
const ROUTER_INTERVAL: Duration = Duration::from_secs(60);

/// Device instances listed for one network; past it, only the count.
const LISTED: usize = 200;

fn input() -> Value {
    schema::input(
        json!({
            "send": {
                "type": "boolean",
                "default": false,
                "description": "Broadcast a Who-Is-Router-To-Network first (broad discovery only)",
            },
            "network": {
                "type": "integer",
                "minimum": 1,
                "maximum": 65534,
                "description": "Ask only for the routers to this network, and map only it",
            },
            "wait_s": { "type": "integer", "minimum": 1, "maximum": 30, "default": 3 },
            "bbmd": {
                "type": "boolean",
                "default": false,
                "description": "Read the BBMD tables of the local B/IP network and check them \
                                (broad discovery only, once a minute)",
            },
            "bbmds": {
                "type": "array",
                "items": { "type": "string", "description": "ip:port" },
                "maxItems": bbmd::MAX_NAMED,
                "description": "BBMDs to start from besides the registration's: each the B/IP \
                                address of a device discovered or a router heard (needs bbmd)",
            },
        }),
        &[],
    )
}

fn output() -> Value {
    let numbers = json!({ "type": "array", "items": { "type": "integer" } });
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    schema::output(json!({
        "sent": { "type": "boolean" },
        "bbmd": bbmd::schema(),
        "routers_heard_s": {
            "type": ["integer", "null"],
            "description": "Seconds since the routers were last asked; null if never",
        },
        "routers": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "address": { "type": "string" },
                    "networks": numbers,
                },
                "required": ["address", "networks"],
            },
        },
        "networks": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "network": { "type": "integer" },
                    "local": { "type": "boolean" },
                    "routers": strings,
                    "speed": { "enum": ["normal", "slow", "fast"] },
                    "rate_per_s": { "type": "integer" },
                    "devices": { "type": "integer" },
                    "instances": numbers,
                },
                "required": ["network", "local", "routers", "speed", "rate_per_s", "devices"],
            },
        },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    #[serde(default)]
    send: bool,
    network: Option<u32>,
    wait_s: Option<u64>,
    #[serde(default)]
    bbmd: bool,
    #[serde(default)]
    bbmds: Vec<String>,
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
) -> Result<Value, ToolError> {
    let network = args
        .network
        .map(|n| {
            u16::try_from(n)
                .ok()
                .filter(|n| (1..=65534).contains(n))
                .ok_or_else(|| ToolError::invalid("network", "a network is 1 to 65534"))
        })
        .transpose()?;
    if let Some(network) = network {
        ctx.allow_network(network)?;
    }
    let engine = ctx.engine()?;
    if !args.bbmd && !args.bbmds.is_empty() {
        return Err(ToolError::invalid("bbmds", "bbmds needs bbmd: true"));
    }
    // Checked before anything is sent, so a refusal sends nothing.
    let plan = match args.bbmd {
        true => Some(bbmd::plan(ctx, engine, &args.bbmds).await?),
        false => None,
    };
    if args.send
        && let Err(error) = ask_routers(ctx, call, network, args.wait_s).await
    {
        if let Some(plan) = &plan {
            bbmd::release(ctx, plan);
        }
        return Err(error);
    }
    let style = engine.address_style();
    let address = |mac: &[u8]| style.locate(mac, None).0;
    let (heard, known) = {
        let routers = ctx.routers.lock().unwrap_or_else(PoisonError::into_inner);
        match &routers.heard {
            Some((at, found)) => (Some(at.elapsed().as_secs()), found.clone()),
            None => (None, Vec::new()),
        }
    };
    let wanted = |n: u16| ctx.policy.allows_network(n) && network.is_none_or(|w| w == n);
    // Each network, with its routers' addresses and the devices on it.
    let mut networks: BTreeMap<u16, (BTreeSet<String>, Vec<u32>)> = BTreeMap::new();
    if network.is_none() && ctx.policy.allows_network(0) {
        networks.entry(0).or_default();
    }
    let mut routers = Vec::new();
    for router in &known {
        let reach: Vec<u16> = router
            .networks
            .iter()
            .copied()
            .filter(|&n| wanted(n))
            .collect();
        if reach.is_empty() {
            continue;
        }
        let at = address(&router.mac);
        for &n in &reach {
            networks.entry(n).or_default().0.insert(at.clone());
        }
        routers.push(json!({ "address": at, "networks": reach }));
    }
    for found in engine.devices().await {
        let on = found.source_network.unwrap_or(0);
        let instance = found.object_identifier.instance_number();
        if !wanted(on) || !ctx.policy.allows_device(instance) {
            continue;
        }
        let entry = networks.entry(on).or_default();
        if on != 0 {
            // A routed device was heard through its router's MAC.
            entry.0.insert(address(&found.mac_address));
        }
        entry.1.push(instance);
    }
    let networks: Vec<Value> = networks
        .into_iter()
        .map(|(n, (via, mut instances))| {
            instances.sort_unstable();
            let speed = if ctx.policy.slow_networks.contains(&n) {
                "slow"
            } else if ctx.policy.fast_networks.contains(&n) {
                "fast"
            } else {
                "normal"
            };
            let count = instances.len();
            instances.truncate(LISTED);
            json!({
                "network": n,
                "local": n == 0,
                "routers": via,
                "speed": speed,
                "rate_per_s": ctx.policy.network_rate(n),
                "devices": count,
                "instances": instances,
            })
        })
        .collect();
    let mut body = json!({
        "sent": args.send,
        "routers_heard_s": heard,
        "routers": routers,
        "networks": networks,
        "limits": ctx.limits(None),
    });
    if let Some(plan) = &plan {
        body["bbmd"] = bbmd::run(ctx, engine, call, plan).await;
    }
    Ok(body)
}

/// Broadcast a Who-Is-Router-To-Network and keep what answers, if the
/// policy allows a broad request and none went in the last minute. The
/// wait for answers ends with the call's budget (measured after the wait
/// for the rate slot), so a short budget returns the routers heard by then
/// rather than timing out.
///
/// A query cancelled while it waits keeps its claim on the minute: after
/// the broadcast, which went, and also during the wait for the rate slot,
/// when nothing was sent yet (accepted: the next query waits a minute
/// either way). The routers known before stay as they were; only a
/// finished query replaces them.
async fn ask_routers<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    network: Option<u16>,
    wait_s: Option<u64>,
) -> Result<(), ToolError> {
    let engine = ctx.engine()?;
    ctx.allow_network(0)?;
    if !ctx.policy.broad_discovery {
        return Err(ToolError::refused(
            "broad-discovery",
            "a Who-Is-Router-To-Network is a broadcast every router answers: it needs \
             broad_discovery = true in the profile's [mcp] table; call without send to see \
             the routers and networks already known",
        ));
    }
    let asked = Instant::now();
    let wanted = Duration::from_secs(wait_s.unwrap_or(3).clamp(1, 30));
    // Checked and claimed under one lock, so parallel calls can't both go.
    let previous_claim = {
        let mut routers = ctx.routers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = routers.asked
            && at.elapsed() < ROUTER_INTERVAL
        {
            let wait = ROUTER_INTERVAL - at.elapsed();
            return Err(ToolError::Refused {
                reason: "rate-limited",
                message: "one Who-Is-Router-To-Network a minute".into(),
                retry_after_s: Some(wait.as_secs().max(1)),
            });
        }
        routers.asked.replace(asked)
    };
    match engine
        .discover_routers(network, wanted, Some(call.deadline))
        .await
    {
        Ok(found) => {
            let mut routers = ctx.routers.lock().unwrap_or_else(PoisonError::into_inner);
            // Asking about one network hears only its routers: keep the
            // others known before, after the ones just heard, so the cap
            // drops old routers first.
            let before = match (network, routers.heard.take()) {
                (Some(_), Some((_, before))) => before,
                _ => Vec::new(),
            };
            let mut merged = found;
            for router in before {
                match merged.iter_mut().find(|known| known.mac == router.mac) {
                    Some(known) => {
                        known.networks.extend(router.networks);
                        known.networks.sort_unstable();
                        known.networks.dedup();
                    }
                    None => merged.push(router),
                }
            }
            merged.truncate(ROUTER_CAP);

            routers.heard = Some((asked, merged));
            Ok(())
        }
        Err(error) => {
            // It never left: drop the claim, and let the next try go.
            ctx.routers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .asked = previous_claim;
            Err(ToolError::Failed(error.to_string()))
        }
    }
}

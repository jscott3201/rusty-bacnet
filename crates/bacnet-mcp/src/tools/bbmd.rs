//! `bacnet_network_map` with `bbmd: true`: the BBMD tables of the local
//! B/IP network and a consistency verdict over them (#2102).
//!
//! **Which BBMDs.** No address is scanned. The survey starts from the BBMD
//! the client registers with as a foreign device (the profile's
//! registration), and from any `bbmds` the agent names, each of which must
//! be the B/IP address of a device in the discovery table (one the policy
//! allows) or of a router heard. From there it follows each BDT read to the
//! BBMDs it lists, breadth first, as the TUI's survey does: those are the
//! site's own peers, named by the BBMDs themselves. Every BBMD is on the
//! local B/IP network (network 0 from here), so the policy must allow it.
//! A listed address is device data, so one no read may go to (all zeros,
//! broadcast, multicast, loopback, our own) is reported, not read
//! ([`bacnet_ops::bbmd::NotFollowed`]).
//!
//! **Charging.** Each table read is one request under the local network's
//! rate, the global one and the principal's tokens. The survey reads at
//! most [`MAX_SURVEY`] BBMDs, [`SURVEY_AT_ONCE`] at once, so a silent one
//! costs its timeout without holding up the rest; a read the call's budget
//! can't wait for is left unread. Like a Who-Is-Router-To-Network it needs
//! broad discovery, and goes at most once a minute.
//!
//! **Read-only.** Only Read-Broadcast-Distribution-Table and
//! Read-Foreign-Device-Table go out (Annex J.2.3 and J.2.7).

use std::net::SocketAddrV4;
use std::sync::PoisonError;
use std::time::Duration;

use std::net::Ipv4Addr;

use bacnet_ops::bbmd::{
    FdtRow, Finding, Here, MAX_SURVEY, Matrix, NO_ANSWER, NotFollowed, Outcome, Probe,
    SURVEY_AT_ONCE, Survey, address_of, survey,
};
use bacnet_ops::engine::Engine;
use bacnet_ops::label::kebab;
use bacnet_transport::bvll::BvlcResultCode;
use bacnet_transport::port::TransportPort;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::context::{Call, Ctx};
use crate::errors::ToolError;

/// How long a survey holds off the next.
const BBMD_INTERVAL: Duration = Duration::from_secs(60);

/// The most BBMDs an agent may name.
pub(super) const MAX_NAMED: usize = 4;

/// Entries listed per table; past it, only the count. The check uses them
/// all.
const LISTED: usize = 24;

/// Findings listed; past it, only the count.
const MAX_FINDINGS: usize = 64;

/// BBMDs past the survey's limit named as examples; the rest are counted.
const UNREAD_EXAMPLES: usize = 4;

/// A table's FDT read, when its BDT was read.
type Fdt = Option<Outcome<Vec<FdtRow>>>;

/// Where a survey starts, checked and its minute claimed.
pub(super) struct Plan {
    seeds: Vec<SocketAddrV4>,
    /// The claim before this one, put back if the call sends nothing.
    previous: Option<Instant>,
}

/// Check that a survey may go and claim its minute; nothing is sent.
pub(super) async fn plan<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    engine: &Engine<T>,
    named: &[String],
) -> Result<Plan, ToolError> {
    // Every BBMD this server can reach is on its own B/IP network.
    ctx.allow_network(0)?;
    if engine.bip().is_none() {
        return Err(ToolError::refused(
            "needs-bip",
            "BBMD tables belong to BACnet/IP: this server's link is not B/IP",
        ));
    }
    if !ctx.policy.broad_discovery {
        return Err(ToolError::refused(
            "broad-discovery",
            "a BBMD survey reads every BBMD the tables name: it needs broad_discovery = true \
             in the profile's [mcp] table",
        ));
    }
    if named.len() > MAX_NAMED {
        return Err(ToolError::invalid(
            "bbmds",
            format!("at most {MAX_NAMED} BBMDs"),
        ));
    }
    let mut seeds: Vec<SocketAddrV4> = engine.registration_bbmd().into_iter().collect();
    for text in named {
        let address: SocketAddrV4 = text
            .parse()
            .map_err(|_| ToolError::invalid("bbmds", format!("{text}: not an ip:port")))?;
        // The same rule as for the peers a table lists: no broadcast,
        // multicast, loopback or own address is read.
        if let Some(why) = NotFollowed::check_address(address, &here(engine)) {
            return Err(ToolError::invalid(
                "bbmds",
                format!("{address} is no BBMD a read may go to ({})", why.as_str()),
            ));
        }
        if !known(ctx, engine, address).await {
            return Err(ToolError::refused(
                "address-not-known",
                format!(
                    "{address} is neither a device in the discovery table nor a router heard: \
                     name the B/IP address of one (bacnet_discover, or send: true for routers)"
                ),
            ));
        }
        if !seeds.contains(&address) {
            seeds.push(address);
        }
    }
    if seeds.is_empty() {
        return Err(ToolError::refused(
            "no-bbmd-known",
            "no BBMD to start from: the server registers with none as a foreign device; name \
             one in bbmds (a device's or router's B/IP address)",
        ));
    }
    // Checked and claimed under one lock, so parallel calls can't both go.
    let mut asked = ctx
        .bbmd_asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = *asked
        && at.elapsed() < BBMD_INTERVAL
    {
        let wait = BBMD_INTERVAL - at.elapsed();
        return Err(ToolError::Refused {
            reason: "rate-limited",
            message: "one BBMD survey a minute".into(),
            retry_after_s: Some(wait.as_secs().max(1)),
        });
    }
    let previous = asked.replace(Instant::now());
    Ok(Plan { seeds, previous })
}

/// Put the claim back: the call stopped before the survey sent anything.
pub(super) fn release<T: TransportPort + 'static>(ctx: &Ctx<T>, plan: &Plan) {
    *ctx.bbmd_asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = plan.previous;
}

/// Whether `address` is a B/IP address the server already knows: a device
/// of the discovery table on the local network that the policy allows, or
/// a router heard.
async fn known<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    engine: &Engine<T>,
    address: SocketAddrV4,
) -> bool {
    let router = {
        let routers = ctx.routers.lock().unwrap_or_else(PoisonError::into_inner);
        routers.heard.as_ref().is_some_and(|(_, found)| {
            found
                .iter()
                .any(|router| address_of(&router.mac) == Some(address))
        })
    };
    router
        || engine.devices().await.iter().any(|found| {
            found.source_network.is_none()
                && address_of(&found.mac_address) == Some(address)
                && ctx
                    .policy
                    .allows_device(found.object_identifier.instance_number())
        })
}

/// Where this server is on its B/IP link: its own address, so a table
/// listing it isn't read back, and its broadcast address. A `bip()` client
/// always has both.
fn here<T: TransportPort + 'static>(engine: &Engine<T>) -> Here {
    let bip = engine.bip();
    Here {
        own: bip
            .and_then(|bip| address_of(bip.local_mac()))
            .unwrap_or(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
        broadcast: bip.map_or(Ipv4Addr::BROADCAST, |bip| bip.broadcast_address()),
    }
}

/// Read the tables and judge them. A survey whose every read the budget
/// left unsent gives its minute back.
pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    engine: &Engine<T>,
    call: &Call,
    plan: &Plan,
) -> Value {
    let found = survey(&plan.seeds, here(engine), |bbmd| read(engine, call, bbmd)).await;
    if found
        .read
        .iter()
        .all(|s| matches!(s.probe, Outcome::Skipped))
    {
        release(ctx, plan);
    }
    report(&plan.seeds, &found)
}

/// One BBMD's BDT, then its FDT when the BDT was read; a read the call's
/// budget has no time for is not sent.
async fn read<T: TransportPort + 'static>(
    engine: &Engine<T>,
    call: &Call,
    bbmd: SocketAddrV4,
) -> (Probe, Fdt) {
    let probe = within_budget(call, engine.read_bdt(bbmd)).await;
    let fdt = match probe {
        Outcome::Table(_) => Some(within_budget(call, engine.read_fdt(bbmd)).await),
        _ => None,
    };
    (probe, fdt)
}

/// A table read, unless the budget is spent; cut short when it runs out.
async fn within_budget<R>(call: &Call, read: impl Future<Output = Outcome<R>>) -> Outcome<R> {
    if call.out_of_time() {
        return Outcome::Skipped;
    }
    let outcome = tokio::time::timeout_at(call.deadline, read)
        .await
        .unwrap_or_else(|_| Outcome::Unreachable("the call's budget ran out first".into()));
    if matches!(&outcome, Outcome::Unreachable(why) if why == NO_ANSWER) {
        call.timed_out();
    }
    outcome
}

/// The survey's result: each BBMD's tables, the findings and the verdict.
fn report(seeds: &[SocketAddrV4], found: &Survey<Fdt>) -> Value {
    let surveyed = &found.read;
    let probes: Vec<(SocketAddrV4, Probe)> =
        surveyed.iter().map(|s| (s.bbmd, s.probe.clone())).collect();
    let matrix = Matrix::build(&probes);
    let mut truncated = false;
    let mut findings: Vec<Value> = matrix.findings.iter().map(finding).collect();
    let mismatched = matrix.findings.iter().any(|f| {
        matches!(
            f,
            Finding::Asymmetric { .. } | Finding::MissingSelf(_) | Finding::MixedMasks { .. }
        )
    });
    let mut bbmds = Vec::new();
    for skip in &found.not_followed {
        findings.push(json!({
            "problem": "not-followed",
            "bbmd": skip.listed_by.to_string(),
            "address": skip.address.to_string(),
            "reason": skip.why.as_str(),
            "message": format!(
                "{} lists {}, which was not read ({}): no read goes to such an address",
                skip.listed_by,
                skip.address,
                skip.why.as_str()
            ),
        }));
    }
    for entry in surveyed {
        let bdt = table(&entry.probe, &mut truncated, |row| {
            json!({
                "address": row.address.to_string(),
                "mask": row.mask.to_string(),
                "forward_to": row.forward_to().to_string(),
            })
        });
        let fdt = match &entry.extra {
            Some(fdt) => {
                if let Outcome::Nak(code) = fdt {
                    findings.push(not_supported(entry.bbmd, "fdt", *code));
                } else if let Outcome::Unreachable(reason) = fdt {
                    findings.push(json!({
                        "problem": "no-answer",
                        "bbmd": entry.bbmd.to_string(),
                        "table": "fdt",
                        "reason": reason,
                        "message": format!("{} could not be read ({reason})", entry.bbmd),
                    }));
                }
                for row in fdt.table().into_iter().flatten().filter(|r| r.expiring()) {
                    findings.push(json!({
                        "problem": "foreign-device-expiring",
                        "bbmd": entry.bbmd.to_string(),
                        "device": row.address.to_string(),
                        "ttl_s": row.ttl,
                        "remaining_s": row.remaining,
                        "message": format!(
                            "{} has {} s left at {}: past its time-to-live of {} s without \
                             registering again",
                            row.address, row.remaining, entry.bbmd, row.ttl
                        ),
                    }));
                }
                table(fdt, &mut truncated, |row| {
                    json!({
                        "address": row.address.to_string(),
                        "ttl_s": row.ttl,
                        "remaining_s": row.remaining,
                    })
                })
            }
            _ => json!({ "status": "not-read" }),
        };
        bbmds.push(json!({ "address": entry.bbmd.to_string(), "bdt": bdt, "fdt": fdt }));
    }
    let unread = !found.unread.is_empty()
        || !found.not_followed.is_empty()
        || surveyed
            .iter()
            .any(|s| !matches!(s.probe, Outcome::Table(_)));
    truncated |= !found.unread.is_empty();
    let verdict = if mismatched {
        "inconsistent"
    } else if unread {
        "incomplete"
    } else {
        "consistent"
    };
    let count = findings.len();
    if count > MAX_FINDINGS {
        findings.truncate(MAX_FINDINGS);
        truncated = true;
    }
    json!({
        "seeds": seeds.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "verdict": verdict,
        "findings": findings,
        "finding_count": count,
        "bbmds": bbmds,
        "unread": {
            "count": found.unread.len(),
            "examples": found
                .unread
                .iter()
                .take(UNREAD_EXAMPLES)
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        },
        "truncated": truncated,
        "limits": {
            "max_bbmds": MAX_SURVEY,
            "at_once": SURVEY_AT_ONCE,
            "listed_entries": LISTED,
            "max_findings": MAX_FINDINGS,
        },
    })
}

/// One table as read: its status, and its entries up to [`LISTED`].
fn table<R>(outcome: &Outcome<Vec<R>>, truncated: &mut bool, row: impl Fn(&R) -> Value) -> Value {
    match outcome {
        Outcome::Table(rows) => {
            *truncated |= rows.len() > LISTED;
            json!({
                "status": "read",
                "count": rows.len(),
                "entries": rows.iter().take(LISTED).map(row).collect::<Vec<_>>(),
            })
        }
        Outcome::Nak(code) => json!({ "status": "not-supported", "result_code": code_json(*code) }),
        Outcome::Unreachable(reason) => json!({ "status": "no-answer", "reason": reason }),
        Outcome::Skipped => json!({ "status": "not-read" }),
    }
}

/// A result code as `{value, name}`, the name in kebab case.
fn code_json(code: BvlcResultCode) -> Value {
    let mut value = json!({ "value": code.to_raw() });
    if let Some(name) = code.name() {
        value["name"] = json!(kebab(name));
    }
    value
}

/// A NAK to a table read: the BBMD doesn't carry the read out (a node
/// that is no BBMD answers so), which is not an error.
fn not_supported(bbmd: SocketAddrV4, table: &str, code: BvlcResultCode) -> Value {
    json!({
        "problem": "read-not-supported",
        "bbmd": bbmd.to_string(),
        "table": table,
        "result_code": code_json(code),
        "message": format!(
            "{bbmd} answered the {} read with a NAK: it refused the read or doesn't support it \
             (it may be no BBMD)",
            table.to_uppercase()
        ),
    })
}

/// A matrix finding as JSON, with the addresses involved.
fn finding(finding: &Finding) -> Value {
    let message = finding.describe();
    match finding {
        Finding::Asymmetric { from, to } => json!({
            "problem": "one-way-entry",
            "from": from.to_string(),
            "to": to.to_string(),
            "message": message,
        }),
        Finding::MissingSelf(bbmd) => json!({
            "problem": "missing-own-entry",
            "bbmd": bbmd.to_string(),
            "message": message,
        }),
        Finding::NoPeers(bbmd) => json!({
            "problem": "no-peers",
            "bbmd": bbmd.to_string(),
            "message": message,
        }),
        Finding::MixedMasks { peer, masks } => json!({
            "problem": "mismatched-masks",
            "bbmd": peer.to_string(),
            "masks": masks.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "message": message,
        }),
        Finding::Nak { bbmd, code } => not_supported(*bbmd, "bdt", *code),
        Finding::Unreachable { bbmd, reason } => json!({
            "problem": "no-answer",
            "bbmd": bbmd.to_string(),
            "table": "bdt",
            "reason": reason,
            "message": message,
        }),
    }
}

/// The `bbmd` block of the output schema.
pub(super) fn schema() -> Value {
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    let code = json!({
        "type": "object",
        "properties": { "value": { "type": "integer" }, "name": { "type": "string" } },
        "required": ["value"],
    });
    let table = |entry: Value| {
        json!({
            "type": "object",
            "properties": {
                "status": { "enum": ["read", "not-supported", "no-answer", "not-read"] },
                "count": { "type": "integer" },
                "entries": { "type": "array", "items": entry },
                "result_code": code,
                "reason": { "type": "string" },
            },
            "required": ["status"],
        })
    };
    let bdt_entry = json!({
        "type": "object",
        "properties": {
            "address": { "type": "string", "description": "ip:port" },
            "mask": { "type": "string", "description": "Broadcast distribution mask, dotted" },
            "forward_to": {
                "type": "string",
                "description": "Where peers send a forwarded broadcast for this entry",
            },
        },
        "required": ["address", "mask", "forward_to"],
    });
    let fdt_entry = json!({
        "type": "object",
        "properties": {
            "address": { "type": "string", "description": "ip:port" },
            "ttl_s": { "type": "integer" },
            "remaining_s": {
                "type": "integer",
                "description": "Seconds left, including the BBMD's 30 s grace period",
            },
        },
        "required": ["address", "ttl_s", "remaining_s"],
    });
    json!({
        "type": "object",
        "description": "With bbmd: true, the BBMD tables read and a consistency verdict",
        "properties": {
            "seeds": strings,
            "verdict": {
                "enum": ["consistent", "inconsistent", "incomplete"],
                "description": "inconsistent: BDTs disagree; incomplete: none disagree but some \
                                BBMD was not read",
            },
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "problem": { "enum": [
                            "one-way-entry", "missing-own-entry", "mismatched-masks",
                            "read-not-supported", "no-answer", "foreign-device-expiring",
                            "not-followed", "no-peers",
                        ] },
                        "address": { "type": "string" },
                        "bbmd": { "type": "string" },
                        "from": { "type": "string" },
                        "to": { "type": "string" },
                        "masks": strings,
                        "table": { "enum": ["bdt", "fdt"] },
                        "result_code": code,
                        "reason": { "type": "string" },
                        "device": { "type": "string" },
                        "ttl_s": { "type": "integer" },
                        "remaining_s": { "type": "integer" },
                        "message": { "type": "string" },
                    },
                    "required": ["problem", "message"],
                },
            },
            "finding_count": { "type": "integer" },
            "bbmds": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "address": { "type": "string" },
                        "bdt": table(bdt_entry),
                        "fdt": table(fdt_entry),
                    },
                    "required": ["address", "bdt", "fdt"],
                },
            },
            "unread": {
                "type": "object",
                "description": "BBMDs the tables list past the survey's limit, not read",
                "properties": {
                    "count": { "type": "integer" },
                    "examples": strings,
                },
                "required": ["count", "examples"],
            },
            "truncated": { "type": "boolean" },
            "limits": { "type": "object" },
        },
        "required": [
            "seeds", "verdict", "findings", "finding_count", "bbmds", "unread", "truncated",
        ],
    })
}

#[cfg(test)]
#[path = "bbmd_tests.rs"]
mod tests;

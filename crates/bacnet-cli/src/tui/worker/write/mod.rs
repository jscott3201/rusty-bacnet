//! The write session's worker side: the TUI's adapter over the shared
//! write desk (`bacnet_ops::safety::desk`, #2068). The desk reads the
//! pre-image, sends the writes, and owns the ledger and the audit log; this
//! side keeps the session's arming, answers the app's commands, and sends
//! the device management requests ([`manage`]), which only the TUI offers.
//!
//! [`run`] is polled beside the session loop under `join!`, like the
//! browser and the watch list, and serves one request at a time. It does
//! not watch for the session's cancellation while a write is out: a write
//! already sent always gets its outcome into the ledger and the audit log,
//! even if the UI never hears of it. Requests still queued when the session
//! ends are recorded as not sent.
//!
//! While it runs it holds the workspace's lock (`bacnet_ops::safety::lock`),
//! so a second session on the same profile can't arm.

mod manage;

use std::path::PathBuf;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_ops::class::Class;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::safety::audit::Audit;
use bacnet_ops::safety::desk::{Caller, Desk, Sent, Waiting};
use bacnet_ops::safety::ledger::Origin;
use bacnet_ops::safety::lock::lock;
use bacnet_ops::safety::{item_json, run_id};
use bacnet_transport::port::TransportPort;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::EventSink;
use crate::tui::message::WorkerEvent;
use crate::tui::profile::{Safety, Writes};
use crate::tui::safety::audit::Context;
use crate::tui::safety::ledger::{Entry, Ledger};
use crate::tui::safety::{Change, Level, MAX_MINUTES, Outcome, SafetyCommand, SafetyEvent};

pub(crate) use bacnet_ops::safety::desk::PASSIVE;
#[cfg(test)]
pub(crate) use bacnet_ops::safety::desk::{STALE, UNKNOWN_PRE_IMAGE};

/// What the write session needs from the session's settings.
#[derive(Debug, Clone)]
pub(crate) struct SafetyOptions {
    /// The profile workspace, home of the ledger and the audit log. None
    /// in a flags-only session, which therefore can't arm.
    pub(crate) workspace: Option<PathBuf>,
    /// The profile's name.
    pub(crate) profile: Option<String>,
    /// The profile's write policy.
    pub(crate) policy: Safety,
    /// `--passive`.
    pub(crate) passive: bool,
    /// `BIP`, `BIP6` or `SC`.
    pub(crate) transport: &'static str,
}

impl Default for SafetyOptions {
    fn default() -> Self {
        Self {
            workspace: None,
            profile: None,
            policy: Safety::default(),
            passive: false,
            transport: "BIP",
        }
    }
}

fn context(options: &SafetyOptions, local: &str) -> Context {
    Context {
        profile: options.profile.clone(),
        transport: options.transport,
        local: local.into(),
    }
}

/// Open the ledger alone, to list it without a client (a passive session).
pub(crate) async fn ready_offline(sink: &EventSink, options: &SafetyOptions) {
    let ledger = options
        .workspace
        .as_deref()
        .and_then(|dir| Ledger::open(dir).ok());
    tell(
        sink,
        SafetyEvent::Ready {
            policy: options.policy.clone(),
            workspace: false,
            run: run_id().into(),
            ledger: ledger.map(|l| l.entries().to_vec()).unwrap_or_default(),
        },
    )
    .await;
}

/// Keep the ledger's entries on exit with no connected session (passive,
/// or disconnected): flag them in the ledger and record it. Without the
/// workspace's lock the entries can't be flagged, and the record says so.
pub(crate) async fn keep_offline(options: &SafetyOptions, level: Level) {
    let Some(dir) = options.workspace.as_deref() else {
        return;
    };
    let audit = Audit::open(dir, context(options, "-"));
    let kept = lock(dir, "tui").and_then(|held| {
        let mut ledger = Ledger::open(dir)?;
        ledger.keep_all();
        ledger.save()?;
        drop(held);
        Ok(ledger
            .entries()
            .iter()
            .map(Entry::label)
            .collect::<Vec<_>>())
    });
    let fields = match &kept {
        Ok(labels) => json!({ "entries": labels, "flagged": true }),
        Err(why) => {
            let labels: Vec<String> = Ledger::open(dir)
                .map(|l| l.entries().iter().map(Entry::label).collect())
                .unwrap_or_default();
            json!({ "entries": labels, "flagged": false, "why": why })
        }
    };
    match audit {
        Ok(audit) => {
            if let Err(error) = audit.record(level, "kept", fields).await {
                tracing::error!("{error}");
            }
        }
        Err(error) => tracing::error!("{error}"),
    }
}

async fn tell(sink: &EventSink, event: SafetyEvent) {
    sink.deliver(WorkerEvent::Safety(event)).await;
}

/// The session's arming: until when, and the reason given.
type Armed = Option<(Instant, String)>;

/// The TUI as the desk's caller: its level from the session's arming, its
/// records carrying the session's reason, and the session's end.
struct Session<'a> {
    options: &'a SafetyOptions,
    armed: &'a Armed,
    sink: &'a EventSink,
    /// Why there is no ledger, which comes before the level in a refusal.
    missing: Option<&'a str>,
}

impl Session<'_> {
    fn armed(&self) -> bool {
        self.armed
            .as_ref()
            .is_some_and(|(until, _)| Instant::now() < *until)
    }
}

impl Caller for Session<'_> {
    fn level(&self) -> Level {
        if self.options.passive {
            Level::Passive
        } else if self.armed() {
            Level::Armed {
                dangerous: self.options.policy.dangerous,
            }
        } else if self.armed.is_some() {
            Level::Expired
        } else {
            Level::ReadOnly
        }
    }

    fn refusal(&self) -> Option<String> {
        if self.options.passive {
            Some(PASSIVE.into())
        } else if let Some(why) = self.missing {
            Some(why.into())
        } else if !self.level().permits(Class::Write) {
            Some("read-only: press W to arm a write session".into())
        } else {
            None
        }
    }

    fn fields(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        if let Some((_, reason)) = self.armed {
            fields.insert("session_reason".into(), json!(reason));
        }
        fields
    }

    fn gone(&self) -> bool {
        self.sink.is_cancelled()
    }

    fn going(&self) -> Waiting<'_> {
        Box::pin(self.sink.cancelled())
    }

    fn ledger_changed<'a>(&'a self, entries: &'a [Entry]) -> Waiting<'a> {
        // The exit guard counts a write in flight.
        Box::pin(tell(self.sink, SafetyEvent::Ledger(entries.to_vec())))
    }
}

/// The locked desk, and the session's arming.
struct Runner<'a> {
    options: &'a SafetyOptions,
    desk: Desk,
    armed: Armed,
}

/// Serve write session requests until `commands` closes.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<SafetyCommand>,
    rate: &RequestLimits,
    options: &SafetyOptions,
    local: &str,
) {
    let mut runner = Runner {
        options,
        desk: Desk::open(
            options.workspace.as_deref(),
            context(options, local),
            Origin::default(),
        ),
        armed: None,
    };
    let caller = Session {
        options,
        armed: &None,
        sink,
        missing: None,
    };
    runner.desk.settle_abandoned(&caller).await;
    let ledger = match (runner.desk.ledger(), &options.workspace) {
        (Some(ledger), _) => ledger.entries().to_vec(),
        // Locked by another session: still list it.
        (None, Some(dir)) => Ledger::open(dir)
            .map(|l| l.entries().to_vec())
            .unwrap_or_default(),
        (None, None) => Vec::new(),
    };
    tell(
        sink,
        SafetyEvent::Ready {
            policy: options.policy.clone(),
            workspace: options.workspace.is_some(),
            run: run_id().into(),
            ledger,
        },
    )
    .await;
    while let Some(command) = commands.recv().await {
        runner.handle(client, sink, rate, command).await;
    }
}

/// The outcomes as the app shows them.
fn outcomes(sent: Vec<Sent>) -> Vec<(String, Outcome)> {
    sent.into_iter().map(|s| (s.label, s.outcome)).collect()
}

impl<'a> Runner<'a> {
    fn caller<'s>(&'s self, sink: &'s EventSink) -> Session<'s> {
        Session {
            options: self.options,
            armed: &self.armed,
            sink,
            missing: self.desk.missing(),
        }
    }

    async fn refuse(
        &self,
        sink: &EventSink,
        req: Option<u64>,
        event: &str,
        reason: String,
        mut fields: Value,
    ) {
        tracing::warn!("{event} refused: {reason}");
        if let Value::Object(map) = &mut fields {
            map.insert(
                "outcome".into(),
                json!({ "result": "refused", "why": reason }),
            );
        }
        self.desk.record(&self.caller(sink), event, fields).await;
        tell(sink, SafetyEvent::Refused { req, reason }).await;
    }

    async fn handle<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RequestLimits,
        command: SafetyCommand,
    ) {
        match command {
            SafetyCommand::Arm { reason, minutes } => {
                let refusal = if self.options.passive {
                    Some(PASSIVE.to_string())
                } else if self.options.policy.writes == Writes::Never {
                    Some("this profile sets writes = \"never\"".into())
                } else if let Some(why) = self.desk.missing() {
                    Some(why.to_string())
                } else if !(1..=MAX_MINUTES).contains(&minutes) {
                    Some(format!("a session lasts 1 to {MAX_MINUTES} minutes"))
                } else {
                    None
                };
                if let Some(why) = refusal {
                    let fields = json!({ "reason": reason, "minutes": minutes });
                    return self.refuse(sink, None, "arm", why, fields).await;
                }
                let until = Instant::now() + Duration::from_secs(u64::from(minutes) * 60);
                self.armed = Some((until, reason.clone()));
                let fields = json!({ "reason": reason, "minutes": minutes });
                self.desk.record(&self.caller(sink), "arm", fields).await;
                tell(sink, SafetyEvent::Armed { minutes }).await;
            }
            SafetyCommand::Disarm { expired } => {
                if self.armed.is_some() {
                    let why = if expired { "expired" } else { "user" };
                    let fields = json!({ "why": why });
                    self.desk.record(&self.caller(sink), "disarm", fields).await;
                    self.armed = None;
                }
                tell(sink, SafetyEvent::Disarmed).await;
            }
            SafetyCommand::Declined {
                change: Change::Write(preview),
            } => {
                let items: Vec<Value> = preview.items.iter().map(item_json).collect();
                let fields = json!({ "items": items, "tier": preview.gate.name() });
                self.desk
                    .record(&self.caller(sink), "declined", fields)
                    .await;
            }
            SafetyCommand::Declined {
                change: Change::Device(request),
            } => {
                let fields = request.audit_json();
                self.desk
                    .record(&self.caller(sink), "declined", fields)
                    .await;
            }
            SafetyCommand::Manage { req, request } => {
                self.manage(client, sink, rate, req, *request).await;
            }
            SafetyCommand::Prepare { req, specs, batch } => {
                let caller = self.caller(sink);
                if let Some(why) = caller.refusal() {
                    return self
                        .refuse(sink, Some(req), "prepare", why, json!({}))
                        .await;
                }
                let dangerous = self.options.policy.dangerous;
                let prepared = self
                    .desk
                    .prepare(client, rate, &caller, &specs, batch, dangerous)
                    .await;
                let event = match prepared {
                    Ok(preview) => SafetyEvent::Preview {
                        req,
                        preview: Box::new(preview),
                    },
                    Err(reason) => SafetyEvent::Refused {
                        req: Some(req),
                        reason,
                    },
                };
                tell(sink, event).await;
            }
            SafetyCommand::Execute { req, preview } => {
                let dangerous = self.options.policy.dangerous;
                let refusal = self
                    .desk
                    .write_refusal(&self.caller(sink), &preview, dangerous);
                if let Some(why) = refusal {
                    let items: Vec<Value> = preview.items.iter().map(item_json).collect();
                    let fields = json!({ "items": items, "tier": preview.gate.name() });
                    return self.refuse(sink, Some(req), "write", why, fields).await;
                }
                let caller = Session {
                    options: self.options,
                    armed: &self.armed,
                    sink,
                    missing: None,
                };
                let sent = self
                    .desk
                    .execute(client, rate, &caller, &preview, dangerous)
                    .await;
                // The ledger first, so the UI never shows a result without it.
                tell(sink, SafetyEvent::Ledger(self.desk.entries())).await;
                let outcomes = outcomes(sent);
                tell(sink, SafetyEvent::Done { req, outcomes }).await;
            }
            SafetyCommand::Revert { req } => {
                let refusal = if self.options.passive {
                    Some(PASSIVE.to_string())
                } else {
                    self.desk.missing().map(str::to_string)
                };
                if let Some(why) = refusal {
                    return self.refuse(sink, Some(req), "revert", why, json!({})).await;
                }
                let caller = Session {
                    options: self.options,
                    armed: &self.armed,
                    sink,
                    missing: None,
                };
                self.desk.revert(client, rate, &caller, |_| true).await;
                let left = self.desk.entries();
                let failed = left.len();
                tell(sink, SafetyEvent::Ledger(left)).await;
                tell(sink, SafetyEvent::Reverted { req, failed }).await;
            }
            SafetyCommand::Keep => {
                let labels: Vec<String> = self.desk.entries().iter().map(Entry::label).collect();
                let flagged = match self.desk.ledger_mut() {
                    Some(ledger) => {
                        ledger.keep_all();
                        self.desk.save()
                    }
                    None => false,
                };
                let mut fields = json!({ "entries": labels, "flagged": flagged });
                if !flagged {
                    fields["why"] = json!(self.desk.missing().unwrap_or_default());
                }
                self.desk.record(&self.caller(sink), "kept", fields).await;
                tell(sink, SafetyEvent::Ledger(self.desk.entries())).await;
            }
        }
    }
}

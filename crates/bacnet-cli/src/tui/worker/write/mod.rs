//! The write session's worker side: it checks every request against the
//! session level and the profile's policy again, reads the pre-image,
//! sends the writes, and owns the ledger and the audit log.
//!
//! [`run`] is polled beside the session loop under `join!`, like the
//! browser and the watch list, and serves one request at a time. It does
//! not watch for the session's cancellation while a write is out: a write
//! already sent always gets its outcome into the ledger and the audit log,
//! even if the UI never hears of it. Requests still queued when the session
//! ends are recorded as not sent.
//!
//! While it runs it holds the workspace's lock ([`lock`]), so a second
//! session on the same profile can't arm.

mod send;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Instant;

use self::send::{find, gate_name, item_json, relinquishes, send, unchanged};
use super::browse::limits::RateCap;
use super::browse::read::{unreachable, Halt, Reader, Wanted};
use super::EventSink;
use crate::core::class::Class;
use crate::tui::message::WorkerEvent;
use crate::tui::profile::{Safety, Writes};
use crate::tui::safety::audit::{utc_now, Audit, Context};
use crate::tui::safety::ledger::{Entry, Ledger};
use crate::tui::safety::lock::{lock, WorkspaceLock};
use crate::tui::safety::plan::{self, PreRead, PRE_READ, PRIORITY_6};
use crate::tui::safety::{
    Gate, Level, Outcome, Planned, Preview, SafetyCommand, SafetyEvent, WriteSpec, MAX_MINUTES,
};

/// Why a passive session refuses anything that transmits.
pub(crate) const PASSIVE: &str = "passive session: nothing is transmitted";

/// Why a stale preview is not sent.
pub(crate) const STALE: &str = "changed since preview, review again";

/// Why revert leaves an entry whose old value wasn't read before the write.
pub(crate) const UNKNOWN_PRE_IMAGE: &str =
    "the old value was not read before the write, so there is nothing to put back";

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

/// This process's run, which marks the ledger entries it writes, so the
/// next launch can tell which ones an earlier run left.
pub(crate) fn run_id() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(|| {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("{}-{millis}", std::process::id())
    })
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
    let kept = lock(dir).and_then(|held| {
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

/// The locked ledger, the audit log, and the session's arming.
struct Runner<'a> {
    options: &'a SafetyOptions,
    ledger: Option<Ledger>,
    audit: Option<Audit>,
    _lock: Option<WorkspaceLock>,
    /// Why there is no ledger to write to.
    missing: String,
    armed: Option<(Instant, String)>,
}

/// Serve write session requests until `commands` closes.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<SafetyCommand>,
    rate: &RateCap,
    options: &SafetyOptions,
    local: &str,
) {
    let mut runner = Runner::open(options, local);
    runner.settle_abandoned().await;
    let ledger = match (&runner.ledger, &options.workspace) {
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

impl<'a> Runner<'a> {
    fn open(options: &'a SafetyOptions, local: &str) -> Self {
        let mut runner = Self {
            options,
            ledger: None,
            audit: None,
            _lock: None,
            missing: String::new(),
            armed: None,
        };
        let Some(dir) = &options.workspace else {
            runner.missing =
                "write sessions need a profile: its workspace holds the ledger and the audit log"
                    .into();
            return runner;
        };
        match Audit::open(dir, context(options, local)) {
            Ok(audit) => runner.audit = Some(audit),
            Err(error) => {
                tracing::error!("{error}");
                runner.missing = error;
                return runner;
            }
        }
        match lock(dir).and_then(|held| Ok((held, Ledger::open(dir)?))) {
            Ok((held, ledger)) => {
                runner._lock = Some(held);
                runner.ledger = Some(ledger);
            }
            Err(error) => {
                tracing::warn!("{error}");
                runner.missing = error;
            }
        }
        runner
    }

    /// Entries an earlier run left mid-request: their outcome is unknown.
    async fn settle_abandoned(&mut self) {
        let Some(ledger) = &mut self.ledger else {
            return;
        };
        let settled = ledger.settle_abandoned(run_id());
        if settled.is_empty() {
            return;
        }
        self.save();
        for entry in settled {
            let fields = json!({
                "target": entry.label(),
                "priority": entry.priority,
                "outcome": { "result": "unknown", "why": "session ended" },
            });
            self.audit("unknown", fields).await;
        }
    }

    fn entries(&self) -> Vec<Entry> {
        self.ledger
            .as_ref()
            .map(|ledger| ledger.entries().to_vec())
            .unwrap_or_default()
    }

    fn armed(&self) -> bool {
        self.armed
            .as_ref()
            .is_some_and(|(until, _)| Instant::now() < *until)
    }

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

    async fn audit(&self, event: &str, mut fields: Value) {
        let level = self.level();
        if let (Some((_, reason)), Value::Object(map)) = (&self.armed, &mut fields) {
            map.entry("session_reason").or_insert(json!(reason));
        }
        if let Some(audit) = &self.audit {
            if let Err(error) = audit.record(level, event, fields).await {
                tracing::error!("{error}");
            }
        }
    }

    /// Save the ledger; false (and logged) when that failed.
    fn save(&self) -> bool {
        let Some(ledger) = &self.ledger else {
            return false;
        };
        let saved = ledger.save();
        if let Err(error) = &saved {
            tracing::error!("{error}");
        }
        saved.is_ok()
    }

    /// Why a write can't go ahead now, if it can't.
    fn refusal(&self) -> Option<String> {
        if self.options.passive {
            Some(PASSIVE.into())
        } else if self.ledger.is_none() {
            Some(self.missing.clone())
        } else if !self.level().permits(Class::Write) {
            Some("read-only: press W to arm a write session".into())
        } else {
            None
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
        self.audit(event, fields).await;
        tell(sink, SafetyEvent::Refused { req, reason }).await;
    }

    async fn handle<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RateCap,
        command: SafetyCommand,
    ) {
        match command {
            SafetyCommand::Arm { reason, minutes } => {
                let refusal = if self.options.passive {
                    Some(PASSIVE.to_string())
                } else if self.options.policy.writes == Writes::Never {
                    Some("this profile sets writes = \"never\"".into())
                } else if self.ledger.is_none() {
                    Some(self.missing.clone())
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
                self.audit("arm", json!({ "reason": reason, "minutes": minutes }))
                    .await;
                tell(sink, SafetyEvent::Armed { minutes }).await;
            }
            SafetyCommand::Disarm { expired } => {
                if self.armed.is_some() {
                    let why = if expired { "expired" } else { "user" };
                    self.audit("disarm", json!({ "why": why })).await;
                    self.armed = None;
                }
                tell(sink, SafetyEvent::Disarmed).await;
            }
            SafetyCommand::Declined { preview } => {
                let items: Vec<Value> = preview.items.iter().map(item_json).collect();
                let fields = json!({ "items": items, "tier": gate_name(&preview.gate) });
                self.audit("declined", fields).await;
            }
            SafetyCommand::Prepare { req, specs, batch } => {
                if let Some(why) = self.refusal() {
                    return self
                        .refuse(sink, Some(req), "prepare", why, json!({}))
                        .await;
                }
                let event = match self.prepare(client, sink, rate, &specs, batch).await {
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
                if let Some(why) = self.write_refusal(&preview) {
                    let items: Vec<Value> = preview.items.iter().map(item_json).collect();
                    let fields = json!({ "items": items, "tier": gate_name(&preview.gate) });
                    return self.refuse(sink, Some(req), "write", why, fields).await;
                }
                let outcomes = self.execute(client, sink, rate, &preview).await;
                // The ledger first, so the UI never shows a result without it.
                tell(sink, SafetyEvent::Ledger(self.entries())).await;
                tell(sink, SafetyEvent::Done { req, outcomes }).await;
            }
            SafetyCommand::Revert { req } => {
                let refusal = if self.options.passive {
                    Some(PASSIVE.to_string())
                } else if self.ledger.is_none() {
                    Some(self.missing.clone())
                } else {
                    None
                };
                if let Some(why) = refusal {
                    return self.refuse(sink, Some(req), "revert", why, json!({})).await;
                }
                let failed = self.revert(client, sink, rate).await;
                tell(sink, SafetyEvent::Ledger(self.entries())).await;
                tell(sink, SafetyEvent::Reverted { req, failed }).await;
            }
            SafetyCommand::Keep => {
                let labels: Vec<String> = self.entries().iter().map(Entry::label).collect();
                let flagged = match &mut self.ledger {
                    Some(ledger) => {
                        ledger.keep_all();
                        self.save()
                    }
                    None => false,
                };
                let mut fields = json!({ "entries": labels, "flagged": flagged });
                if !flagged {
                    fields["why"] = json!(self.missing);
                }
                self.audit("kept", fields).await;
                tell(sink, SafetyEvent::Ledger(self.entries())).await;
            }
        }
    }

    /// Why a confirmed preview may not be sent, checked here again whatever
    /// the app decided.
    fn write_refusal(&self, preview: &Preview) -> Option<String> {
        if let Some(why) = self.refusal() {
            return Some(why);
        }
        if let Gate::Blocked(why) = &preview.gate {
            return Some(why.clone());
        }
        let six = preview.items.iter().any(|i| i.priority == Some(6));
        if six && !self.options.policy.dangerous {
            return Some(PRIORITY_6.into());
        }
        if preview.batch {
            let specs: Vec<&WriteSpec> = preview.items.iter().map(|i| &i.spec).collect();
            return plan::repeated_target(&specs);
        }
        None
    }

    /// Read what `specs` need and plan them. Stops once the session ends.
    async fn prepare<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RateCap,
        specs: &[WriteSpec],
        batch: bool,
    ) -> Result<Preview, String> {
        let device = specs.first().ok_or("nothing to write")?.device;
        if specs.iter().any(|spec| spec.device != device) {
            return Err("a batch goes to one device".into());
        }
        let cancel = AtomicBool::new(false);
        let mut reader = Reader {
            client,
            device,
            rate,
            cancel: &cancel,
            rpm: true,
        };
        let mut reads = Vec::with_capacity(specs.len());
        for spec in specs {
            let mut wanted = PRE_READ.to_vec();
            if !wanted.contains(&spec.property) && spec.index.is_none() {
                wanted.push(spec.property);
            }
            let read = tokio::select! {
                biased;
                () = sink.cancelled() => return Err("the session ended".into()),
                read = reader.properties(spec.object, Wanted::List(&wanted)) => read,
            };
            let rows = match read {
                Ok((rows, _)) => rows,
                Err(Halt::Device(error)) => {
                    return Err(format!("reading {}: {error}", spec.object))
                }
                Err(Halt::Cancelled) => return Err("cancelled".into()),
            };
            // An indexed write's pre-image is its element, read alone.
            // Index 0 (a resize) has no pre-image to read.
            let element = match spec.index {
                None | Some(0) => None,
                Some(index) => {
                    let read = tokio::select! {
                        biased;
                        () = sink.cancelled() => return Err("the session ended".into()),
                        read = reader.rp(spec.object, spec.property, Some(index)) => read,
                    };
                    Some(match read {
                        Ok(bytes) => Ok(bytes),
                        Err(Halt::Device(error)) if unreachable(&error) => {
                            return Err(format!("reading {}: {error}", spec.object))
                        }
                        Err(Halt::Device(error)) => Err(error.to_string()),
                        Err(Halt::Cancelled) => return Err("cancelled".into()),
                    })
                }
            };
            reads.push(PreRead { rows, element });
        }
        let ledger = self.ledger.as_ref();
        let ours = |spec: &WriteSpec, priority, now: &[u8]| {
            ledger.is_some_and(|l| l.owns(spec, priority, now))
        };
        plan::preview(specs, &reads, ours, self.options.policy.dangerous, batch)
    }

    /// Record every write of `preview` as not sent, and why.
    async fn not_sent(&self, preview: &Preview, why: &str) -> Vec<(String, Outcome)> {
        let outcome = Outcome::NotSent(why.into());
        for item in &preview.items {
            self.audit_write(preview, item, "write", &outcome).await;
        }
        preview
            .items
            .iter()
            .map(|item| (item.label(), outcome.clone()))
            .collect()
    }

    /// Send a confirmed preview and record each outcome: read the targets
    /// again first, then the ledger, then the request.
    async fn execute<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RateCap,
        preview: &Preview,
    ) -> Vec<(String, Outcome)> {
        if sink.is_cancelled() {
            // The session ended before this was sent: record it, send nothing.
            return self.not_sent(preview, "the session ended first").await;
        }
        let specs: Vec<WriteSpec> = preview.items.iter().map(|i| i.spec.clone()).collect();
        let fresh = self
            .prepare(client, sink, rate, &specs, preview.batch)
            .await;
        if !fresh.as_ref().is_ok_and(|fresh| unchanged(preview, fresh)) {
            let items: Vec<Value> = preview.items.iter().map(item_json).collect();
            let detail = fresh
                .err()
                .unwrap_or_else(|| "the pre-image changed".into());
            let fields = json!({ "items": items, "why": "stale", "detail": detail });
            self.audit("declined", fields).await;
            return preview
                .items
                .iter()
                .map(|item| (item.label(), Outcome::NotSent(STALE.into())))
                .collect();
        }
        if let Some(why) = self.refusal() {
            // The session expired or was disarmed during the re-read.
            return self.not_sent(preview, &why).await;
        }
        let Some(ledger) = &mut self.ledger else {
            return Vec::new();
        };
        let backup = ledger.entries().to_vec();
        let at = utc_now();
        let ids: Vec<u64> = preview
            .items
            .iter()
            .map(|item| ledger.begin(item, run_id(), &at))
            .collect();
        if !self.save() {
            // Nothing goes out that the ledger couldn't record first.
            if let Some(ledger) = &mut self.ledger {
                for id in &ids {
                    ledger.finish(*id, Some(false), &backup);
                }
            }
            return self
                .not_sent(preview, "the ledger could not be saved")
                .await;
        }
        for item in &preview.items {
            self.audit_write(preview, item, "sending", &Outcome::NotSent(String::new()))
                .await;
        }
        // The exit guard counts a write in flight; by now it is on disk
        // in both the ledger and the audit log.
        tell(sink, SafetyEvent::Ledger(self.entries())).await;
        rate.acquire().await;
        let outcomes = send(client, preview).await;
        for ((item, id), outcome) in preview.items.iter().zip(&ids).zip(&outcomes) {
            if let Some(ledger) = &mut self.ledger {
                ledger.finish(*id, outcome.applied(), &backup);
            }
            let event = if relinquishes(item) {
                "relinquish"
            } else {
                "write"
            };
            self.audit_write(preview, item, event, outcome).await;
        }
        self.save();
        preview
            .items
            .iter()
            .map(Planned::label)
            .zip(outcomes)
            .collect()
    }

    /// One write's audit record. `sending` carries no outcome yet.
    async fn audit_write(&self, preview: &Preview, item: &Planned, event: &str, outcome: &Outcome) {
        let service = if preview.batch {
            "WritePropertyMultiple"
        } else {
            "WriteProperty"
        };
        let mut fields = item_json(item);
        if let Value::Object(map) = &mut fields {
            map.insert("service".into(), json!(service));
            map.insert("tier".into(), json!(gate_name(&preview.gate)));
            if event != "sending" {
                map.insert("outcome".into(), outcome.json());
            }
        }
        self.audit(event, fields).await;
    }

    /// Undo every entry, latest write first; returns how many are left.
    async fn revert<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RateCap,
    ) -> usize {
        let entries = self
            .ledger
            .as_ref()
            .map(Ledger::newest_first)
            .unwrap_or_default();
        for entry in entries {
            let outcome = match (entry.undo(), entry.object()) {
                _ if sink.is_cancelled() => Outcome::NotSent("the session ended first".into()),
                (Some(undo), Some(object)) => {
                    find(client, entry.device).await;
                    rate.acquire().await;
                    let result = client
                        .write_property_to_device(
                            entry.device,
                            object,
                            entry.property(),
                            entry.index,
                            undo.bytes.clone(),
                            entry.priority,
                        )
                        .await;
                    Outcome::of(&result)
                }
                _ => Outcome::NotSent(UNKNOWN_PRE_IMAGE.into()),
            };
            if outcome.applied() == Some(true) {
                if let Some(ledger) = &mut self.ledger {
                    ledger.remove(entry.id);
                }
                self.save();
            }
            let fields = json!({
                "target": entry.label(),
                "service": "WriteProperty",
                "priority": entry.priority,
                "params": { "value": entry.undo().map(|u| u.text) },
                "pre_image": { "written": entry.written.text },
                "outcome": outcome.json(),
            });
            self.audit("revert", fields).await;
        }
        self.entries().len()
    }
}

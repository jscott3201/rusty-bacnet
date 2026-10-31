//! The engine half of a write, shared by the TUI's write worker and the MCP
//! server (#2068): read what a write needs and plan it, send a confirmed
//! preview through the ledger and the audit log, and revert ledger entries.
//!
//! A [`Desk`] holds a workspace's ledger, its audit log and its lock. The
//! TUI opens one for its whole session; the MCP server opens one around
//! each write and lets it go after, so the TUI can take the workspace when
//! the agent is idle. The front end says who it is, what level it is at
//! and when it has gone through a [`Caller`]; the desk checks the level
//! again before it sends anything.
//!
//! Sending never watches for the caller going away once a request is out:
//! a write already sent always gets its outcome into the ledger and the
//! audit log, even if nobody hears of it.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use serde_json::{Map, Value, json};

use super::audit::{Audit, Context, utc_now};
use super::ledger::{Entry, Ledger, Origin};
use super::lock::{WorkspaceLock, lock};
use super::plan::{self, PRE_READ, PreRead};
use super::send::{find, item_json, send};
use super::{Gate, Level, Outcome, Planned, Preview, WriteSpec, run_id};
use crate::engine::{Halt, Reader, Wanted, unreachable};
use crate::limits::RequestLimits;

/// Why a passive session refuses anything that transmits.
pub const PASSIVE: &str = "passive session: nothing is transmitted";

/// Why a stale preview is not sent.
pub const STALE: &str = "changed since preview, review again";

/// Why revert leaves an entry whose old value wasn't read before the write.
pub const UNKNOWN_PRE_IMAGE: &str =
    "the old value was not read before the write, so there is nothing to put back";

/// Why a request isn't sent once the caller has gone.
pub const ENDED: &str = "the session ended first";

/// A future the desk waits on, boxed so a [`Caller`] stays object-safe and
/// a write can run on a task of its own.
pub type Waiting<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// The front end a desk works for: its level, the fields its records
/// carry, and whether it is still there.
pub trait Caller: Sync {
    /// The session's level now.
    fn level(&self) -> Level;

    /// Why a write may not go out now, if it may not: checked before the
    /// pre-image is read and again just before sending (the session may
    /// have expired during the read). The desk checks the ledger itself.
    fn refusal(&self) -> Option<String>;

    /// Fields every audit record of this caller carries (the TUI's
    /// session reason; MCP's tool, client and approval). A record's own
    /// field of the same name wins.
    fn fields(&self) -> Map<String, Value> {
        Map::new()
    }

    /// True once the caller has gone (the TUI's session ended).
    fn gone(&self) -> bool {
        false
    }

    /// Resolves when the caller goes: a pre-image read stops then.
    fn going(&self) -> Waiting<'_> {
        Box::pin(std::future::pending())
    }

    /// The ledger changed: written as `sending`, before the request goes.
    fn ledger_changed<'a>(&'a self, _entries: &'a [Entry]) -> Waiting<'a> {
        Box::pin(async {})
    }
}

/// One write's result: its target, outcome, and the ledger entry that
/// keeps its pre-image (none when the write put the pre-image back, was
/// refused, or an entry for the whole array covers it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// `device 100 analog-output 1 present-value @8`.
    pub label: String,
    /// How it ended.
    pub outcome: Outcome,
    /// The ledger entry, while it is in effect.
    pub entry: Option<u64>,
}

/// A workspace's ledger, audit log and lock.
pub struct Desk {
    ledger: Option<Ledger>,
    audit: Option<Audit>,
    _lock: Option<WorkspaceLock>,
    /// Why there is no ledger to write to.
    missing: String,
}

/// Why a desk without a workspace can't write.
pub const NO_WORKSPACE: &str =
    "write sessions need a profile: its workspace holds the ledger and the audit log";

impl Desk {
    /// Open `workspace`'s audit log, then take its lock and open its
    /// ledger, for `origin`'s front end. Without a workspace, or when
    /// another process holds the lock, the desk has no ledger and
    /// [`missing`](Self::missing) says why; the audit log is still open.
    pub fn open(workspace: Option<&Path>, context: Context, origin: Origin) -> Self {
        let mut desk = Self {
            ledger: None,
            audit: None,
            _lock: None,
            missing: String::new(),
        };
        let Some(dir) = workspace else {
            desk.missing = NO_WORKSPACE.into();
            return desk;
        };
        match Audit::open(dir, context) {
            Ok(audit) => {
                desk.audit =
                    Some(audit.written_by(origin.front_end.clone(), origin.principal.clone()));
            }
            Err(error) => {
                tracing::error!("{error}");
                desk.missing = error;
                return desk;
            }
        }
        let front_end = origin.front_end.name().to_string();
        match lock(dir, &front_end).and_then(|held| Ok((held, Ledger::open(dir)?))) {
            Ok((held, ledger)) => {
                desk._lock = Some(held);
                desk.ledger = Some(ledger.written_by(origin));
            }
            Err(error) => {
                tracing::warn!("{error}");
                desk.missing = error;
            }
        }
        desk
    }

    /// The ledger, while this desk holds the workspace.
    pub fn ledger(&self) -> Option<&Ledger> {
        self.ledger.as_ref()
    }

    /// Flag every entry as kept on exit and save the ledger; false when
    /// there is no ledger or it couldn't be saved. The caller records it.
    pub fn keep_all(&mut self) -> bool {
        match &mut self.ledger {
            Some(ledger) => {
                ledger.keep_all();
                self.save()
            }
            None => false,
        }
    }

    /// The ledger's entries, oldest first; none without a ledger.
    pub fn entries(&self) -> Vec<Entry> {
        self.ledger
            .as_ref()
            .map(|ledger| ledger.entries().to_vec())
            .unwrap_or_default()
    }

    /// Why there is no ledger: no workspace, the audit log couldn't be
    /// opened, or another process holds the lock.
    pub fn missing(&self) -> Option<&str> {
        self.ledger.is_none().then_some(self.missing.as_str())
    }

    /// Append an audit record for `caller`, with its fields; a failure is
    /// logged, never fatal.
    pub async fn record(&self, caller: &dyn Caller, event: &str, mut fields: Value) {
        if let Value::Object(map) = &mut fields {
            for (key, value) in caller.fields() {
                map.entry(key).or_insert(value);
            }
        }
        if let Some(audit) = &self.audit
            && let Err(error) = audit.record(caller.level(), event, fields).await
        {
            tracing::error!("{error}");
        }
    }

    /// Save the ledger; false (and logged) when that failed.
    pub fn save(&self) -> bool {
        let Some(ledger) = &self.ledger else {
            return false;
        };
        let saved = ledger.save();
        if let Err(error) = &saved {
            tracing::error!("{error}");
        }
        saved.is_ok()
    }

    /// Entries left mid-request with no writer: their outcome is unknown.
    /// A writer in flight holds the workspace's lock, which this desk
    /// holds, so every `sending` entry is one whose session ended or whose
    /// task failed before the answer, in this run or an earlier one.
    pub async fn settle_abandoned(&mut self, caller: &dyn Caller) {
        let Some(ledger) = &mut self.ledger else {
            return;
        };
        let settled = ledger.settle_abandoned();
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
            self.record(caller, "unknown", fields).await;
        }
    }

    /// Why a confirmed preview may not be sent, checked here again whatever
    /// the front end decided: the caller's refusal, no ledger, a blocked
    /// change, priority 6 without `dangerous`, or a batch naming a target
    /// twice.
    pub fn write_refusal(
        &self,
        caller: &dyn Caller,
        preview: &Preview,
        dangerous: bool,
    ) -> Option<String> {
        if let Some(why) = caller.refusal() {
            return Some(why);
        }
        if let Some(why) = self.missing() {
            return Some(why.to_string());
        }
        if let Gate::Blocked(why) = &preview.gate {
            return Some(why.clone());
        }
        let six = preview.items.iter().any(|i| i.priority == Some(6));
        if six && !dangerous {
            return Some(plan::PRIORITY_6.into());
        }
        if preview.batch {
            let specs: Vec<&WriteSpec> = preview.items.iter().map(|i| &i.spec).collect();
            return plan::repeated_target(&specs);
        }
        None
    }

    /// Read what `specs` need and plan them, the slots this ledger holds
    /// counted as ours. Stops once the caller goes.
    pub async fn prepare<T: TransportPort + 'static>(
        &self,
        client: &BACnetClient<T>,
        rate: &RequestLimits,
        caller: &dyn Caller,
        specs: &[WriteSpec],
        batch: bool,
        dangerous: bool,
    ) -> Result<Preview, String> {
        let ledger = self.ledger.as_ref();
        prepare(client, rate, caller, specs, batch, dangerous, ledger).await
    }

    /// Record every write of `preview` as not sent, and why.
    pub(crate) async fn not_sent(
        &self,
        caller: &dyn Caller,
        preview: &Preview,
        why: &str,
    ) -> Vec<Sent> {
        let outcome = Outcome::NotSent(why.into());
        for item in &preview.items {
            self.audit_write(caller, preview, item, "write", &outcome)
                .await;
        }
        preview
            .items
            .iter()
            .map(|item| Sent {
                label: item.label(),
                outcome: outcome.clone(),
                entry: None,
            })
            .collect()
    }

    /// Send a confirmed preview and record each outcome: read the targets
    /// again first (a preview that no longer matches is not sent, and is
    /// recorded as declined, stale), then the ledger, then the request.
    pub async fn execute<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        rate: &RequestLimits,
        caller: &dyn Caller,
        preview: &Preview,
        dangerous: bool,
    ) -> Vec<Sent> {
        if caller.gone() {
            // The session ended before this was sent: record it, send nothing.
            return self.not_sent(caller, preview, ENDED).await;
        }
        let specs: Vec<WriteSpec> = preview.items.iter().map(|i| i.spec.clone()).collect();
        let fresh = self
            .prepare(client, rate, caller, &specs, preview.batch, dangerous)
            .await;
        if !fresh.as_ref().is_ok_and(|fresh| preview.unchanged(fresh)) {
            let items: Vec<Value> = preview.items.iter().map(item_json).collect();
            let detail = fresh
                .err()
                .unwrap_or_else(|| "the pre-image changed".into());
            let fields = json!({ "items": items, "why": "stale", "detail": detail });
            self.record(caller, "declined", fields).await;
            return preview
                .items
                .iter()
                .map(|item| Sent {
                    label: item.label(),
                    outcome: Outcome::NotSent(STALE.into()),
                    entry: None,
                })
                .collect();
        }
        self.commit(client, rate, caller, preview).await
    }

    /// Send `preview`, read just now, through the ledger and the audit
    /// log: check the caller again (the session may have expired during
    /// the read), record each write as `sending` (nothing goes out that the
    /// ledger couldn't record first), send, then settle and record each
    /// outcome. [`execute`](Self::execute) reads the targets again first; a
    /// front end that has just read them calls this directly.
    pub async fn commit<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        rate: &RequestLimits,
        caller: &dyn Caller,
        preview: &Preview,
    ) -> Vec<Sent> {
        if let Some(why) = caller.refusal() {
            // The session expired or was disarmed during the re-read.
            return self.not_sent(caller, preview, &why).await;
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
            // The save may have failed only at the directory sync, with
            // the `sending` rows already in the file: put the file back as
            // it was, so it doesn't claim a write that never went. Should
            // this save fail before its rename too, the next holder of the
            // lock settles those rows as unknown.
            self.save();
            return self
                .not_sent(caller, preview, "the ledger could not be saved")
                .await;
        }
        for item in &preview.items {
            self.audit_write(
                caller,
                preview,
                item,
                "sending",
                &Outcome::NotSent(String::new()),
            )
            .await;
        }
        // By now the write is on disk in both the ledger and the audit log.
        caller.ledger_changed(&self.entries()).await;
        // One turn at the device for the preview's writes, sent one after
        // another (#1791).
        let turn = rate.turn(preview.device).await;
        let outcomes = send(client, preview).await;
        drop(turn);
        for ((item, id), outcome) in preview.items.iter().zip(&ids).zip(&outcomes) {
            if let Some(ledger) = &mut self.ledger {
                ledger.finish(*id, outcome.applied(), &backup);
            }
            let event = if item.relinquishes() {
                "relinquish"
            } else {
                "write"
            };
            self.audit_write(caller, preview, item, event, outcome)
                .await;
        }
        self.save();
        let held = |id: &u64| {
            self.ledger
                .as_ref()
                .is_some_and(|l| l.entries().iter().any(|e| e.id == *id))
        };
        preview
            .items
            .iter()
            .zip(ids)
            .zip(outcomes)
            .map(|((item, id), outcome)| Sent {
                label: item.label(),
                outcome,
                entry: held(&id).then_some(id),
            })
            .collect()
    }

    /// One write's audit record. `sending` carries no outcome yet.
    pub(crate) async fn audit_write(
        &self,
        caller: &dyn Caller,
        preview: &Preview,
        item: &Planned,
        event: &str,
        outcome: &Outcome,
    ) {
        let mut fields = item_json(item);
        if let Value::Object(map) = &mut fields {
            map.insert("service".into(), json!(preview.service()));
            map.insert("tier".into(), json!(preview.gate.name()));
            if event != "sending" {
                map.insert("outcome".into(), outcome.json());
            }
        }
        self.record(caller, event, fields).await;
    }

    /// Undo the entries `chosen` picks, latest write first, each recorded;
    /// each with its outcome. An entry undone leaves the ledger.
    pub async fn revert<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        rate: &RequestLimits,
        caller: &dyn Caller,
        chosen: impl Fn(&Entry) -> bool,
    ) -> Vec<(Entry, Outcome)> {
        let entries: Vec<Entry> = self
            .ledger
            .as_ref()
            .map(Ledger::newest_first)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| chosen(entry))
            .collect();
        let mut done = Vec::with_capacity(entries.len());
        for entry in entries {
            let outcome = match (entry.undo(), entry.object()) {
                _ if caller.gone() => Outcome::NotSent(ENDED.into()),
                (Some(undo), Some(object)) => {
                    find(client, entry.device).await;
                    let turn = rate.turn(entry.device).await;
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
                    drop(turn);
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
            self.record(caller, "revert", fields).await;
            done.push((entry, outcome));
        }
        done
    }
}

/// Read what `specs` need and plan them, the slots `ledger` holds counted
/// as ours (a front end without the workspace's lock reads the file).
/// Stops once the caller goes.
pub async fn prepare<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    caller: &dyn Caller,
    specs: &[WriteSpec],
    batch: bool,
    dangerous: bool,
    ledger: Option<&Ledger>,
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
            () = caller.going() => return Err("the session ended".into()),
            read = reader.properties(spec.object, Wanted::List(&wanted)) => read,
        };
        let rows = match read {
            Ok((rows, _)) => rows,
            Err(Halt::Device(error)) => {
                return Err(format!("reading {}: {error}", spec.object));
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
                    () = caller.going() => return Err("the session ended".into()),
                    read = reader.rp(spec.object, spec.property, Some(index)) => read,
                };
                Some(match read {
                    Ok(bytes) => Ok(bytes),
                    Err(Halt::Device(error)) if unreachable(&error) => {
                        return Err(format!("reading {}: {error}", spec.object));
                    }
                    Err(Halt::Device(error)) => Err(error.to_string()),
                    Err(Halt::Cancelled) => return Err("cancelled".into()),
                })
            }
        };
        reads.push(PreRead { rows, element });
    }
    let ours = |spec: &WriteSpec, priority, now: &[u8]| {
        ledger.is_some_and(|l| l.owns(spec, priority, now))
    };
    plan::preview(specs, &reads, ours, dangerous, batch)
}

#[cfg(test)]
#[path = "desk_tests.rs"]
mod tests;

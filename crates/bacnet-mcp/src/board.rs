//! What the TUI and its attached agents' sessions share for the agent
//! pane (#2185; design §9): every tool call the sessions answered, in a
//! ring of the latest [`CALLS`], and whether the TUI's write session is
//! armed, which `bacnet://session` reports to the agents.
//!
//! The sessions run on the listener's runtime and the pane on the TUI's
//! loop. A session records a call under a std mutex held only to push the
//! row (the oldest goes, counted, when the ring is full), never across an
//! await; the TUI takes a copy with [`Board::calls_since`], which never
//! waits for the lock, so it never holds up a frame.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::time::{Duration, SystemTime};

use rmcp::ErrorData;
use rmcp::model::CallToolResponse;
use serde_json::{Map, Value};

/// The calls the ring keeps; one more drops the oldest.
pub const CALLS: usize = 500;

/// How a call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOutcome {
    /// It answered.
    Ok,
    /// Refused before anything was sent, with the reason
    /// (`not-armed`, `rate-limited`, …).
    Refused(String),
    /// The person at the TUI declined it, it timed out unanswered, or the
    /// agent withdrew it (`declined`, `withdrawn`).
    Declined(String),
    /// Its preview no longer held when it was to be sent.
    Stale,
    /// Any other error, in a word or two.
    Error(String),
}

/// One tool call of an attached agent's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRow {
    /// When it was answered.
    pub at: SystemTime,
    /// The agent's connection number on the listener.
    pub agent: u64,
    /// The name its client claims, cleaned and capped: only a claim.
    pub client: Option<String>,
    /// The tool.
    pub tool: String,
    /// A short summary of what it was aimed at, cleaned.
    pub target: String,
    /// How it ended.
    pub outcome: CallOutcome,
    /// The requests charged to the call's account: its reads, and for a
    /// write the re-read before it and the writes sent.
    pub requests: u32,
    /// How long the call took.
    pub elapsed: Duration,
}

/// A copy of the ring, for the pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calls {
    /// Newest first.
    pub rows: Vec<CallRow>,
    /// Calls dropped from the ring, oldest first, since the listener
    /// started.
    pub dropped: u64,
    /// Bumped by every call recorded.
    pub seq: u64,
}

#[derive(Debug, Default)]
struct Log {
    rows: VecDeque<CallRow>,
    dropped: u64,
    seq: u64,
}

#[derive(Debug, Default)]
struct Inner {
    log: Mutex<Log>,
    armed: AtomicBool,
}

/// The pane's feed and the TUI's armed state. Cheap to clone: clones
/// share one board.
#[derive(Debug, Clone, Default)]
pub struct Board {
    inner: Arc<Inner>,
}

impl PartialEq for Board {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Board {
    /// Record a call, dropping the oldest when the ring is full.
    pub fn record(&self, row: CallRow) {
        let mut log = self
            .inner
            .log
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if log.rows.len() >= CALLS {
            log.rows.pop_front();
            log.dropped += 1;
        }
        log.rows.push_back(row);
        log.seq += 1;
    }

    /// The calls, newest first, when some were recorded since `seq`;
    /// `None` when none were, or when a session holds the lock this
    /// moment (the caller asks again later rather than wait).
    pub fn calls_since(&self, seq: u64) -> Option<Calls> {
        let log = match self.inner.log.try_lock() {
            Ok(log) => log,
            Err(TryLockError::Poisoned(log)) => log.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        (log.seq != seq).then(|| Calls {
            rows: log.rows.iter().rev().cloned().collect(),
            dropped: log.dropped,
            seq: log.seq,
        })
    }

    /// The TUI's write session is armed, or not any more.
    pub fn set_armed(&self, armed: bool) {
        self.inner.armed.store(armed, Ordering::SeqCst);
    }

    /// Whether the TUI's write session is armed: an agent's write can be
    /// confirmed there now.
    pub fn armed(&self) -> bool {
        self.inner.armed.load(Ordering::SeqCst)
    }
}

/// An attached session's place on the board: the board and its agent's
/// connection number.
#[derive(Debug, Clone)]
pub(crate) struct Pane {
    pub(crate) board: Board,
    pub(crate) agent: u64,
}

impl Pane {
    /// Record the call of `tool` at `target`, from the client that
    /// claims to be `client`, answered `response` after `elapsed`.
    pub(crate) fn record(
        &self,
        (tool, target, client): (&str, String, Option<String>),
        response: &Result<CallToolResponse, ErrorData>,
        elapsed: Duration,
    ) {
        let (outcome, requests) = match response {
            Ok(CallToolResponse::Complete(result)) => {
                let body = result.structured_content.as_ref();
                let requests = body
                    .and_then(|b| b["cost"]["requests"].as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0);
                (outcome(body, result.is_error == Some(true)), requests)
            }
            Ok(_) => (CallOutcome::Error("input-required".into()), 0),
            Err(error) => (
                CallOutcome::Error(crate::untrusted::clean(&error.message, 48).0),
                0,
            ),
        };
        self.board.record(CallRow {
            at: SystemTime::now(),
            agent: self.agent,
            client,
            tool: crate::untrusted::clean(tool, 32).0,
            target,
            outcome,
            requests,
            elapsed,
        });
    }
}

/// The longest target summary, in characters.
const TARGET: usize = 48;

/// What the arguments `args` aim at, in a few words: the device, the
/// first object read or written (and how many more), a network, or the
/// ledger entries of a revert. Agent text, so cleaned and capped; it stops
/// once the cap is reached, never rendering a whole long value first.
pub(crate) fn target(args: &Map<String, Value>) -> String {
    let mut out = String::new();
    let mut push = |label: &str, value: Option<&Value>| {
        if out.chars().count() > TARGET {
            return;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(label);
        match value {
            Some(Value::String(text)) => out.extend(text.chars().take(TARGET)),
            Some(Value::Number(n)) => out.push_str(&n.to_string()),
            Some(Value::Bool(b)) => out.push_str(if *b { "true" } else { "false" }),
            Some(_) => out.push('…'),
            None => {}
        }
    };
    if let Some(device) = args.get("device") {
        push("dev ", Some(device));
    }
    if let Some(network) = args.get("network") {
        push("net ", Some(network));
    }
    if let Some(object) = args.get("object") {
        push("", Some(object));
    }
    for list in ["reads", "writes", "points"] {
        if let Some(Value::Array(items)) = args.get(list)
            && let Some(first) = items.first()
        {
            push("", Some(first.get("object").unwrap_or(first)));
            if items.len() > 1 {
                push(&format!("+{}", items.len() - 1), None);
            }
        }
    }
    if let Some(Value::Array(entries)) = args.get("entries") {
        let mut ids = String::new();
        for entry in entries.iter().take(TARGET) {
            if ids.len() > TARGET {
                break;
            }
            if !ids.is_empty() {
                ids.push(',');
            }
            ids.push_str(&entry.as_u64().map_or("?".into(), |n| n.to_string()));
        }
        push("entries ", Some(&Value::String(ids)));
    }
    if args.get("all") == Some(&Value::Bool(true)) {
        push("all", None);
    }
    crate::untrusted::clean(&out, TARGET).0
}

/// How a call's result `body` (its structured content, `is_error` when it
/// is an error body) ended.
pub(crate) fn outcome(body: Option<&Value>, is_error: bool) -> CallOutcome {
    let Some(body) = body.filter(|_| is_error) else {
        return CallOutcome::Ok;
    };
    let word = |key: &str| body[key].as_str().unwrap_or("failed").to_string();
    match body["error"].as_str() {
        Some("refused") => match body["reason"].as_str() {
            Some("stale") => CallOutcome::Stale,
            Some("declined" | "withdrawn") => CallOutcome::Declined(word("reason")),
            _ => CallOutcome::Refused(word("reason")),
        },
        _ => CallOutcome::Error(word("error")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(n: u64) -> CallRow {
        CallRow {
            at: SystemTime::UNIX_EPOCH,
            agent: n,
            client: None,
            tool: "bacnet_read".into(),
            target: String::new(),
            outcome: CallOutcome::Ok,
            requests: 1,
            elapsed: Duration::ZERO,
        }
    }

    /// The ring keeps the latest [`CALLS`], newest first, and counts the
    /// oldest it drops; an unchanged ring isn't copied again.
    #[test]
    fn the_ring_drops_the_oldest_and_counts_it() {
        let board = Board::default();
        for n in 0..(CALLS as u64 + 3) {
            board.record(row(n));
        }
        let calls = board.calls_since(0).unwrap();
        assert_eq!(calls.rows.len(), CALLS);
        assert_eq!(calls.dropped, 3);
        assert_eq!(calls.rows[0].agent, CALLS as u64 + 2, "newest first");
        assert_eq!(calls.rows[CALLS - 1].agent, 3, "the oldest three went");
        assert_eq!(board.calls_since(calls.seq), None);
    }

    /// The pane never waits for a session's lock: busy, it gets nothing
    /// this time.
    #[test]
    fn a_busy_ring_is_skipped_not_waited_for() {
        let board = Board::default();
        board.record(row(1));
        let held = board.inner.log.lock().unwrap();
        assert_eq!(board.calls_since(0), None);
        drop(held);
        assert!(board.calls_since(0).is_some());
    }

    /// The outcome comes from the error body: a refusal's reason, a
    /// decline, a stale preview, or another error's kind.
    #[test]
    fn outcomes_come_from_the_error_body() {
        let refused = |reason: &str| json!({ "error": "refused", "reason": reason });
        assert_eq!(outcome(Some(&json!({})), false), CallOutcome::Ok);
        assert_eq!(
            outcome(Some(&refused("not-armed")), true),
            CallOutcome::Refused("not-armed".into())
        );
        assert_eq!(
            outcome(Some(&refused("declined")), true),
            CallOutcome::Declined("declined".into())
        );
        assert_eq!(outcome(Some(&refused("stale")), true), CallOutcome::Stale);
        assert_eq!(
            outcome(Some(&json!({ "error": "timeout" })), true),
            CallOutcome::Error("timeout".into())
        );
    }

    /// A huge argument is cut, not rendered whole first.
    #[test]
    fn a_long_target_is_cut() {
        let args = json!({ "device": "x".repeat(10_000), "entries": (0..5000).collect::<Vec<_>>(),
            "reads": [{ "object": { "nested": "y".repeat(10_000) } }] });
        let Value::Object(args) = args else {
            unreachable!()
        };
        assert!(target(&args).chars().count() <= TARGET);
        let args = json!({ "entries": [4, 5] });
        let Value::Object(args) = args else {
            unreachable!()
        };
        assert_eq!(target(&args), "entries 4,5");
    }

    /// The target names the device, the first object and how many more,
    /// cleaned of control characters.
    #[test]
    fn the_target_is_short_and_clean() {
        let args = json!({ "device": 100, "writes": [
            { "object": "ao:1\u{1b}[2J", "value": 1 }, { "object": "ao:2", "value": 2 }] });
        let Value::Object(args) = args else {
            unreachable!()
        };
        let target = target(&args);
        assert!(target.starts_with("dev 100 ao:1"), "{target}");
        assert!(target.ends_with(" +1"), "{target}");
        assert!(!target.contains('\u{1b}'), "{target}");
    }
}

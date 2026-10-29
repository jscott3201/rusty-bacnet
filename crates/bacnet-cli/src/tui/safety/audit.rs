//! The TUI's side of `audit.jsonl`: the shared log (`bacnet_ops::safety::audit`),
//! the [`Secret`] a device management password lives in from the masked
//! prompt to the request, and the record of a confirmed MS/TP trunk join.
//!
//! Secrets never reach the file: a [`Secret`] serialises as `[redacted]`,
//! and the shared log redacts any field named for a secret on the way out.

use std::path::Path;

#[cfg(test)]
pub(crate) use bacnet_ops::safety::audit::FILE;
pub(crate) use bacnet_ops::safety::audit::{Audit, Context, REDACTED, utc};
use serde::{Serialize, Serializer};
use serde_json::json;

use super::Level;

/// A password or other secret: never written or printed. Debug output and
/// serialisation both give [`REDACTED`]; only [`Secret::expose`] gives the
/// text, for the request that carries it. DeviceCommunicationControl and
/// ReinitializeDevice hold their passwords in one from the masked prompt
/// to the request (#1799).
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Secret(String);

impl Secret {
    /// A secret holding `text`; the TUI builds its secrets a character at
    /// a time, so only tests start from whole text.
    #[cfg(test)]
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Add a typed character.
    pub(crate) fn push(&mut self, ch: char) {
        self.0.push(ch);
    }

    /// Erase the last character.
    pub(crate) fn pop(&mut self) {
        self.0.pop();
    }

    /// How many characters it holds: all a mask may show.
    pub(crate) fn chars(&self) -> usize {
        self.0.chars().count()
    }

    /// True when nothing is held.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The text itself, for the request that sends it and nothing else.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(REDACTED)
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(REDACTED)
    }
}

/// The event of a confirmed MS/TP trunk join.
pub(crate) const MSTP_JOIN: &str = "mstp-join";

/// Record a confirmed MS/TP trunk join on profile `profile` (#1862):
/// joining puts a new master in the token ring, a T2 change to a shared
/// bus. Written before the port opens; the session is not armed, so the
/// level is read-only. `thread` is how the station's thread is to run
/// (#501): a join that can't apply it fails after this record.
pub(crate) async fn record_join(
    workspace: &Path,
    profile: &str,
    settings: &crate::mstp::MstpArgs,
    thread: &bacnet_transport::mstp::MstpThreadConfig,
) -> Result<(), String> {
    let context = Context {
        profile: Some(profile.into()),
        transport: "MSTP",
        local: settings.port.clone(),
    };
    let fields = json!({
        "port": settings.port,
        "station": settings.mac,
        "baud": settings.baud,
        "max_master": settings.max_master,
        "max_info_frames": settings.max_info_frames,
        "realtime_priority": thread.realtime_priority,
        "cpu_affinity": thread.cpu_affinity,
    });
    Audit::open(workspace, context)?
        .record(Level::ReadOnly, MSTP_JOIN, fields)
        .await
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[tokio::test]
    async fn records_parse_and_never_hold_a_password() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Audit::open(
            dir.path(),
            Context {
                profile: Some("lab".into()),
                transport: "BIP",
                local: "10.0.0.1:47808".into(),
            },
        )
        .unwrap();
        let secret = Secret::new("hunter2-test-password");
        let fields = json!({ "service": "DeviceCommunicationControl",
            "params": { "password": "hunter2-test-password", "secret": secret } });
        audit
            .record(Level::ReadOnly, "write", fields)
            .await
            .unwrap();
        audit
            .record(
                Level::Armed { dangerous: false },
                "arm",
                json!({ "reason": "ticket 7" }),
            )
            .await
            .unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!text.contains("hunter2"), "{text}");
        let records: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["params"]["password"], REDACTED);
        assert_eq!(records[0]["params"]["secret"], REDACTED);
        assert_eq!(records[1]["level"], "write-session");
        assert_eq!(records[1]["profile"], "lab");
        for key in ["time", "user", "host", "transport", "local", "event"] {
            assert!(records[1].get(key).is_some(), "{key}");
        }
    }
}

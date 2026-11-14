//! `audit.jsonl`: an append-only, client-side record of every change a
//! front end (the TUI, the MCP server) makes and every safety decision
//! around it, one JSON object a line.
//!
//! Each record names when (UTC), who (OS user and host), the front end
//! (`front_end`, and for MCP the `principal`), where (profile, transport
//! and local address), the session level and the event; a write adds its
//! target, service, parameters, priority, pre-image and outcome. Arming,
//! disarming, refusals and declined confirmations are recorded too. Every
//! line is flushed and synced before the next. Front ends append to one
//! file without the workspace's lock: each record is one `write` of a whole
//! line in append mode.
//!
//! Secrets never reach the file: any field whose name holds `password`,
//! `token`, `authorization`, `secret` or `key` is redacted on the way out.
//! This record is separate from BACnet audit reporting (Clause 19.6).

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::{FrontEnd, Level};

/// The log's file in the workspace.
pub const FILE: &str = "audit.jsonl";

/// What a redacted value is written as.
pub const REDACTED: &str = "[redacted]";

/// The words a field name holding a secret contains (#2068 widened them
/// from `password` alone).
pub const SECRET_KEYS: [&str; 5] = ["password", "token", "authorization", "secret", "key"];

/// Replace every value under a key that names a secret ([`SECRET_KEYS`]).
pub fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                let key = key.to_ascii_lowercase();
                if SECRET_KEYS.iter().any(|word| key.contains(word)) {
                    *value = Value::String(REDACTED.into());
                } else {
                    redact(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact),
        _ => {}
    }
}

/// Where the session runs, repeated in every record.
#[derive(Debug, Clone)]
pub struct Context {
    /// The profile, if any.
    pub profile: Option<String>,
    /// `BIP`, `BIP6` or `SC`.
    pub transport: &'static str,
    /// The local address.
    pub local: String,
}

/// The open log. Records are written on the blocking pool (the host name
/// lookup and the sync can stall), one at a time, each awaited.
pub struct Audit {
    file: Arc<Mutex<File>>,
    context: Context,
    front_end: FrontEnd,
    principal: Option<String>,
}

impl Audit {
    /// Open (or create) the log in `workspace` for appending.
    pub fn open(workspace: &Path, context: Context) -> Result<Self, String> {
        let path = workspace.join(FILE);
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            context,
            front_end: FrontEnd::Tui,
            principal: None,
        })
    }

    /// The log, with every record naming `front_end` (the TUI otherwise)
    /// and `principal`.
    pub fn written_by(mut self, front_end: FrontEnd, principal: Option<String>) -> Self {
        self.front_end = front_end;
        self.principal = principal;
        self
    }

    /// Append one record: the common fields, `event`, then `fields`. Done
    /// on the blocking pool; the file's lock is held only there, never
    /// across an await.
    pub async fn record(&self, level: Level, event: &str, fields: Value) -> Result<(), String> {
        let file = Arc::clone(&self.file);
        let context = self.context.clone();
        let mut fields = fields;
        if let Value::Object(map) = &mut fields {
            map.insert("front_end".into(), json!(self.front_end.name()));
            if let Some(principal) = &self.principal {
                map.entry("principal").or_insert(json!(principal));
            }
        }
        let event = event.to_string();
        tokio::task::spawn_blocking(move || {
            let line = line(&context, level, &event, fields);
            let mut file = file
                .lock()
                .map_err(|_| "the audit log's lock is poisoned".to_string())?;
            file.write_all(line.as_bytes())
                .and_then(|()| file.flush())
                .and_then(|()| file.sync_data())
                .map_err(|e| format!("cannot write the audit log: {e}"))
        })
        .await
        .map_err(|e| format!("the audit writer failed: {e}"))?
    }
}

/// One record as a line.
fn line(context: &Context, level: Level, event: &str, fields: Value) -> String {
    let mut record = Map::new();
    record.insert("time".into(), json!(utc_now()));
    record.insert("user".into(), json!(os_user()));
    record.insert("host".into(), json!(host_name()));
    record.insert("profile".into(), json!(context.profile));
    record.insert("transport".into(), json!(context.transport));
    record.insert("local".into(), json!(context.local));
    record.insert("level".into(), json!(level.audit_name()));
    record.insert("event".into(), json!(event));
    if let Value::Object(fields) = fields {
        record.extend(fields);
    }
    let mut record = Value::Object(record);
    redact(&mut record);
    let mut line = record.to_string();
    line.push('\n');
    line
}

/// The OS user: `USER`, `USERNAME` or `LOGNAME`.
fn os_user() -> &'static str {
    static USER: OnceLock<String> = OnceLock::new();
    USER.get_or_init(|| {
        ["USER", "USERNAME", "LOGNAME"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
            .unwrap_or_else(|| "unknown".into())
    })
}

/// The host name: `HOSTNAME` or `COMPUTERNAME`, else the `hostname`
/// command, which Linux, macOS and Windows all have. Looked up once.
fn host_name() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        ["HOSTNAME", "COMPUTERNAME"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
            .or_else(|| {
                let out = std::process::Command::new("hostname").output().ok()?;
                let name = String::from_utf8(out.stdout).ok()?.trim().to_string();
                (out.status.success() && !name.is_empty()).then_some(name)
            })
            .unwrap_or_else(|| "unknown".into())
    })
}

/// Now, as `2026-10-18T12:34:56.789Z`.
pub fn utc_now() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    utc(since.as_secs(), since.subsec_millis())
}

/// `secs` after the Unix epoch as an ISO 8601 UTC time.
pub fn utc(secs: u64, millis: u32) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_times_are_iso_8601() {
        assert_eq!(utc(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc(951_782_400, 5), "2000-02-29T00:00:00.005Z");
        assert_eq!(utc(1_792_326_896, 789), "2026-10-18T12:34:56.789Z");
    }

    /// Records name their front end and principal, and the widened keys
    /// are redacted at any depth (#2068).
    #[tokio::test]
    async fn records_name_the_front_end_and_hide_tokens_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let context = Context {
            profile: Some("lab".into()),
            transport: "BIP",
            local: "10.0.0.1:47808".into(),
        };
        let audit = Audit::open(dir.path(), context.clone())
            .unwrap()
            .written_by(FrontEnd::Mcp, Some("agent".into()));
        let fields = json!({ "tool": "bacnet_write", "client": { "name": "host" },
            "params": { "Authorization": "Bearer s3cr3t-a", "api_key": "s3cr3t-b",
                        "refresh_token": "s3cr3t-c", "client_secret": "s3cr3t-d" } });
        audit
            .record(Level::ReadOnly, "write", fields)
            .await
            .unwrap();
        Audit::open(dir.path(), context)
            .unwrap()
            .record(Level::ReadOnly, "arm", json!({}))
            .await
            .unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!text.contains("s3cr3t"), "{text}");
        let records: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records[0]["front_end"], "mcp");
        assert_eq!(records[0]["principal"], "agent");
        assert_eq!(records[0]["tool"], "bacnet_write");
        assert_eq!(records[0]["params"]["api_key"], REDACTED);
        assert_eq!(records[1]["front_end"], "tui");
        assert!(records[1].get("principal").is_none());
    }
}

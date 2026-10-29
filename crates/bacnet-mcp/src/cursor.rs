//! Signed pagination cursors (design §5.1).
//!
//! A cursor is opaque to the client, as MCP requires. Ours carries the
//! position to resume from, the tool, a digest of the principal and of the
//! call's other arguments and an expiry, and is signed with HMAC-SHA256
//! under a key drawn for this process. A forged cursor, or one presented
//! by another principal, for another tool or other arguments, or after it
//! expired, fails closed.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// How long a cursor stays good.
const LIFETIME: Duration = Duration::from_secs(600);

/// Signs and checks the cursors of one server process.
pub(crate) struct Cursors {
    key: [u8; 32],
}

/// What a cursor binds, and where it resumes.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Sealed {
    tool: String,
    principal: String,
    args: String,
    expires: u64,
    position: u64,
}

impl Cursors {
    /// Cursors under a fresh random key.
    pub(crate) fn new() -> Self {
        let mut key = [0; 32];
        // The OS's random source; a process without one can't sign.
        getrandom::fill(&mut key).expect("the OS random source for the cursor key");
        Self { key }
    }

    fn mac(&self) -> Hmac<Sha256> {
        Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC takes a key of any length")
    }

    /// A cursor resuming `tool` at `position`, for this principal and
    /// these arguments (the cursor itself left out).
    pub(crate) fn seal(
        &self,
        tool: &str,
        principal: &str,
        args: &Map<String, Value>,
        position: u64,
    ) -> String {
        let sealed = Sealed {
            tool: tool.into(),
            principal: who(principal),
            args: digest(args),
            expires: now().saturating_add(LIFETIME.as_secs()),
            position,
        };
        let body = serde_json::to_vec(&sealed).expect("a cursor serializes");
        let mut mac = self.mac();
        mac.update(&body);
        let tag = mac.finalize().into_bytes();
        format!("{}.{}", hex(&body), hex(&tag))
    }

    /// The position `token` resumes at, if it is one of ours, unexpired,
    /// and for this tool, principal and arguments.
    pub(crate) fn open(
        &self,
        token: &str,
        tool: &str,
        principal: &str,
        args: &Map<String, Value>,
    ) -> Result<u64, &'static str> {
        let (body, tag) = token.split_once('.').ok_or("malformed")?;
        let (body, tag) = (
            unhex(body).ok_or("malformed")?,
            unhex(tag).ok_or("malformed")?,
        );
        let mut mac = self.mac();
        mac.update(&body);
        mac.verify_slice(&tag)
            .map_err(|_| "not signed by this server")?;
        let sealed: Sealed = serde_json::from_slice(&body).map_err(|_| "malformed")?;
        if sealed.expires < now() {
            return Err("expired");
        }
        if sealed.tool != tool || sealed.principal != who(principal) || sealed.args != digest(args)
        {
            return Err("issued for another call");
        }
        Ok(sealed.position)
    }
}

/// The principal as the cursor carries it: a digest, so a cursor doesn't
/// show the OS user's name to whoever sees it.
fn who(principal: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"principal:");
    hash.update(principal.as_bytes());
    hex(&hash.finalize()[..16])
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// A digest of a call's arguments other than its cursor, the same
/// whatever order the client sent the keys in.
pub(crate) fn digest(args: &Map<String, Value>) -> String {
    let mut canonical = Value::Object(args.clone());
    if let Value::Object(map) = &mut canonical {
        map.remove("cursor");
    }
    let text = sorted(canonical).to_string();
    hex(&Sha256::digest(text.as_bytes()))
}

/// `value` with every object's keys in order.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_cursor_resumes_only_its_own_call() {
        let cursors = Cursors::new();
        let call = args(json!({ "device": "1201", "types": ["ai"], "cursor": "old" }));
        let token = cursors.seal("bacnet_list_objects", "alice", &call, 200);
        assert!(!token.contains(&hex(b"alice")), "the user's name stays out");
        let reordered = args(json!({ "types": ["ai"], "device": "1201" }));
        assert_eq!(
            cursors.open(&token, "bacnet_list_objects", "alice", &reordered),
            Ok(200)
        );
        assert!(cursors.open(&token, "bacnet_read", "alice", &call).is_err());
        assert!(
            cursors
                .open(&token, "bacnet_list_objects", "bob", &call)
                .is_err()
        );
        let other = args(json!({ "device": "1202", "types": ["ai"] }));
        assert!(
            cursors
                .open(&token, "bacnet_list_objects", "alice", &other)
                .is_err()
        );
    }

    #[test]
    fn a_forged_or_foreign_cursor_fails_closed() {
        let cursors = Cursors::new();
        let call = args(json!({ "device": "1" }));
        let token = cursors.seal("bacnet_discover", "alice", &call, 5);
        // Another process's key.
        assert_eq!(
            Cursors::new().open(&token, "bacnet_discover", "alice", &call),
            Err("not signed by this server")
        );
        // A body edited to skip ahead, with the old tag.
        let (body, tag) = token.split_once('.').unwrap();
        let mut sealed: Sealed = serde_json::from_slice(&unhex(body).unwrap()).unwrap();
        sealed.position = 1_000;
        let forged = format!("{}.{tag}", hex(&serde_json::to_vec(&sealed).unwrap()));
        assert!(
            cursors
                .open(&forged, "bacnet_discover", "alice", &call)
                .is_err()
        );
        for junk in ["", "zz.zz", "abc"] {
            assert_eq!(
                cursors.open(junk, "bacnet_discover", "alice", &call),
                Err("malformed")
            );
        }
        assert!(
            cursors
                .open(".", "bacnet_discover", "alice", &call)
                .is_err()
        );
    }

    #[test]
    fn an_expired_cursor_is_refused() {
        let cursors = Cursors::new();
        let call = args(json!({}));
        let sealed = Sealed {
            tool: "t".into(),
            principal: who("p"),
            args: digest(&call),
            expires: now() - 1,
            position: 0,
        };
        let body = serde_json::to_vec(&sealed).unwrap();
        let mut mac = cursors.mac();
        mac.update(&body);
        let token = format!("{}.{}", hex(&body), hex(&mac.finalize().into_bytes()));
        assert_eq!(cursors.open(&token, "t", "p", &call), Err("expired"));
    }
}

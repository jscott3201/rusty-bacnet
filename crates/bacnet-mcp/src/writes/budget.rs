//! A plan grant's write count, kept in the profile workspace beside the
//! ledger (#2159), so restarting the server inside the plan's window
//! doesn't give its budget back.
//!
//! `plan-budget.json` names the authorization it counts for (a digest of
//! the grant's terms and its authorization time, [`key`]) and the writes
//! made under it. `bacnet mcp authorize` writes it with a count of zero
//! ([`crate::policy::PlanGrant::start_budget`]), so authorizing again
//! starts a fresh count. The server reads it and writes the new count,
//! atomically, under the workspace's lock and before a write the plan
//! covers is sent; a count it can't read for this authorization, or can't
//! save, refuses the write. A missing file, an unreadable or corrupt one,
//! and one left by another authorization are all refused alike: none of
//! them can give back budget, and authorizing again is the way out.
//! Authorizing writes the file without the lock (the TUI may hold it for
//! its whole session): a server spending at that moment may put the old
//! authorization's count back, which then matches nothing, so that too
//! ends in a refusal rather than a larger budget.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_ops::safety::hex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::policy::PlanGrant;

/// The count's file in the workspace.
pub(crate) const FILE: &str = "plan-budget.json";

/// The file's format.
const VERSION: u32 = 1;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetFile {
    version: u32,
    /// The authorization counted: [`key`].
    plan: String,
    /// Writes made under it.
    used: u32,
}

/// What names one authorization of a plan: its terms and when it was
/// authorized. Another authorization, or a plan changed since, has
/// another key.
pub(crate) fn key(plan: &PlanGrant) -> String {
    let properties: Vec<u32> = plan.properties.iter().map(|p| p.to_raw()).collect();
    let terms = json!([
        plan.authorized_unix,
        plan.devices,
        properties,
        plan.priorities,
        plan.max_writes,
        plan.expires_unix,
    ]);
    hex(&Sha256::digest(terms.to_string().as_bytes()))
}

/// The writes counted for `plan` in `workspace`, or why there is no count
/// to trust.
pub(crate) fn read(workspace: &Path, plan: &PlanGrant) -> Result<u32, String> {
    let path = workspace.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "the plan grant's write count ({}) is missing",
                path.display()
            ));
        }
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let file: BudgetFile = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a write count: {e}", path.display()))?;
    if file.version != VERSION {
        return Err(format!(
            "{} has format {}, not {VERSION}",
            path.display(),
            file.version
        ));
    }
    if file.plan != key(plan) {
        return Err(format!(
            "{} counts another authorization of the plan",
            path.display()
        ));
    }
    Ok(file.used)
}

/// Save `used` writes for `plan` in `workspace`: a complete, synced
/// temporary file renamed over the old one.
pub(crate) fn write(workspace: &Path, plan: &PlanGrant, used: u32) -> Result<(), String> {
    let path = workspace.join(FILE);
    let text = serde_json::to_string_pretty(&BudgetFile {
        version: VERSION,
        plan: key(plan),
        used,
    })
    .map_err(|e| format!("cannot encode the write count: {e}"))?;
    let temp = workspace.join(format!(
        ".plan-budget-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    bacnet_ops::safety::file::publish(&temp, &path, &text, false)
        .map_err(|e| format!("cannot save {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::PropertyIdentifier;

    use super::*;

    fn plan(authorized_unix: u64) -> PlanGrant {
        PlanGrant {
            devices: vec![(1, 2)],
            properties: vec![PropertyIdentifier::PRESENT_VALUE],
            priorities: (8, 16),
            max_writes: 50,
            authorized_unix,
            expires_unix: authorized_unix + 3_600,
            reason: None,
        }
    }

    /// A count reads back for its own authorization only; a missing,
    /// corrupt, unknown-format or foreign file gives no count at all.
    #[test]
    fn a_count_reads_back_only_for_its_authorization() {
        let dir = tempfile::tempdir().unwrap();
        let first = plan(1_000);
        assert!(read(dir.path(), &first).unwrap_err().contains("missing"));
        write(dir.path(), &first, 30).unwrap();
        assert_eq!(read(dir.path(), &first), Ok(30));
        let again = plan(1_001);
        assert!(read(dir.path(), &again).unwrap_err().contains("another"));
        let wider = PlanGrant {
            max_writes: 51,
            ..first.clone()
        };
        assert!(read(dir.path(), &wider).is_err(), "a changed plan");
        let path = dir.path().join(FILE);
        for text in [
            "{".to_string(),
            "{}".to_string(),
            format!(r#"{{"version":1,"plan":"{}","used":-1}}"#, key(&first)),
            format!(r#"{{"version":2,"plan":"{}","used":0}}"#, key(&first)),
            format!(
                r#"{{"version":1,"plan":"{}","used":0,"extra":1}}"#,
                key(&first)
            ),
        ] {
            std::fs::write(&path, &text).unwrap();
            assert!(read(dir.path(), &first).is_err(), "{text}");
        }
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file left behind");
    }
}

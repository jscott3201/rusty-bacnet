//! Profile I/O at the coordinator boundary, before client construction.

use super::checks::{self, Checks};
use super::{store::Store, Overrides, Profile};
use std::time::{SystemTime, UNIX_EPOCH};

/// Pure UI requests. Connecting includes the exact document previewed by the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    List,
    Load(String),
    Save {
        name: String,
        profile: Profile,
        create: bool,
        previous: Option<Profile>,
    },
    Connect {
        name: String,
        profile: Profile,
        attest: bool,
        /// The check warnings the operator confirmed on the preview.
        accepted: Vec<String>,
    },
}

/// A successful filesystem operation, rendered by the pure UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    Catalog {
        names: Vec<String>,
        root: String,
    },
    /// The stored profile, and the file checks of its resolved settings.
    Loaded {
        name: String,
        profile: Profile,
        checks: Checks,
    },
}

/// What connecting needs: the stored document (as the UI should now show
/// it) and the settings resolved with the command-line overrides.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub(crate) stored: Profile,
    pub(crate) resolved: Profile,
}

/// `p` without what a first connect fills in: the SC identity and the
/// attestation time. Two instances racing that first connect differ only here.
fn first_use_neutral(p: &Profile) -> Profile {
    let mut p = p.clone();
    p.authorized_at_unix = None;
    if let Some(sc) = &mut p.sc {
        (sc.vmac, sc.device_uuid) = (None, None);
    }
    p
}

/// Owns the store and command-line overrides for a profile-enabled session.
pub(crate) struct Service {
    pub(crate) store: Result<Store, String>,
    pub(crate) overrides: Overrides,
}

impl Service {
    pub(crate) fn new(store: Result<Store, String>, overrides: Overrides) -> Self {
        Self { store, overrides }
    }
    fn store(&self) -> Result<&Store, String> {
        self.store.as_ref().map_err(Clone::clone)
    }
    pub(crate) fn catalog(&self) -> Result<Reply, String> {
        let store = self.store()?;
        Ok(Reply::Catalog {
            names: store.list()?,
            root: store.root().display().to_string(),
        })
    }
    /// Read a profile and check its files. Writes nothing.
    pub(crate) fn load(&self, name: &str) -> Result<Reply, String> {
        let profile = self.store()?.load(name)?;
        self.loaded(name, profile)
    }
    fn loaded(&self, name: &str, profile: Profile) -> Result<Reply, String> {
        let checks = checks::inspect(&self.overrides.resolve(&profile)?);
        Ok(Reply::Loaded {
            name: name.into(),
            profile,
            checks,
        })
    }
    /// Save an edited profile. An SC profile gets its VMAC and device UUID
    /// here, once, and relative file paths are made absolute.
    pub(crate) fn save(
        &self,
        name: &str,
        mut profile: Profile,
        create: bool,
        previous: Option<Profile>,
    ) -> Result<Reply, String> {
        let store = self.store()?;
        let stored = if create {
            None
        } else {
            Some(store.load(name)?)
        };
        if !create && previous != stored {
            return Err("profile changed on disk; reload before saving".into());
        }
        if let Some(sc) = &mut profile.sc {
            for path in [&mut sc.ca, &mut sc.cert, &mut sc.key] {
                if !path.as_os_str().is_empty() {
                    *path = std::path::absolute(&*path)
                        .map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
                }
            }
            // An identity, once stored, outlives every edit (Annex AB.1.5.3):
            // only a hand edit of the file can change it.
            if let Some(kept) = stored.as_ref().and_then(|p| p.sc.as_ref()) {
                sc.vmac = kept.vmac.or(sc.vmac);
                sc.device_uuid = kept.device_uuid.or(sc.device_uuid);
            }
            sc.assign_identity()?;
        }
        self.overrides.resolve(&profile)?;
        store.save(name, &profile, create)?;
        self.loaded(name, profile)
    }
    /// The workspace of profile `name`, which connecting it created.
    pub(crate) fn workspace(&self, name: &str) -> Option<std::path::PathBuf> {
        self.store.as_ref().ok()?.workspace(name).ok()
    }
    /// Validate the displayed snapshot and its files, persist the SC identity
    /// and the authorization when missing, and prepare the workspace. Any
    /// failure returns before a client exists, so nothing is sent. A check
    /// warning not in `accepted` (fresh, so a file changed since the preview
    /// counts) refuses too.
    pub(crate) fn prepare(
        &self,
        name: &str,
        displayed: &Profile,
        attest: bool,
        accepted: &[String],
    ) -> Result<Prepared, String> {
        let store = self.store()?;
        let mut profile = store.load(name)?;
        if profile != *displayed {
            return Err("profile changed on disk; reload and review it before connecting".into());
        }
        self.overrides.resolve(&profile)?;
        let mut changed = false;
        if let Some(sc) = &mut profile.sc {
            // A hand-written file may lack them; assign before first use.
            changed |= sc.assign_identity()?;
        }
        let resolved = self.overrides.resolve(&profile)?;
        let checks = checks::inspect(&resolved);
        checks.refusal()?;
        checks.unconfirmed(accepted)?;
        if profile.authorized_at_unix.is_none() {
            if !attest {
                return Err("explicit authorization is required before first use".into());
            }
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| format!("cannot timestamp authorization: {e}"))?
                .as_secs();
            profile.authorized_at_unix = Some(at);
            profile.authorized_safety = Some(profile.safety.clone());
            changed = true;
        }
        if changed {
            store.save(name, &profile, false)?;
            // Another instance may have saved its own first identity at the
            // same moment, and the last rename wins: connect as what is on
            // disk, so every instance uses the one identity that persisted.
            let on_disk = store.load(name)?;
            // Only the identity and the attestation time may differ.
            if on_disk.authorized_at_unix.is_none()
                || first_use_neutral(&on_disk) != first_use_neutral(&profile)
            {
                return Err(
                    "profile changed on disk; reload and review it before connecting".into(),
                );
            }
            profile = on_disk;
        }
        store.workspace(name)?;
        let resolved = self.overrides.resolve(&profile)?;
        Ok(Prepared {
            stored: profile,
            resolved,
        })
    }
}

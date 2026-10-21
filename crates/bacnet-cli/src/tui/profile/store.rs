//! Small atomic file store. No directory is created while browsing profiles.

use super::Profile;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The config root also contains every profile workspace.
#[derive(Debug, Clone)]
pub(crate) struct Store {
    root: PathBuf,
}

/// Check names before using them in any filesystem path, on every platform.
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    let reserved = matches!(
        name.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if name.is_empty()
        || name.len() > 64
        || reserved
        || !name
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
    {
        return Err("profile name must be 1-64 ASCII letters, digits, '-' or '_', excluding reserved device names".into());
    }
    Ok(())
}

impl Store {
    /// Resolve platform paths without creating files. An override isolates all state.
    pub(crate) fn discover() -> Result<Self, String> {
        crate::paths::discover_config_root().map(Self::new)
    }

    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    fn path(&self, name: &str) -> Result<PathBuf, String> {
        validate_name(name)?;
        Ok(self.root.join("profiles").join(format!("{name}.toml")))
    }
    pub(crate) fn list(&self) -> Result<Vec<String>, String> {
        let entries = match fs::read_dir(self.root.join("profiles")) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("cannot list profiles: {e}")),
        };
        let mut names = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|e| format!("cannot read profile entry: {e}"))?
                .path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                if let Some(name) = path.file_stem().and_then(|v| v.to_str()) {
                    if validate_name(name).is_ok() {
                        names.push(name.to_owned());
                    }
                }
            }
        }
        names.sort();
        Ok(names)
    }
    pub(crate) fn load(&self, name: &str) -> Result<Profile, String> {
        let path = self.path(name)?;
        if fs::metadata(&path)
            .map_err(|e| format!("cannot inspect profile {name}: {e}"))?
            .len()
            > 65_536
        {
            return Err("profile is larger than 64 KiB".into());
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut profile: Profile =
            toml::from_str(&text).map_err(|e| format!("invalid profile {name}: {e}"))?;
        profile.validate()?;
        profile.revoke_if_raised();
        Ok(profile)
    }
    /// Save a complete synced temporary file, then atomically publish it.
    /// A new profile uses a hard link so concurrent creation never overwrites.
    pub(crate) fn save(&self, name: &str, profile: &Profile, create: bool) -> Result<(), String> {
        let path = self.path(name)?;
        profile.validate()?;
        let text =
            toml::to_string_pretty(profile).map_err(|e| format!("cannot encode profile: {e}"))?;
        let directory = path.parent().expect("profile path has a parent");
        fs::create_dir_all(directory)
            .map_err(|e| format!("cannot create profiles directory: {e}"))?;
        let temp = directory.join(format!(
            ".profile-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        publish(&temp, &path, &text, create).map_err(|e| format!("cannot save profile {name}: {e}"))
    }
    pub(crate) fn workspace(&self, name: &str) -> Result<PathBuf, String> {
        validate_name(name)?;
        let path = self.root.join("workspaces").join(name);
        fs::create_dir_all(&path).map_err(|e| format!("cannot create profile workspace: {e}"))?;
        Ok(path)
    }
}

/// Cleanup only files this attempt created. A stale temporary name or another
/// writer's collision must survive an unsuccessful create_new. The session
/// ledger is saved the same way.
pub(crate) fn publish(temp: &Path, path: &Path, text: &str, create: bool) -> std::io::Result<()> {
    let mut created = false;
    let result = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(temp)?;
        created = true;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if create {
            fs::hard_link(temp, path)?;
        } else {
            fs::rename(temp, path)?;
        }
        Ok(())
    })();
    if created {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_temporary_creation_preserves_both_prior_profile_and_unowned_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("site.toml");
        let temp = root.path().join(".occupied.tmp");
        fs::write(&path, "previous profile").unwrap();
        fs::write(&temp, "another writer").unwrap();
        assert!(publish(&temp, &path, "replacement", false).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "previous profile");
        assert_eq!(fs::read_to_string(temp).unwrap(), "another writer");
    }

    #[test]
    fn failed_publication_preserves_the_existing_target_and_cleans_its_temp() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("site.toml");
        let temp = root.path().join(".new.tmp");
        fs::write(&path, "previous profile").unwrap();
        assert!(publish(&temp, &path, "replacement", true).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "previous profile");
        assert!(!temp.exists());
    }
}

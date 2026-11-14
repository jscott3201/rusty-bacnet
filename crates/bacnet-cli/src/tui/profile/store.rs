//! Small atomic file store. No directory is created while browsing profiles.

use super::Profile;
use std::fs;
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
            if path.extension().is_some_and(|ext| ext == "toml")
                && let Some(name) = path.file_stem().and_then(|v| v.to_str())
                && validate_name(name).is_ok()
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }
    /// Whether profile `name` has a file, its name checked first.
    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub(crate) fn exists(&self, name: &str) -> Result<bool, String> {
        match fs::metadata(self.path(name)?) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("cannot inspect profile {name}: {e}")),
        }
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
    /// A new profile uses a hard link so concurrent creation never
    /// overwrites; creating it again with the same text, as a retry does,
    /// finds it saved. When only the directory sync fails, the error says
    /// the new file is in place.
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
        match publish(&temp, &path, &text, create) {
            Ok(()) => Ok(()),
            // An earlier save of this very profile went in place, and only
            // its directory sync failed: it is there already, and saved
            // once its directory syncs.
            Err(e)
                if create
                    && e.kind() == std::io::ErrorKind::AlreadyExists
                    && fs::read_to_string(&path).is_ok_and(|saved| saved == text) =>
            {
                sync_parent(&path).map_err(|e| {
                    format!(
                        "profile {name}: the file is in place, but its directory could not be \
                         synced, so it may not survive a power loss: {e}"
                    )
                })
            }
            Err(e) if in_place(&e) => Err(format!("profile {name}: {e}")),
            Err(e) => Err(format!("cannot save profile {name}: {e}")),
        }
    }
    /// Where the profile's workspace is, without creating it.
    pub(crate) fn workspace_path(&self, name: &str) -> Result<PathBuf, String> {
        validate_name(name)?;
        Ok(self.root.join("workspaces").join(name))
    }

    /// The profile's workspace, created if missing. On Unix a new one is
    /// mode 0700, as the TUI's MCP socket needs (#2185); an existing one
    /// keeps its mode.
    pub(crate) fn workspace(&self, name: &str) -> Result<PathBuf, String> {
        let path = self.workspace_path(name)?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&path)
            .map_err(|e| format!("cannot create profile workspace: {e}"))?;
        Ok(path)
    }
}

/// The atomic publish profiles and the session ledger share; it lives
/// beside the ledger in `bacnet_tool::safety::file`.
pub(crate) use bacnet_tool::safety::file::{in_place, publish};

/// [`bacnet_tool::safety::file::sync_parent`], counted for the tests.
fn sync_parent(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    tests::count_sync();
    bacnet_tool::safety::file::sync_parent(path)
}

#[cfg(test)]
pub(super) mod tests {
    use std::cell::Cell;

    use super::*;

    thread_local! {
        /// Directory syncs the store asked for on this thread after
        /// finding a file in place.
        static SYNCS: Cell<usize> = const { Cell::new(0) };
    }

    /// Fail the directory syncs that follow on this thread, the publish's
    /// own included, or stop.
    pub(in crate::tui::profile) fn fail_syncs(fail: bool) {
        bacnet_tool::safety::file::testing::fail_directory_syncs(fail);
    }

    pub(super) fn count_sync() {
        SYNCS.with(|n| n.set(n.get() + 1));
    }

    /// A retried creation that finds its file in place still syncs the
    /// directory, and reports that sync's failure, file in place.
    #[test]
    fn a_retried_creation_syncs_its_directory() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().to_path_buf());
        let profile = Profile::default();
        store.save("site", &profile, true).unwrap();
        let before = SYNCS.with(Cell::get);
        store.save("site", &profile, true).unwrap();
        assert_eq!(SYNCS.with(Cell::get), before + 1, "the retry synced");
        fail_syncs(true);
        let failed = store.save("site", &profile, true);
        fail_syncs(false);
        let error = failed.unwrap_err();
        assert!(error.contains("in place"), "{error}");
        assert_eq!(store.load("site").unwrap(), profile);
    }

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

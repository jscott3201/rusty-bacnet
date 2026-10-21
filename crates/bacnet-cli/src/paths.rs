//! Where the TUI keeps its profiles: the platform config directory from
//! `directories`, never `$HOME` directly (Windows has no `HOME`).
//!
//! Resolution is pure: the environment and the platform directories are
//! passed in, so each OS's layout is testable without touching the real
//! environment. Nothing here writes anything.

use directories::ProjectDirs;
use std::ffi::OsString;
use std::path::PathBuf;

/// Environment variable that replaces the config directory, and with it
/// every profile and profile workspace.
pub(crate) const CONFIG_DIR_ENV: &str = "BACNET_CONFIG_DIR";

/// This program's platform directories, if the OS reports a home directory.
pub(crate) fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "bacnet")
}

/// The config root: the override when set, else the platform config dir.
pub(crate) fn config_root(
    env_override: Option<OsString>,
    dirs: Option<&ProjectDirs>,
) -> Result<PathBuf, String> {
    if let Some(root) = env_override {
        if root.is_empty() {
            return Err(format!("{CONFIG_DIR_ENV} must not be empty"));
        }
        return Ok(PathBuf::from(root));
    }
    dirs.map(|dirs| dirs.config_dir().to_owned())
        .ok_or_else(|| format!("cannot locate the config directory; set {CONFIG_DIR_ENV}"))
}

/// The config root of this process.
pub(crate) fn discover_config_root() -> Result<PathBuf, String> {
    config_root(std::env::var_os(CONFIG_DIR_ENV), project_dirs().as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dirs() -> ProjectDirs {
        project_dirs().expect("CI runners have a home directory")
    }

    #[test]
    fn override_wins_and_must_not_be_empty() {
        let root = config_root(Some("/tmp/site-a".into()), Some(&dirs())).unwrap();
        assert_eq!(root, Path::new("/tmp/site-a"));
        assert_eq!(
            config_root(Some("relative".into()), None).unwrap(),
            Path::new("relative")
        );
        assert!(config_root(Some(OsString::new()), Some(&dirs()))
            .unwrap_err()
            .contains(CONFIG_DIR_ENV));
        assert!(config_root(None, None)
            .unwrap_err()
            .contains(CONFIG_DIR_ENV));
    }

    #[test]
    fn default_config_root_is_the_platform_config_dir() {
        let dirs = dirs();
        assert_eq!(config_root(None, Some(&dirs)).unwrap(), dirs.config_dir());
    }

    /// The platform layout, checked natively on each CI OS.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_uses_the_xdg_config_dir() {
        let dirs = dirs();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| Path::new(v).is_absolute())
            .map_or_else(|| home.join(".config"), PathBuf::from);
        assert_eq!(
            config_root(None, Some(&dirs)).unwrap(),
            config.join("bacnet")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_application_support() {
        let dirs = dirs();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let support = home.join("Library/Application Support/bacnet");
        assert_eq!(config_root(None, Some(&dirs)).unwrap(), support);
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_roaming_app_data_not_home() {
        let dirs = dirs();
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap();
        assert_eq!(
            config_root(None, Some(&dirs)).unwrap(),
            roaming.join("bacnet").join("config")
        );
    }
}

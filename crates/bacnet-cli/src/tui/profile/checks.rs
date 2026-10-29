//! Pre-connect checks that need the filesystem: the files a profile names
//! must be readable before anything is sent, and with `sc-tls` the
//! credentials pass the pre-dial lint (#968). Run at the coordinator boundary
//! (never in the pure UI), both for the preview and again just before a
//! connection attempt, which an error refuses and an unconfirmed warning
//! holds back.

use super::{Profile, Sc};
use std::fs::File;
use std::path::Path;

/// What the connection screen shows under the resolved settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Checks {
    /// Problems that refuse a connection.
    pub(crate) errors: Vec<String>,
    /// Problems worth fixing that allow one once confirmed.
    pub(crate) warnings: Vec<String>,
}

impl Checks {
    /// The first error, as a refusal.
    pub(crate) fn refusal(&self) -> Result<(), String> {
        match self.errors.first() {
            Some(error) => Err(format!("validation failed: {error}")),
            None => Ok(()),
        }
    }

    /// The first warning not in `accepted`, as a refusal: a warning holds a
    /// connection back until the operator confirms exactly what was shown.
    pub(crate) fn unconfirmed(&self, accepted: &[String]) -> Result<(), String> {
        match self.warnings.iter().find(|w| !accepted.contains(w)) {
            Some(warning) => Err(format!(
                "confirm the warnings first (y on the preview): {warning}"
            )),
            None => Ok(()),
        }
    }
}

/// Inspect the files of an already resolved profile, at the clock's time.
pub(crate) fn inspect(profile: &Profile) -> Checks {
    inspect_at(profile, None)
}

/// [`inspect`] with the lint's time injected (Unix seconds).
pub(crate) fn inspect_at(profile: &Profile, at_unix: Option<i64>) -> Checks {
    let mut checks = Checks::default();
    if let Some(sc) = &profile.sc {
        inspect_sc(sc, &mut checks, at_unix);
    }
    checks
}

fn inspect_sc(sc: &Sc, checks: &mut Checks, at_unix: Option<i64>) {
    for (label, path) in [
        ("CA file", &sc.ca),
        ("Certificate file", &sc.cert),
        ("Key file", &sc.key),
    ] {
        if let Err(error) = readable(path) {
            checks
                .errors
                .push(format!("{label} {} {error}", path.display()));
        }
    }
    #[cfg(unix)]
    if let Some(warning) = key_exposure(&sc.key) {
        checks.warnings.push(warning);
    }
    #[cfg(feature = "sc-tls")]
    if checks.errors.is_empty() {
        lint(sc, checks, at_unix);
    }
    #[cfg(not(feature = "sc-tls"))]
    let _ = at_unix;
}

/// The credential lint: its hard failures are errors and its warnings need
/// a confirmation. Key read bits are `key_exposure`'s (Unix only), so not repeated.
#[cfg(feature = "sc-tls")]
fn lint(sc: &Sc, checks: &mut Checks, at_unix: Option<i64>) {
    use crate::sc_check::lint::{Lint, Weight};
    match Lint::run(&sc.cert, Some(&sc.ca), Some(&sc.key), at_unix) {
        Err(error) => checks.errors.push(error),
        Ok(lint) => {
            for finding in &lint.findings {
                match finding.weight {
                    Weight::Hard => checks.errors.push(finding.text.clone()),
                    Weight::Warning if !finding.code.starts_with("key_permissions.") => {
                        checks.warnings.push(finding.text.clone());
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Open the file for reading; a directory or an unreadable file fails.
fn readable(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "does not exist".to_owned(),
        std::io::ErrorKind::PermissionDenied => "is not readable".to_owned(),
        _ => format!("cannot be opened: {e}"),
    })?;
    match file.metadata() {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err("is not a regular file".into()),
        Err(e) => Err(format!("cannot be inspected: {e}")),
    }
}

/// A private key that the group or other users can read.
#[cfg(unix)]
fn key_exposure(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
    (mode & 0o077 != 0).then(|| {
        format!(
            "Key file {} is readable by group or others (mode {mode:o}); chmod 600 it.",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::profile::sc_tests::pem_files;

    fn sc_profile(dir: &Path) -> Profile {
        let (ca, cert, key, _) = pem_files(dir);
        let mut profile = Profile::sc_default();
        let sc = profile.sc.as_mut().unwrap();
        sc.hub_url = "wss://127.0.0.1:1".into();
        (sc.ca, sc.cert, sc.key) = (ca, cert, key);
        profile
    }

    #[test]
    fn readable_files_pass_and_each_missing_file_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let mut profile = sc_profile(dir.path());
        assert_eq!(inspect(&profile), Checks::default());
        assert_eq!(inspect(&Profile::default()), Checks::default());
        let sc = profile.sc.as_mut().unwrap();
        sc.ca = dir.path().join("missing-ca.pem");
        sc.cert = dir.path().to_owned();
        let checks = inspect(&profile);
        assert_eq!(checks.errors.len(), 2, "{checks:?}");
        assert!(checks.errors[0].contains("missing-ca.pem"));
        assert!(checks.errors[0].contains("does not exist"));
        // A directory: refused on open (Windows) or as not a regular file.
        assert!(checks.errors[1].starts_with("Certificate file"));
        assert!(checks.refusal().unwrap_err().contains("missing-ca.pem"));
    }

    #[cfg(unix)]
    #[test]
    fn group_or_world_readable_keys_warn_without_refusing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let profile = sc_profile(dir.path());
        let key = &profile.sc.as_ref().unwrap().key;
        for mode in [0o640, 0o604, 0o644] {
            std::fs::set_permissions(key, std::fs::Permissions::from_mode(mode)).unwrap();
            let checks = inspect(&profile);
            assert!(checks.errors.is_empty());
            // Once: the lint's own read-bit warning is not repeated.
            assert_eq!(checks.warnings.len(), 1, "{mode:o}: {checks:?}");
            assert!(checks.warnings[0].contains("node.key"));
            assert!(checks.refusal().is_ok());
            assert!(checks.unconfirmed(&[]).is_err());
            assert!(checks.unconfirmed(&checks.warnings).is_ok());
        }
        std::fs::set_permissions(key, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert!(inspect(&profile).warnings.is_empty());
    }

    /// The lint, at an injected time (#968).
    #[cfg(feature = "sc-tls")]
    mod lint {
        use super::*;

        /// 2024-01-01T00:00:00Z.
        const NOW: i64 = 1_704_067_200;

        struct Site {
            params: rcgen::CertificateParams,
            key: rcgen::KeyPair,
        }

        fn ca(name: &str) -> Site {
            let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            params.distinguished_name = rcgen::DistinguishedName::new();
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, name);
            Site {
                params,
                key: rcgen::KeyPair::generate().unwrap(),
            }
        }

        /// A node certificate valid from NOW for `days`, signed by `by`.
        fn node(days: u64, by: &Site) -> (String, rcgen::KeyPair) {
            let mut params = rcgen::CertificateParams::new(vec!["node".into()]).unwrap();
            params.not_before = rcgen::date_time_ymd(2024, 1, 1);
            params.not_after = params.not_before + std::time::Duration::from_secs(days * 86_400);
            let key = rcgen::KeyPair::generate().unwrap();
            let issuer = rcgen::Issuer::from_params(&by.params, &by.key);
            (params.signed_by(&key, &issuer).unwrap().pem(), key)
        }

        /// A profile over `cert`, `key` and a bundle of `cas`.
        fn profile(dir: &Path, cert: &str, key: &rcgen::KeyPair, cas: &[&Site]) -> Profile {
            let mut p = sc_profile(dir);
            let sc = p.sc.as_mut().unwrap();
            let bundle: String = cas
                .iter()
                .map(|ca| ca.params.self_signed(&ca.key).unwrap().pem())
                .collect();
            std::fs::write(&sc.ca, bundle).unwrap();
            std::fs::write(&sc.cert, cert).unwrap();
            std::fs::write(&sc.key, key.serialize_pem()).unwrap();
            p
        }

        #[test]
        fn five_twenty_five_and_four_hundred_days_are_alarm_warning_and_ok() {
            let dir = tempfile::tempdir().unwrap();
            let site = ca("site");
            for (days, expected) in [
                (5, Some("seven days")),
                (25, Some("thirty days")),
                (400, None),
            ] {
                let (cert, key) = node(days, &site);
                let checks = inspect_at(&profile(dir.path(), &cert, &key, &[&site]), Some(NOW));
                assert!(checks.errors.is_empty(), "{days}: {checks:?}");
                match expected {
                    Some(words) => {
                        assert_eq!(checks.warnings.len(), 1, "{days}: {checks:?}");
                        assert!(checks.warnings[0].contains(words), "{checks:?}");
                    }
                    None => assert!(checks.warnings.is_empty(), "{days}: {checks:?}"),
                }
            }
        }

        #[test]
        fn expired_and_mismatched_credentials_are_errors() {
            let dir = tempfile::tempdir().unwrap();
            let site = ca("site");
            let (cert, key) = node(400, &site);
            let p = profile(dir.path(), &cert, &key, &[&site]);
            let expired = inspect_at(&p, Some(NOW + 401 * 86_400));
            assert!(
                expired
                    .refusal()
                    .unwrap_err()
                    .contains("Operational certificate")
            );
            let early = inspect_at(&p, Some(NOW - 3_600));
            assert_eq!(early.errors.len(), 1, "{early:?}");
            // A minute early is likely clock skew: a warning to confirm.
            let skewed = inspect_at(&p, Some(NOW - 60));
            assert!(skewed.errors.is_empty(), "{skewed:?}");
            assert!(skewed.warnings[0].contains("valid in 60 s"), "{skewed:?}");
            let other = rcgen::KeyPair::generate().unwrap();
            let mismatched = inspect_at(&profile(dir.path(), &cert, &other, &[&site]), Some(NOW));
            assert_eq!(mismatched.errors.len(), 1, "{mismatched:?}");
            assert!(mismatched.errors[0].starts_with("Key file:"));
        }

        #[test]
        fn a_leaf_from_an_intermediate_outside_the_bundle_is_a_warning() {
            let dir = tempfile::tempdir().unwrap();
            let root = ca("site root");
            let issuing = ca("site issuing");
            let (cert, key) = node(400, &issuing);
            let checks = inspect_at(&profile(dir.path(), &cert, &key, &[&root]), Some(NOW));
            assert!(checks.errors.is_empty(), "{checks:?}");
            assert_eq!(checks.warnings.len(), 1, "{checks:?}");
            assert!(checks.warnings[0].contains("signed the identity leaf directly"));
            let direct = inspect_at(
                &profile(dir.path(), &cert, &key, &[&root, &issuing]),
                Some(NOW),
            );
            assert_eq!(direct, Checks::default());
        }
    }
}

//! The pre-dial lint (#968): `bacnet sc-check`'s inspection and severities,
//! applied to this node's own SC credentials before a hub dial. It must never
//! refuse a dial that TLS would allow, so it weighs a few findings lower than
//! sc-check's audit severities.
//!
//! - Hard failures block the dial, in the TUI and in one-shot commands alike,
//!   before any socket: an identity leaf that is expired, has an invalid
//!   interval, cannot be read, or starts more than [`SKEW_ALLOWANCE`] seconds
//!   after the inspection time; a key that does not match the leaf or cannot be
//!   used; and an input that cannot be read or decoded.
//! - Warnings (expiry within 30 days, a leaf no bundle CA issued directly, a
//!   key file others can read, an unreadable CA or chain entry, a leaf that
//!   starts within the skew allowance, inputs past the inspection limits, and
//!   interpretation limits) can be overridden: the TUI asks for an explicit
//!   confirmation, one-shots log them and go on.
//! - Notices only record what was not assessed.

use super::{Diagnostic, projection};
use bacnet_transport::sc_tls::inspection::{
    ScInputErrorKind, ScInspectionError, ScPemInspection, inspect_sc_pem_files,
};
use std::path::Path;

/// How far in the future (seconds) the leaf's not-before may lie and still
/// only warn: a few minutes is more likely this computer's clock than the
/// certificate.
pub(crate) const SKEW_ALLOWANCE: i64 = 300;

/// How much a finding weighs before a dial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    /// Blocks the dial.
    Hard,
    /// Allowed once confirmed (TUI) or logged (one-shots).
    Warning,
    /// Recorded only.
    Notice,
}

/// One finding, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    /// Its weight.
    pub(crate) weight: Weight,
    /// sc-check's diagnostic code, such as `key.mismatch`.
    pub(crate) code: String,
    /// What it applies to and what to do.
    pub(crate) text: String,
}

/// The inspection and its findings.
#[derive(Debug, Clone)]
pub(crate) struct Lint {
    /// The transport's offline report, for the TUI's credentials view;
    /// `None` when the inputs exceed the inspection limits.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub(crate) report: Option<ScPemInspection>,
    /// sc-check's diagnostics, in its order, weighed for a dial.
    pub(crate) findings: Vec<Finding>,
}

fn subject(diagnostic: &Diagnostic) -> String {
    let index = diagnostic
        .index
        .map_or(String::new(), |i| format!(" [{i}]"));
    match diagnostic.role {
        Some("identity") => format!("Operational certificate{index}"),
        Some("peer_trust") => format!("CA bundle{index}"),
        Some("private_key") => "Key file".into(),
        _ => "Credentials".into(),
    }
}

/// Seconds until the leaf's not-before, when it is still ahead.
fn starts_in(report: &ScPemInspection) -> Option<i64> {
    let leaf = report.certificates.first()?.metadata.as_ref()?;
    let ahead = i128::from(leaf.not_before_unix()) - i128::from(report.inspected_at_unix);
    i64::try_from(ahead).ok().filter(|s| *s > 0)
}

/// A finding with sc-check's severity, reweighed for a dial.
fn weigh(report: &ScPemInspection, diagnostic: Diagnostic) -> Finding {
    let leaf = diagnostic.role == Some("identity") && diagnostic.index == Some(0);
    let mut text = format!("{}: {}", subject(&diagnostic), diagnostic.message);
    let weight = match (diagnostic.severity, diagnostic.code.as_str()) {
        // The hub checks only the leaf; the TLS configuration decides about
        // the rest of the chain and the CA file when it loads them.
        ("error", "certificate.malformed") if !leaf => Weight::Warning,
        ("error", "validity.not_yet_valid") if leaf => match starts_in(report) {
            Some(secs) if secs <= SKEW_ALLOWANCE => {
                text = format!(
                    "{}: valid in {secs} s; check this computer's clock.",
                    subject(&diagnostic)
                );
                Weight::Warning
            }
            _ => Weight::Hard,
        },
        ("error", _) => Weight::Hard,
        ("warning", _) => Weight::Warning,
        _ => Weight::Notice,
    };
    Finding {
        weight,
        text,
        code: diagnostic.code,
    }
}

impl Lint {
    /// Inspect the files at `at_unix`, or at the clock's time when `None`.
    /// An input that can't be read or decoded is an `Err`, which callers
    /// treat as a hard failure. Inputs past the inspection limits (which TLS
    /// may still accept) give a lint with one warning and no report.
    pub(crate) fn run(
        cert: &Path,
        ca: Option<&Path>,
        key: Option<&Path>,
        at_unix: Option<i64>,
    ) -> Result<Self, String> {
        let report = match inspect_sc_pem_files(cert, ca, key, at_unix) {
            Ok(report) => report,
            Err(ScInspectionError::Input {
                kind:
                    kind @ (ScInputErrorKind::InputTooLarge
                    | ScInputErrorKind::TooManyCertificates
                    | ScInputErrorKind::CertificateTooLarge),
                ..
            }) => {
                return Ok(Self {
                    report: None,
                    findings: vec![Finding {
                        weight: Weight::Warning,
                        code: format!("credentials.{}", kind.as_str()),
                        text: format!(
                            "Credentials: not assessed; the files exceed the inspection \
                             limits ({}). TLS still checks them.",
                            kind.as_str()
                        ),
                    }],
                });
            }
            Err(error) => {
                let error = projection::input_error(error);
                let mut text =
                    format!("SC credential check could not complete: {}", error.category);
                if let Some(role) = error.role {
                    text.push_str(&format!(" {role}"));
                }
                if let Some(kind) = error.kind {
                    text.push_str(&format!(" ({kind})"));
                }
                return Err(format!("{text}: {}", error.message));
            }
        };
        let findings = projection::diagnostics(&report)
            .into_iter()
            .map(|diagnostic| weigh(&report, diagnostic))
            .collect();
        Ok(Self {
            report: Some(report),
            findings,
        })
    }

    /// The findings of `weight`.
    pub(crate) fn of(&self, weight: Weight) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |f| f.weight == weight)
    }

    /// Why the dial is refused, when a hard failure blocks it.
    pub(crate) fn refusal(&self) -> Option<String> {
        let hard: Vec<&str> = self.of(Weight::Hard).map(|f| f.text.as_str()).collect();
        (!hard.is_empty()).then(|| {
            format!(
                "SC credential check refused the dial before connecting: {}",
                hard.join(" ")
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z

    /// A self-signed certificate valid from NOW for `days`, and its key.
    fn files(dir: &Path, days: u64) -> (std::path::PathBuf, std::path::PathBuf) {
        let pair = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["node".into()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = params.not_before + std::time::Duration::from_secs(days * 86_400);
        let cert = dir.join("node.pem");
        let key = dir.join("node.key");
        std::fs::write(&cert, params.self_signed(&pair).unwrap().pem()).unwrap();
        std::fs::write(&key, pair.serialize_pem()).unwrap();
        (cert, key)
    }

    fn codes(lint: &Lint, weight: Weight) -> Vec<&str> {
        lint.of(weight).map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn thresholds_weigh_as_alarm_warning_and_ok_and_dates_block() {
        let dir = tempfile::tempdir().unwrap();
        for (days, expected) in [
            (5, Some("expiry.alarm")),
            (25, Some("expiry.warning")),
            (400, None),
        ] {
            let (cert, key) = files(dir.path(), days);
            let lint = Lint::run(&cert, Some(&cert), Some(&key), Some(NOW)).unwrap();
            let warnings = codes(&lint, Weight::Warning);
            let expiry: Vec<_> = warnings
                .iter()
                .filter(|c| c.starts_with("expiry."))
                .collect();
            assert_eq!(expiry.first().map(|c| **c), expected, "{days} days");
            assert!(lint.refusal().is_none(), "{days} days: {:?}", lint.findings);
        }
        let (cert, key) = files(dir.path(), 400);
        for (at, code) in [
            (NOW - SKEW_ALLOWANCE - 1, "validity.not_yet_valid"),
            (NOW + 401 * 86_400, "validity.expired"),
        ] {
            let lint = Lint::run(&cert, Some(&cert), Some(&key), Some(at)).unwrap();
            assert_eq!(codes(&lint, Weight::Hard), [code]);
            assert!(
                lint.refusal()
                    .unwrap()
                    .contains("Operational certificate [0]")
            );
        }
    }

    #[test]
    fn a_mismatched_key_is_hard_and_unreadable_input_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, _) = files(dir.path(), 400);
        let other = dir.path().join("other.key");
        std::fs::write(&other, rcgen::KeyPair::generate().unwrap().serialize_pem()).unwrap();
        let lint = Lint::run(&cert, Some(&cert), Some(&other), Some(NOW)).unwrap();
        assert_eq!(codes(&lint, Weight::Hard), ["key.mismatch"]);
        assert!(lint.refusal().unwrap().contains("Key file:"));
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        let error = Lint::run(&empty, None, None, Some(NOW)).unwrap_err();
        assert!(error.contains("empty_certificates"), "{error}");
    }

    #[test]
    fn a_leaf_starting_within_the_skew_allowance_only_warns() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = files(dir.path(), 400);
        for ahead in [1, 120, SKEW_ALLOWANCE] {
            let lint = Lint::run(&cert, Some(&cert), Some(&key), Some(NOW - ahead)).unwrap();
            assert!(lint.refusal().is_none(), "{ahead}: {:?}", lint.findings);
            let skew = lint
                .of(Weight::Warning)
                .find(|f| f.code == "validity.not_yet_valid")
                .expect("a skew warning");
            assert!(
                skew.text.contains(&format!("valid in {ahead} s")),
                "{skew:?}"
            );
        }
    }

    /// A self-signed CA certificate, as PEM.
    fn ca_pem(name: &str) -> String {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let key = rcgen::KeyPair::generate().unwrap();
        params.self_signed(&key).unwrap().pem()
    }

    #[test]
    fn an_unreadable_ca_entry_beside_a_good_one_only_warns() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = files(dir.path(), 400);
        let bundle = dir.path().join("bundle.pem");
        let malformed = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n";
        let good = std::fs::read_to_string(&cert).unwrap();
        std::fs::write(&bundle, format!("{good}{malformed}")).unwrap();
        let lint = Lint::run(&cert, Some(&bundle), Some(&key), Some(NOW)).unwrap();
        assert!(lint.refusal().is_none(), "{:?}", lint.findings);
        assert!(codes(&lint, Weight::Warning).contains(&"certificate.malformed"));
        // The leaf itself unreadable stays hard.
        let lint = Lint::run(&bundle, Some(&cert), None, Some(NOW)).unwrap();
        assert!(lint.refusal().is_none(), "the chain entry only warns");
        let leaf_bad = dir.path().join("leaf-bad.pem");
        std::fs::write(&leaf_bad, format!("{malformed}{good}")).unwrap();
        let lint = Lint::run(&leaf_bad, Some(&cert), None, Some(NOW)).unwrap();
        assert_eq!(codes(&lint, Weight::Hard), ["certificate.malformed"]);
    }

    #[test]
    fn inputs_past_the_inspection_limits_warn_instead_of_refusing() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = files(dir.path(), 400);
        let bundle = dir.path().join("big-bundle.pem");
        let cas: String = (0..64).map(|i| ca_pem(&format!("CA {i}"))).collect();
        std::fs::write(&bundle, cas).unwrap();
        let lint = Lint::run(&cert, Some(&bundle), Some(&key), Some(NOW)).unwrap();
        assert!(lint.report.is_none());
        assert!(lint.refusal().is_none());
        assert_eq!(
            codes(&lint, Weight::Warning),
            ["credentials.too_many_certificates"]
        );
    }
}

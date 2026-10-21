//! This node's own SC credentials (#968), for the SC screen's third view:
//! the pre-dial lint's inventory as plain text, built by the worker before
//! the dial so the view needs no I/O.

/// How a line reads; every level has a word as well as a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    /// Nothing to do.
    Ok,
    /// Not assessed, or for information.
    Notice,
    /// Worth fixing; a dial needs it confirmed (TUI) or logs it (one-shots).
    Warning,
    /// Seven days or fewer to expiry.
    Alarm,
    /// Blocks the dial.
    Failed,
}

impl Level {
    /// The word shown beside the line.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Ok => "[ ok ]",
            Self::Notice => "[note]",
            Self::Warning => "[warn]",
            Self::Alarm => "[ALRM]",
            Self::Failed => "[FAIL]",
        }
    }
}

/// One certificate of the operational chain or the CA bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cert {
    /// `certificate 0` (the operational leaf), `chain 1`, or `CA 0`.
    pub(crate) label: String,
    /// Subject distinguished name, escaped.
    pub(crate) subject: String,
    /// Issuer distinguished name, escaped.
    pub(crate) issuer: String,
    /// Subject alternative names, or why there are none.
    pub(crate) san: String,
    /// The public key's algorithm.
    pub(crate) key_algorithm: String,
    /// Extended key usage, client and server authentication among them.
    pub(crate) eku: String,
    /// Not-after, as a UTC date.
    pub(crate) not_after: String,
    /// The validity window against the inspection time, in days.
    pub(crate) window: String,
    /// The window's level.
    pub(crate) level: Level,
}

/// The inventory and the lint's verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Credentials {
    /// When it was inspected, as a UTC date.
    pub(crate) inspected_at: String,
    /// The operational chain, then the CA bundle.
    pub(crate) certificates: Vec<Cert>,
    /// Whether the key matches the operational certificate.
    pub(crate) key: (Level, String),
    /// Whether a bundle CA issued the operational certificate directly.
    pub(crate) issuer: (Level, String),
    /// Hard failures and warnings, in sc-check's order.
    pub(crate) findings: Vec<(Level, String)>,
}

/// Whole days in `seconds`, rounded toward zero.
fn days(seconds: i128) -> i128 {
    seconds / 86_400
}

/// A public key algorithm's usual name, or its OID.
fn key_algorithm(oid: &str) -> String {
    match oid {
        "1.2.840.113549.1.1.1" => "RSA".into(),
        "1.2.840.113549.1.1.10" => "RSASSA-PSS".into(),
        "1.2.840.10045.2.1" => "EC".into(),
        "1.3.101.112" => "Ed25519".into(),
        "1.3.101.113" => "Ed448".into(),
        other => format!("OID {other}"),
    }
}

/// A certificate signature algorithm's usual name, or its dotted OID.
pub(crate) fn signature_algorithm(oid: &str) -> &str {
    match oid {
        "1.2.840.10045.4.3.2" => "ecdsa-with-SHA256",
        "1.2.840.10045.4.3.3" => "ecdsa-with-SHA384",
        "1.2.840.10045.4.3.4" => "ecdsa-with-SHA512",
        "1.2.840.113549.1.1.5" => "sha1WithRSAEncryption",
        "1.2.840.113549.1.1.11" => "sha256WithRSAEncryption",
        "1.2.840.113549.1.1.12" => "sha384WithRSAEncryption",
        "1.2.840.113549.1.1.13" => "sha512WithRSAEncryption",
        "1.2.840.113549.1.1.10" => "RSASSA-PSS",
        "1.3.101.112" => "Ed25519",
        "1.3.101.113" => "Ed448",
        other => other,
    }
}

#[cfg(feature = "sc-tls")]
mod build {
    use super::*;
    use crate::sc_check::lint::{Lint, Weight, SKEW_ALLOWANCE};
    use bacnet_transport::sc_tls::inspection::{
        ScDirectIssuer, ScExpiry, ScExtensionInspection, ScInputRole, ScInspectedCertificate,
        ScKeyConsistency, ScValidity,
    };

    fn values(extension: &ScExtensionInspection, absent: &str) -> String {
        match (extension.assessed, extension.present) {
            (false, _) => "not assessed".into(),
            (true, false) => absent.into(),
            (true, true) if extension.values.is_empty() => "present, unreadable".into(),
            (true, true) => extension.values.join(", "),
        }
    }

    fn cert(entry: &ScInspectedCertificate, now: i64) -> Cert {
        let label = match (entry.role, entry.index) {
            (ScInputRole::Identity, 0) => "certificate 0".into(),
            (ScInputRole::Identity, i) => format!("chain {i}"),
            (_, i) => format!("CA {i}"),
        };
        let leaf = entry.role == ScInputRole::Identity && entry.index == 0;
        let Some(meta) = &entry.metadata else {
            return Cert {
                label,
                subject: "(certificate could not be read)".into(),
                issuer: String::new(),
                san: String::new(),
                key_algorithm: String::new(),
                eku: String::new(),
                not_after: String::new(),
                window: "unreadable".into(),
                // Only an unreadable leaf blocks a dial.
                level: if leaf { Level::Failed } else { Level::Warning },
            };
        };
        let left = i128::from(meta.not_after_unix()) - i128::from(now);
        let ahead = i128::from(meta.not_before_unix()) - i128::from(now);
        let (window, level) = match entry.validity {
            ScValidity::Expired => (format!("expired {} days ago", days(-left)), Level::Failed),
            // Within the skew allowance: more likely this computer's clock.
            ScValidity::NotYetValid if ahead <= i128::from(SKEW_ALLOWANCE) => (
                format!("valid in {ahead} s; check this computer's clock"),
                Level::Warning,
            ),
            ScValidity::NotYetValid => (
                format!("not valid for {} more days", days(ahead)),
                Level::Failed,
            ),
            ScValidity::InvalidInterval => ("ends before it starts".into(), Level::Failed),
            ScValidity::Unavailable => ("not assessed".into(), Level::Notice),
            ScValidity::WithinInterval => (
                format!("{} days left", days(left)),
                match entry.expiry {
                    ScExpiry::Alarm => Level::Alarm,
                    ScExpiry::Warning => Level::Warning,
                    _ => Level::Ok,
                },
            ),
        };
        // Only the leaf's dates block a dial; elsewhere they warn.
        let level = match level {
            Level::Failed if !leaf => Level::Warning,
            other => other,
        };
        Cert {
            label,
            subject: meta.subject().into(),
            issuer: meta.issuer().into(),
            san: values(&entry.san, "none"),
            key_algorithm: key_algorithm(meta.public_key_algorithm_oid()),
            // Kebab-case, as the TUI writes enumerations (`client-auth`).
            eku: values(&entry.extended_key_usage, "none (any purpose)").replace('_', "-"),
            not_after: crate::sc_check::date(meta.not_after_unix()),
            window,
            level,
        }
    }

    impl Credentials {
        /// The view of a lint.
        pub(crate) fn from_lint(lint: &Lint) -> Self {
            let findings = lint
                .findings
                .iter()
                .filter_map(|f| match f.weight {
                    Weight::Hard => Some((Level::Failed, f.text.clone())),
                    Weight::Warning => Some((Level::Warning, f.text.clone())),
                    Weight::Notice => None,
                })
                .collect();
            let Some(report) = &lint.report else {
                // Past the inspection limits: only the warning saying so.
                return Self {
                    inspected_at: "not inspected".into(),
                    certificates: Vec::new(),
                    key: (Level::Notice, "not checked".into()),
                    issuer: (Level::Notice, "not checked".into()),
                    findings,
                };
            };
            let now = report.inspected_at_unix;
            let key = match report.key_consistency {
                ScKeyConsistency::Match => (Level::Ok, "matches the operational certificate"),
                ScKeyConsistency::Mismatch => {
                    (Level::Failed, "does NOT match the operational certificate")
                }
                ScKeyConsistency::Unusable => (Level::Failed, "could not be used"),
                ScKeyConsistency::Unknown => (Level::Warning, "match could not be established"),
                ScKeyConsistency::NotAssessed => (Level::Notice, "not checked"),
            };
            let issuer = match (report.direct_issuer, report.direct_issuer_index) {
                (ScDirectIssuer::Direct, Some(i)) => {
                    (Level::Ok, format!("issued directly by bundle CA {i}"))
                }
                (ScDirectIssuer::NotDirect, _) => (
                    Level::Warning,
                    "no bundle CA issued it directly (Annex AB.7.4 default)".into(),
                ),
                (ScDirectIssuer::NotAssessed, _) => (Level::Notice, "not checked".into()),
                _ => (
                    Level::Warning,
                    "direct issuance could not be checked".into(),
                ),
            };
            Self {
                inspected_at: crate::sc_check::date(now),
                certificates: report.certificates.iter().map(|c| cert(c, now)).collect(),
                key: (key.0, key.1.into()),
                issuer,
                findings,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_round_toward_zero_and_algorithms_have_names() {
        assert_eq!(days(5 * 86_400 + 86_399), 5);
        assert_eq!(days(86_399), 0);
        assert_eq!(key_algorithm("1.2.840.10045.2.1"), "EC");
        assert_eq!(key_algorithm("1.2.3"), "OID 1.2.3");
        assert_eq!(
            signature_algorithm("1.2.840.10045.4.3.2"),
            "ecdsa-with-SHA256"
        );
        assert_eq!(
            signature_algorithm("1.2.840.113549.1.1.11"),
            "sha256WithRSAEncryption"
        );
        assert_eq!(signature_algorithm("1.2.3.4"), "1.2.3.4");
    }

    #[cfg(feature = "sc-tls")]
    #[test]
    fn a_lint_reads_as_levels_with_days_to_expiry() {
        use crate::sc_check::lint::Lint;
        const NOW: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z
        let dir = tempfile::tempdir().unwrap();
        let pair = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["node.site".into()]).unwrap();
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = params.not_before + std::time::Duration::from_secs(25 * 86_400);
        let cert = dir.path().join("node.pem");
        let key = dir.path().join("node.key");
        std::fs::write(&cert, params.self_signed(&pair).unwrap().pem()).unwrap();
        std::fs::write(&key, pair.serialize_pem()).unwrap();
        let lint = Lint::run(&cert, Some(&cert), Some(&key), Some(NOW)).unwrap();
        let view = Credentials::from_lint(&lint);
        let leaf = &view.certificates[0];
        assert_eq!(leaf.label, "certificate 0");
        assert_eq!(
            (leaf.window.as_str(), leaf.level),
            ("25 days left", Level::Warning)
        );
        assert_eq!(leaf.san, "DNS:node.site");
        assert_eq!(leaf.eku, "client-auth");
        assert_eq!(view.certificates[1].label, "CA 0");
        assert_eq!(view.key.0, Level::Ok);
        assert_eq!(
            view.issuer,
            (Level::Ok, "issued directly by bundle CA 0".into())
        );
        assert!(view
            .findings
            .iter()
            .any(|(level, text)| *level == Level::Warning && text.contains("thirty days")));
        // Past the end: the leaf fails.
        let late = Lint::run(&cert, Some(&cert), Some(&key), Some(NOW + 27 * 86_400)).unwrap();
        let late = Credentials::from_lint(&late);
        assert_eq!(
            (
                late.certificates[0].window.as_str(),
                late.certificates[0].level
            ),
            ("expired 2 days ago", Level::Failed)
        );
        // The same dates on a bundle CA only warn.
        assert_eq!(late.certificates[1].level, Level::Warning);
    }
}

//! CLI-owned serialization and observation severity; no TLS policy is applied.

use super::{Diagnostic, Document, ExecutionError, Summary, UNASSESSED};
use bacnet_transport::sc_tls::{inspection::*, ScCertificateMetadata};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct Inspection<'a> {
    pub(super) inspected_at_unix: i64,
    pub(super) peer_trust_supplied: bool,
    pub(super) key_consistency: &'static str,
    pub(super) key_file_permissions: Option<Permissions>,
    pub(super) direct_issuer: &'static str,
    pub(super) direct_issuer_index: Option<usize>,
    pub(super) certificates: Vec<Certificate<'a>>,
}
#[derive(Serialize)]
pub(super) struct Permissions {
    pub(super) group_readable: bool,
    pub(super) world_readable: bool,
}
#[derive(Serialize)]
pub(super) struct Certificate<'a> {
    pub(super) role: &'static str,
    pub(super) index: usize,
    pub(super) sha256: String,
    pub(super) metadata: Option<Metadata<'a>>,
    pub(super) validity: &'static str,
    pub(super) expiry: &'static str,
    pub(super) san: Extension<'a>,
    pub(super) key_usage: Extension<'a>,
    pub(super) extended_key_usage: Extension<'a>,
    pub(super) findings: Vec<Finding>,
}
#[derive(Serialize)]
pub(super) struct Metadata<'a> {
    pub(super) subject: &'a str,
    pub(super) issuer: &'a str,
    pub(super) not_before_unix: i64,
    pub(super) not_after_unix: i64,
    pub(super) public_key_algorithm_oid: &'a str,
    pub(super) signature_algorithm_oid: &'a str,
    pub(super) text_truncated: bool,
}
impl<'a> From<&'a ScCertificateMetadata> for Metadata<'a> {
    fn from(value: &'a ScCertificateMetadata) -> Self {
        Self {
            subject: value.subject(),
            issuer: value.issuer(),
            not_before_unix: value.not_before_unix(),
            not_after_unix: value.not_after_unix(),
            public_key_algorithm_oid: value.public_key_algorithm_oid(),
            signature_algorithm_oid: value.signature_algorithm_oid(),
            text_truncated: value.text_truncated(),
        }
    }
}
#[derive(Serialize)]
pub(super) struct Extension<'a> {
    pub(super) assessed: bool,
    pub(super) present: bool,
    pub(super) values: &'a [String],
}
impl<'a> From<&'a ScExtensionInspection> for Extension<'a> {
    fn from(value: &'a ScExtensionInspection) -> Self {
        Self {
            assessed: value.assessed,
            present: value.present,
            values: &value.values,
        }
    }
}
#[derive(Serialize)]
pub(super) struct Finding {
    pub(super) area: &'static str,
    pub(super) kind: &'static str,
}

pub(super) fn completed(report: &ScPemInspection) -> Document<Inspection<'_>> {
    let diagnostics = diagnostics(report);
    let mut summary = Summary::default();
    for diagnostic in &diagnostics {
        match diagnostic.severity {
            "error" => summary.errors += 1,
            "warning" => summary.warnings += 1,
            _ => summary.notices += 1,
        }
    }
    Document {
        outcome: "completed",
        summary: Some(summary),
        diagnostics,
        error: None,
        unassessed: UNASSESSED,
        inspection: Some(Inspection {
            inspected_at_unix: report.inspected_at_unix,
            peer_trust_supplied: report.peer_trust_supplied,
            key_consistency: report.key_consistency.as_str(),
            key_file_permissions: report.key_file_permissions.map(|value| Permissions {
                group_readable: value.group_readable,
                world_readable: value.world_readable,
            }),
            direct_issuer: report.direct_issuer.as_str(),
            direct_issuer_index: report.direct_issuer_index,
            certificates: report
                .certificates
                .iter()
                .map(|value| Certificate {
                    role: value.role.as_str(),
                    index: value.index,
                    sha256: value
                        .sha256
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect(),
                    metadata: value.metadata.as_ref().map(Metadata::from),
                    validity: value.validity.as_str(),
                    expiry: value.expiry.as_str(),
                    san: (&value.san).into(),
                    key_usage: (&value.key_usage).into(),
                    extended_key_usage: (&value.extended_key_usage).into(),
                    findings: value
                        .findings
                        .iter()
                        .map(|finding| Finding {
                            area: finding.area.as_str(),
                            kind: finding.kind.as_str(),
                        })
                        .collect(),
                })
                .collect(),
        }),
    }
}

fn diagnostic(
    role: ScInputRole,
    index: Option<usize>,
    area: &'static str,
    kind: &str,
    severity: &'static str,
    message: &str,
) -> Diagnostic {
    Diagnostic {
        code: format!("{area}.{kind}"),
        severity,
        role: Some(role.as_str()),
        index,
        message: message.into(),
        area,
    }
}

pub(super) fn diagnostics(report: &ScPemInspection) -> Vec<Diagnostic> {
    let mut result = Vec::new();
    for cert in &report.certificates {
        let mut add = |area, kind: &str, severity, message: &str| {
            result.push(diagnostic(
                cert.role,
                Some(cert.index),
                area,
                kind,
                severity,
                message,
            ))
        };
        for finding in &cert.findings {
            let severity = if finding.area == ScFindingArea::Certificate
                && finding.kind == ScFindingKind::Malformed
            {
                "error"
            } else {
                "warning"
            };
            let message = match (finding.area, finding.kind) {
                (ScFindingArea::Certificate, ScFindingKind::Malformed) =>
                    "Certificate data could not be interpreted. Replace this item with a valid certificate.",
                (ScFindingArea::Certificate, ScFindingKind::Truncated) =>
                    "Certificate extension count exceeded the inspection limit. Review unassessed extensions independently.",
                (_, ScFindingKind::Malformed) =>
                    "Extension data could not be interpreted completely. Review its encoding independently.",
                (_, ScFindingKind::Duplicate) =>
                    "Duplicate extension data was found. Review the certificate instead of selecting one value.",
                (_, ScFindingKind::Unsupported) =>
                    "Some certificate data is unsupported by this display. Review it with a suitable inspection tool.",
                (_, ScFindingKind::Truncated) =>
                    "Inspection or display limits truncated this area. Review the remaining data independently.",
            };
            add(
                finding.area.as_str(),
                finding.kind.as_str(),
                severity,
                message,
            );
        }
        let date_severity = if cert.role == ScInputRole::Identity && cert.index == 0 {
            "error"
        } else {
            "warning"
        };
        match cert.validity {
            ScValidity::WithinInterval => {}
            ScValidity::Unavailable => add(
                "validity",
                "unavailable",
                "notice",
                "The validity interval could not be assessed. Inspect the certificate encoding.",
            ),
            validity => add(
                "validity",
                validity.as_str(),
                date_severity,
                if date_severity == "error" {
                    "The identity leaf has expired, is not valid yet, or has invalid dates. Check the inspection time and replace it as needed."
                } else {
                    "This inventory certificate has a date problem. Review its intended role; no certificate path or trust anchor was selected."
                },
            ),
        }
        match cert.expiry {
            ScExpiry::Alarm => add("expiry", "alarm", "warning",
                "It expires within seven days. Arrange renewal; this notice does not itself make the certificate invalid."),
            ScExpiry::Warning => add("expiry", "warning", "warning",
                "It expires within thirty days. Plan renewal."),
            _ => {},
        }
        if cert
            .metadata
            .as_ref()
            .is_some_and(ScCertificateMetadata::text_truncated)
        {
            add("metadata", "text_truncated", "warning",
                "Metadata display text reached its bound. Inspect the original certificate with a suitable tool.");
        }
        for (area, extension) in [
            ("san", &cert.san),
            ("key_usage", &cert.key_usage),
            ("extended_key_usage", &cert.extended_key_usage),
        ] {
            if !extension.assessed {
                add(area, "not_assessed", "notice",
                    "Presence and values were not fully assessed. Do not interpret this state as absence.");
            }
        }
    }
    let mut add_key = |area, kind, severity, message| {
        result.push(diagnostic(
            ScInputRole::PrivateKey,
            None,
            area,
            kind,
            severity,
            message,
        ))
    };
    match report.key_consistency {
        ScKeyConsistency::Match => {},
        ScKeyConsistency::NotAssessed => add_key("key", "not_assessed", "notice",
            "No private key was supplied. Supply --sc-key to compare it with the identity leaf."),
        ScKeyConsistency::Mismatch => add_key("key", "mismatch", "error",
            "The private key does not match the identity leaf. Select the corresponding certificate and key."),
        ScKeyConsistency::Unusable => add_key("key", "unusable", "error",
            "The supplied private key could not be used for comparison. Check its supported unencrypted encoding."),
        ScKeyConsistency::Unknown => add_key("key", "unknown", "warning",
            "A key match could not be established. Review the identity leaf and key before relying on them."),
    }
    match report.key_file_permissions {
        None => add_key("key_permissions", "not_assessed", "notice",
            "Unix key read bits were not observed. The key was omitted or this platform does not provide that observation."),
        Some(bits) => {
            if bits.group_readable { add_key("key_permissions", "group_readable", "warning",
                "The opened key file has its Unix group-read bit set. Review local access requirements; ACLs were not assessed."); }
            if bits.world_readable { add_key("key_permissions", "world_readable", "warning",
                "The opened key file has its Unix world-read bit set. Review local access requirements; ACLs were not assessed."); }
        }
    }
    let mut add_issuer = |kind, message| {
        result.push(diagnostic(
            ScInputRole::Identity,
            Some(0),
            "issuer",
            kind,
            "warning",
            message,
        ))
    };
    match report.direct_issuer {
        ScDirectIssuer::Direct | ScDirectIssuer::NotAssessed => {}
        ScDirectIssuer::NotDirect => add_issuer("not_direct",
            "No CA in the --sc-ca bundle signed the identity leaf directly. Peers using the Annex AB.7.4 default check with this bundle can refuse it; have a bundle CA issue it, or add its issuing CA to every peer's bundle."),
        ScDirectIssuer::Unknown => add_issuer("unknown",
            "Direct issuance by a --sc-ca CA could not be established: the leaf could not be read, or a candidate CA uses a signature algorithm this check does not support."),
    }
    if !report.peer_trust_supplied {
        result.push(diagnostic(ScInputRole::PeerTrust, None, "peer_trust", "not_assessed", "notice",
            "No peer-trust file was supplied. Supply --sc-ca to inventory that file and to check that one of its CAs issued the identity leaf directly."));
    }
    let role = |value: Option<&str>| match value {
        Some("identity") => 0,
        Some("peer_trust") => 1,
        Some("private_key") => 2,
        _ => 3,
    };
    result.sort_by(|a, b| {
        role(a.role)
            .cmp(&role(b.role))
            .then(a.index.cmp(&b.index))
            .then(a.area.cmp(b.area))
            .then(a.code.cmp(&b.code))
    });
    result
}

pub(super) fn input_error(error: ScInspectionError) -> ExecutionError {
    match error {
        ScInspectionError::Input { role, index, kind } => ExecutionError {
            category: "input", role: Some(role.as_str()), index, kind: Some(kind.as_str()),
            os_code: None,
            message: match kind {
                ScInputErrorKind::InputTooLarge => "Selected files exceed the one-MiB aggregate input limit. Reduce their contents.",
                ScInputErrorKind::CertificateTooLarge => "A decoded certificate exceeds 128 KiB. Select a bounded certificate.",
                ScInputErrorKind::TooManyCertificates => "The inputs exceed 64 certificates. Reduce the inventory.",
                ScInputErrorKind::EmptyCertificates => "No certificate was found in the supplied file. Select a certificate PEM file.",
                ScInputErrorKind::MalformedPem => "PEM input could not be decoded. Check the selected file's PEM framing and encoding.",
                ScInputErrorKind::MissingPrivateKey => "No supported unencrypted private key was found. Check the selected key file.",
                ScInputErrorKind::ClockOutOfRange => "The clock cannot be represented in signed Unix seconds. Supply --at-unix explicitly.",
            },
        },
        ScInspectionError::Io { role, source } => ExecutionError {
            category: "io", role: Some(role.as_str()), index: None, kind: None,
            os_code: source.raw_os_error(),
            message: "Could not read a selected input file. Check that it exists and is readable; the role and OS code identify the failure.",
        },
    }
}

//! Complete human inventory using the same projection and diagnostics as JSON.

use super::HUB_NAME_NOTE;
use super::{
    Document,
    projection::{Extension, Inspection},
};
use std::io::{self, Write};

pub(crate) fn date(seconds: i64) -> String {
    let formatted = time::OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|value| {
            value
                .format(&time::format_description::well_known::Rfc3339)
                .ok()
        });
    match formatted {
        Some(value) => format!("{value} (Unix {seconds})"),
        None => format!("Unix {seconds} (UTC calendar display unavailable)"),
    }
}

pub(super) fn distance(end: i64, now: i64) -> String {
    let delta = i128::from(end) - i128::from(now);
    let seconds = delta.abs();
    let relation = if delta < 0 {
        "elapsed since expiry"
    } else {
        "remaining until expiry"
    };
    format!(
        "{} days {:02}:{:02}:{:02} {relation}",
        seconds / 86400,
        seconds % 86400 / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

fn extension(name: &str, value: &Extension<'_>, output: &mut impl Write) -> io::Result<()> {
    let state = match (value.assessed, value.present) {
        (false, true) => "present; interpretation not assessed",
        (false, false) => "not assessed; presence unknown",
        (true, false) => "absent",
        (true, true) => "present",
    };
    writeln!(output, "  {name}: {state}")?;
    if value.present && value.values.is_empty() {
        writeln!(output, "    no interpreted values; consult findings")?;
    }
    for value in value.values {
        writeln!(output, "    {value}")?;
    }
    Ok(())
}

pub(super) fn human(
    document: &Document<Inspection<'_>>,
    output: &mut impl Write,
) -> io::Result<()> {
    let inspection = document.inspection.as_ref().expect("completed inspection");
    let summary = document.summary.as_ref().expect("completed summary");
    writeln!(
        output,
        "SC offline inspection at {}",
        date(inspection.inspected_at_unix)
    )?;
    writeln!(
        output,
        "{} errors, {} warnings, {} notices",
        summary.errors, summary.warnings, summary.notices
    )?;
    writeln!(output, "\nRole / index | Validity | Expiry")?;
    for cert in &inspection.certificates {
        writeln!(
            output,
            "{}[{}] | {} | {}",
            cert.role, cert.index, cert.validity, cert.expiry
        )?;
    }
    for cert in &inspection.certificates {
        writeln!(output, "\n{}[{}]", cert.role, cert.index)?;
        writeln!(output, "  SHA-256: {}", cert.sha256)?;
        match &cert.metadata {
            Some(meta) => {
                writeln!(output, "  Subject: {}", meta.subject)?;
                writeln!(output, "  Issuer: {}", meta.issuer)?;
                writeln!(
                    output,
                    "  Public key algorithm OID: {}",
                    meta.public_key_algorithm_oid
                )?;
                writeln!(
                    output,
                    "  Signature algorithm OID: {}",
                    meta.signature_algorithm_oid
                )?;
                writeln!(output, "  Not before: {}", date(meta.not_before_unix))?;
                writeln!(output, "  Not after: {}", date(meta.not_after_unix))?;
                writeln!(
                    output,
                    "  {}",
                    distance(meta.not_after_unix, inspection.inspected_at_unix)
                )?;
                if meta.text_truncated {
                    writeln!(output, "  Metadata text: truncated")?;
                }
            }
            None => writeln!(
                output,
                "  Metadata: unavailable; certificate could not be interpreted"
            )?,
        }
        extension("SAN", &cert.san, output)?;
        extension("KU", &cert.key_usage, output)?;
        extension(
            "EKU (including client_auth/server_auth)",
            &cert.extended_key_usage,
            output,
        )?;
        for finding in &cert.findings {
            writeln!(output, "  Finding: {}/{}", finding.area, finding.kind)?;
        }
    }
    writeln!(
        output,
        "\nPeer-trust file supplied: {}",
        inspection.peer_trust_supplied
    )?;
    writeln!(output, "Key consistency: {}", inspection.key_consistency)?;
    match inspection.direct_issuer_index {
        Some(index) => writeln!(
            output,
            "Direct issuer: {} (peer_trust[{index}])",
            inspection.direct_issuer
        )?,
        None => writeln!(output, "Direct issuer: {}", inspection.direct_issuer)?,
    }
    match &inspection.key_file_permissions {
        Some(bits) => writeln!(
            output,
            "Key Unix read bits: group_readable={}, world_readable={} (ACLs not assessed)",
            bits.group_readable, bits.world_readable
        )?,
        None => writeln!(output, "Key Unix read bits: not assessed")?,
    }
    writeln!(output, "\nDiagnostics:")?;
    for diagnostic in &document.diagnostics {
        write!(
            output,
            "  {} {}",
            diagnostic.severity,
            diagnostic.role.unwrap_or("inspection")
        )?;
        if let Some(index) = diagnostic.index {
            write!(output, "[{index}]")?;
        }
        writeln!(output, " {}: {}", diagnostic.code, diagnostic.message)?;
    }
    writeln!(
        output,
        "\nNot assessed: issuer authorization by remote peers, certificate path validation, expected peer name, revocation, and remote acceptance."
    )?;
    writeln!(output, "{HUB_NAME_NOTE}")
}

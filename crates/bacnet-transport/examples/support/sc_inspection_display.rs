//! Package-local example helper, exercised by the public-boundary integration test.

use bacnet_transport::sc_tls::inspection::{inspect_sc_pem_files, ScPemInspection};
use std::{io::Write, path::Path};

pub fn inspect_and_write(
    out: &mut impl Write,
    identity: &Path,
    peer_trust: Option<&Path>,
    key: Option<&Path>,
    inspected_at_unix: Option<i64>,
) -> Result<ScPemInspection, Box<dyn std::error::Error>> {
    let report = inspect_sc_pem_files(identity, peer_trust, key, inspected_at_unix)?;
    writeln!(out, "inspected at: {}", report.inspected_at_unix)?;
    writeln!(out, "key consistency: {}", report.key_consistency.as_str())?;
    for cert in &report.certificates {
        let fingerprint = cert
            .sha256
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        writeln!(
            out,
            "{} {} {} {} {}",
            cert.role.as_str(),
            cert.index,
            fingerprint,
            cert.validity.as_str(),
            cert.expiry.as_str()
        )?;
        if let Some(metadata) = &cert.metadata {
            writeln!(out, "{} {}", metadata.subject(), metadata.issuer())?;
        }
        for finding in &cert.findings {
            writeln!(
                out,
                "finding: {} {}",
                finding.area.as_str(),
                finding.kind.as_str()
            )?;
        }
    }
    if let Some(mode) = report.key_file_permissions {
        writeln!(
            out,
            "key group/world read mode bits: {} {}",
            mode.group_readable, mode.world_readable
        )?;
    }
    Ok(report)
}

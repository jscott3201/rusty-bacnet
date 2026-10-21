//! Synchronous, advisory inspection of explicitly supplied local PEM materials.
//!
//! No chain construction, hostname checking, revocation, network I/O, credential
//! reload or TLS-policy mutation occurs. The one signature check asks whether a
//! peer-trust certificate signed the identity leaf directly (see
//! [`ScDirectIssuer`]); it does not build a path. Dates, usage displays and that
//! check cannot establish remote acceptance or SC admission.
//!
//! Offline-only bounds: 1 MiB total input, 64 certificates, 128 KiB per DER,
//! 64 extensions and 16 KiB interpreted extension content per certificate.
//! Each of SAN/KU/EKU retains at most 16 values of 256 ASCII-escaped bytes;
//! metadata fields retain at most 1024 bytes each. Truncation is explicit.
//! Existing connection adapters use [`super::pem`] without these new limits.

use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer},
    sign::CertifiedKey,
};
use std::{
    fs::File,
    io::Read,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use x509_parser::{nom::Parser, prelude::*};

use super::{peer_info, pem};
#[path = "inspection_types.rs"]
mod types;
pub use types::*;
#[path = "inspection_extensions.rs"]
mod extensions;

/// Total bytes across all supplied PEM inputs, checked before parsing/allocation.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
/// Total certificates across identity and peer-trust inputs.
pub const MAX_CERTIFICATES: usize = 64;
/// Maximum decoded size of one certificate.
pub const MAX_CERTIFICATE_BYTES: usize = 128 * 1024;

/// Inspect PEM bytes. Identity is required; supplied empty optional inputs are
/// errors. `None` time samples the clock once. The first identity certificate is
/// the key-comparison leaf. Private-key decoding preserves first-key semantics.
pub fn inspect_sc_pem(
    identity: &[u8],
    peer_trust: Option<&[u8]>,
    private_key: Option<&[u8]>,
    inspected_at_unix: Option<i64>,
) -> Result<ScPemInspection, ScInspectionError> {
    let mut total = 0usize;
    for (role, bytes) in [
        (ScInputRole::Identity, Some(identity)),
        (ScInputRole::PeerTrust, peer_trust),
        (ScInputRole::PrivateKey, private_key),
    ] {
        if let Some(bytes) = bytes {
            total = total.saturating_add(bytes.len());
            if total > MAX_INPUT_BYTES {
                return Err(input(role, None, ScInputErrorKind::InputTooLarge));
            }
        }
    }
    let now = match inspected_at_unix {
        Some(now) => now,
        None => clock()?,
    };
    let mut entries = Vec::new();
    let identity = decode_certificates(identity, ScInputRole::Identity, &mut entries, now)?;
    let (direct_issuer, direct_issuer_index) = match peer_trust {
        Some(bytes) => {
            let trust = decode_certificates(bytes, ScInputRole::PeerTrust, &mut entries, now)?;
            direct_issuer(&identity[0], &trust)
        }
        None => (ScDirectIssuer::NotAssessed, None),
    };
    let key_consistency = match private_key {
        None => ScKeyConsistency::NotAssessed,
        Some(bytes) => {
            let key = pem::private_key(bytes).map_err(|error| {
                let kind = if matches!(error, rustls::pki_types::pem::Error::NoItemsFound) {
                    ScInputErrorKind::MissingPrivateKey
                } else {
                    ScInputErrorKind::MalformedPem
                };
                input(ScInputRole::PrivateKey, None, kind)
            })?;
            compare_key(identity, key)
        }
    };
    Ok(ScPemInspection {
        inspected_at_unix: now,
        peer_trust_supplied: peer_trust.is_some(),
        certificates: entries,
        key_consistency,
        key_file_permissions: None,
        direct_issuer,
        direct_issuer_index,
    })
}

/// Open and bounded-read each selected file once, in identity/trust/key order.
/// Key mode bits come from the opened descriptor. The report describes these
/// bytes only; a later connection startup may read different file contents.
pub fn inspect_sc_pem_files(
    identity: &Path,
    peer_trust: Option<&Path>,
    private_key: Option<&Path>,
    inspected_at_unix: Option<i64>,
) -> Result<ScPemInspection, ScInspectionError> {
    let now = match inspected_at_unix {
        Some(now) => now,
        None => clock()?,
    };
    let mut remaining = MAX_INPUT_BYTES;
    let (identity, _) = read(identity, ScInputRole::Identity, &mut remaining)?;
    let trust = peer_trust
        .map(|path| read(path, ScInputRole::PeerTrust, &mut remaining))
        .transpose()?
        .map(|(bytes, _)| bytes);
    let key = private_key
        .map(|path| read(path, ScInputRole::PrivateKey, &mut remaining))
        .transpose()?;
    let mut report = inspect_sc_pem(
        &identity,
        trust.as_deref(),
        key.as_ref().map(|(b, _)| b.as_slice()),
        Some(now),
    )?;
    report.key_file_permissions = key.and_then(|(_, permissions)| permissions);
    Ok(report)
}

fn input(role: ScInputRole, index: Option<usize>, kind: ScInputErrorKind) -> ScInspectionError {
    ScInspectionError::Input { role, index, kind }
}
fn clock() -> Result<i64, ScInspectionError> {
    let value = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i128::from(d.as_secs()),
        Err(e) => {
            -i128::from(e.duration().as_secs()) - i128::from(e.duration().subsec_nanos() != 0)
        }
    };
    i64::try_from(value).map_err(|_| {
        input(
            ScInputRole::Identity,
            None,
            ScInputErrorKind::ClockOutOfRange,
        )
    })
}
fn read(
    path: &Path,
    role: ScInputRole,
    remaining: &mut usize,
) -> Result<(Vec<u8>, Option<ScKeyFilePermissions>), ScInspectionError> {
    let io = |source| ScInspectionError::Io { role, source };
    let file = File::open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    if metadata.len() > *remaining as u64 {
        return Err(input(role, None, ScInputErrorKind::InputTooLarge));
    }
    let mut bytes = Vec::new();
    file.take(*remaining as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > *remaining {
        return Err(input(role, None, ScInputErrorKind::InputTooLarge));
    }
    *remaining -= bytes.len();
    let permissions = permissions(&metadata, role);
    Ok((bytes, permissions))
}
#[cfg(unix)]
fn permissions(metadata: &std::fs::Metadata, role: ScInputRole) -> Option<ScKeyFilePermissions> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    (role == ScInputRole::PrivateKey).then_some(ScKeyFilePermissions {
        group_readable: mode & 0o040 != 0,
        world_readable: mode & 0o004 != 0,
    })
}
#[cfg(not(unix))]
fn permissions(_: &std::fs::Metadata, _: ScInputRole) -> Option<ScKeyFilePermissions> {
    None
}

fn decode_certificates(
    bytes: &[u8],
    role: ScInputRole,
    entries: &mut Vec<ScInspectedCertificate>,
    now: i64,
) -> Result<Vec<CertificateDer<'static>>, ScInspectionError> {
    let mut certs = Vec::new();
    for (index, cert) in pem::certificate_iter(bytes).enumerate() {
        if entries.len() >= MAX_CERTIFICATES {
            return Err(input(
                role,
                Some(index),
                ScInputErrorKind::TooManyCertificates,
            ));
        }
        let cert = cert.map_err(|_| input(role, Some(index), ScInputErrorKind::MalformedPem))?;
        if cert.len() > MAX_CERTIFICATE_BYTES {
            return Err(input(
                role,
                Some(index),
                ScInputErrorKind::CertificateTooLarge,
            ));
        }
        entries.push(inspect_certificate(&cert, role, index, now));
        certs.push(cert);
    }
    if certs.is_empty() {
        return Err(input(role, None, ScInputErrorKind::EmptyCertificates));
    }
    Ok(certs)
}
fn inspect_certificate(
    der: &[u8],
    role: ScInputRole,
    index: usize,
    now: i64,
) -> ScInspectedCertificate {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, der);
    let mut entry = ScInspectedCertificate {
        role,
        index,
        sha256: digest.as_ref().try_into().expect("SHA256 size"),
        metadata: None,
        validity: ScValidity::Unavailable,
        expiry: ScExpiry::NotAssessed,
        san: ScExtensionInspection::default(),
        key_usage: ScExtensionInspection::default(),
        extended_key_usage: ScExtensionInspection::default(),
        findings: Vec::new(),
    };
    match X509CertificateParser::new()
        .with_deep_parse_extensions(false)
        .parse(der)
    {
        Ok(([], cert)) => {
            let metadata = peer_info::metadata(&cert);
            (entry.validity, entry.expiry) =
                assess_time(metadata.not_before_unix(), metadata.not_after_unix(), now);
            extensions::inspect(&cert, &mut entry);
            entry.metadata = Some(metadata);
        }
        _ => entry.findings.push(ScInspectionFinding {
            area: ScFindingArea::Certificate,
            kind: ScFindingKind::Malformed,
        }),
    }
    entry
}
fn assess_time(before: i64, after: i64, now: i64) -> (ScValidity, ScExpiry) {
    let validity = if after < before {
        ScValidity::InvalidInterval
    } else if now < before {
        ScValidity::NotYetValid
    } else if now > after {
        ScValidity::Expired
    } else {
        ScValidity::WithinInterval
    };
    let expiry = if validity != ScValidity::WithinInterval {
        ScExpiry::NotAssessed
    } else {
        match i128::from(after) - i128::from(now) {
            0..=604_800 => ScExpiry::Alarm,
            604_801..=2_592_000 => ScExpiry::Warning,
            _ => ScExpiry::None,
        }
    };
    (validity, expiry)
}
/// Did one of `trust` sign `leaf`, with nothing in between? A candidate is a
/// trust certificate whose subject is byte-for-byte the leaf's issuer name; the
/// leaf's signature must then verify with that certificate's public key. Annex
/// AB.7.4 makes this the default peer check, so a leaf that only chains through
/// an intermediate can be refused by a peer that holds the same CA bundle.
fn direct_issuer(
    leaf: &[u8],
    trust: &[CertificateDer<'static>],
) -> (ScDirectIssuer, Option<usize>) {
    let parse = |der| {
        X509CertificateParser::new()
            .with_deep_parse_extensions(false)
            .parse(der)
            .ok()
            .and_then(|(rest, cert)| rest.is_empty().then_some(cert))
    };
    let Some(leaf) = parse(leaf) else {
        return (ScDirectIssuer::Unknown, None);
    };
    let mut undecided = false;
    for (index, der) in trust.iter().enumerate() {
        // A malformed trust entry is already a finding on its own entry.
        let Some(ca) = parse(der) else { continue };
        if ca.subject().as_raw() != leaf.issuer().as_raw() {
            continue;
        }
        match leaf.verify_signature(Some(ca.public_key())) {
            Ok(()) => return (ScDirectIssuer::Direct, Some(index)),
            Err(X509Error::SignatureUnsupportedAlgorithm) => undecided = true,
            // Same name, other key: a re-keyed CA or a namesake.
            Err(_) => {}
        }
    }
    let verdict = if undecided {
        ScDirectIssuer::Unknown
    } else {
        ScDirectIssuer::NotDirect
    };
    (verdict, None)
}
fn compare_key(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> ScKeyConsistency {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let Ok(key) = provider.key_provider.load_private_key(key) else {
        return ScKeyConsistency::Unusable;
    };
    classify_match(CertifiedKey::new(certs, key).keys_match())
}
fn classify_match(result: Result<(), rustls::Error>) -> ScKeyConsistency {
    match result {
        Ok(()) => ScKeyConsistency::Match,
        Err(rustls::Error::InconsistentKeys(rustls::InconsistentKeys::KeyMismatch)) => {
            ScKeyConsistency::Mismatch
        }
        // Unknown public-key representation and unusable leaf DER cannot prove a match.
        Err(_) => ScKeyConsistency::Unknown,
    }
}

#[cfg(test)]
#[path = "inspection_unit_tests.rs"]
mod tests;

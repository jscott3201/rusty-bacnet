//! Local-only extension interpretation. Peer snapshots keep their shallow parser.

use std::{
    fmt,
    net::{Ipv4Addr, Ipv6Addr},
};
use x509_parser::{
    extensions::{ExtendedKeyUsage, GeneralName, KeyUsage, SubjectAlternativeName},
    prelude::*,
};

use super::super::peer_info::escaped_bounded;
use super::{
    ScExtensionInspection, ScFindingArea as Area, ScFindingKind as Kind, ScInspectedCertificate,
    ScInspectionFinding,
};

const MAX_EXTENSIONS: usize = 64;
const MAX_EXTENSION_BYTES: usize = 16 * 1024;
const MAX_VALUES: usize = 16;

pub(super) fn inspect(cert: &X509Certificate<'_>, entry: &mut ScInspectedCertificate) {
    if cert.extensions().len() > MAX_EXTENSIONS {
        finding(&mut entry.findings, Area::Certificate, Kind::Truncated);
        return;
    }
    let mut remaining = MAX_EXTENSION_BYTES;
    for (oid, area, output) in [
        (&[0x55, 0x1d, 0x11][..], Area::San, &mut entry.san),
        (
            &[0x55, 0x1d, 0x0f][..],
            Area::KeyUsage,
            &mut entry.key_usage,
        ),
        (
            &[0x55, 0x1d, 0x25][..],
            Area::ExtendedKeyUsage,
            &mut entry.extended_key_usage,
        ),
    ] {
        output.assessed = true;
        let mut matching = cert
            .extensions()
            .iter()
            .filter(|extension| extension.oid.as_bytes() == oid);
        let Some(extension) = matching.next() else {
            continue;
        };
        output.present = true;
        if matching.next().is_some() {
            finding(&mut entry.findings, area, Kind::Duplicate);
            continue;
        }
        if extension.value.len() > remaining {
            output.assessed = false;
            finding(&mut entry.findings, area, Kind::Truncated);
            continue;
        }
        remaining -= extension.value.len();
        match area {
            Area::San => san(extension.value, output, &mut entry.findings),
            Area::KeyUsage => ku(extension.value, output, &mut entry.findings),
            Area::ExtendedKeyUsage => eku(extension.value, output, &mut entry.findings),
            Area::Certificate => unreachable!(),
        }
    }
}
fn finding(findings: &mut Vec<ScInspectionFinding>, area: Area, kind: Kind) {
    let item = ScInspectionFinding { area, kind };
    if !findings.contains(&item) {
        findings.push(item);
    }
}
fn value(
    output: &mut ScExtensionInspection,
    findings: &mut Vec<ScInspectionFinding>,
    area: Area,
    text: &impl fmt::Display,
) {
    if output.values.len() == MAX_VALUES {
        finding(findings, area, Kind::Truncated);
        return;
    }
    let mut truncated = false;
    output
        .values
        .push(escaped_bounded(text, &mut truncated, 256));
    if truncated {
        finding(findings, area, Kind::Truncated);
    }
}
fn san(bytes: &[u8], output: &mut ScExtensionInspection, findings: &mut Vec<ScInspectionFinding>) {
    let Ok(([], names)) = SubjectAlternativeName::from_der(bytes) else {
        finding(findings, Area::San, Kind::Malformed);
        return;
    };
    if names.general_names.is_empty() {
        finding(findings, Area::San, Kind::Malformed);
    }
    for name in names.general_names {
        match name {
            GeneralName::DNSName(name) => {
                if !valid_dns(name) {
                    finding(findings, Area::San, Kind::Malformed);
                }
                value(output, findings, Area::San, &format_args!("DNS:{name}"));
            }
            GeneralName::IPAddress(ip) => match ip.len() {
                4 => value(
                    output,
                    findings,
                    Area::San,
                    &format_args!("IP:{}", Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])),
                ),
                16 => value(
                    output,
                    findings,
                    Area::San,
                    &format_args!(
                        "IP:{}",
                        Ipv6Addr::from(<[u8; 16]>::try_from(ip).expect("checked length"))
                    ),
                ),
                _ => finding(findings, Area::San, Kind::Malformed),
            },
            GeneralName::RFC822Name(s) | GeneralName::URI(s) => {
                if !s.is_ascii() || s.bytes().any(|b| b.is_ascii_control()) {
                    finding(findings, Area::San, Kind::Malformed);
                }
                value(output, findings, Area::San, &name);
            }
            GeneralName::DirectoryName(_) | GeneralName::RegisteredID(_) => {
                value(output, findings, Area::San, &name)
            }
            GeneralName::Invalid(..) => finding(findings, Area::San, Kind::Malformed),
            _ => finding(findings, Area::San, Kind::Unsupported),
        }
    }
}
// Informational DNS presentation syntax (including a whole first-label wildcard),
// not a hostname match against any intended peer.
fn valid_dns(name: &str) -> bool {
    let name = name.strip_prefix("*.").unwrap_or(name);
    let name = name.strip_suffix('.').unwrap_or(name);
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
fn ku(bytes: &[u8], output: &mut ScExtensionInspection, findings: &mut Vec<ScInspectionFinding>) {
    let Ok(([], usage)) = KeyUsage::from_der(bytes) else {
        finding(findings, Area::KeyUsage, Kind::Malformed);
        return;
    };
    // The library's u16 bit fold can discard higher bits. Inspect the original
    // BIT STRING as well, so extra usage bits cannot silently look supported.
    let Ok(([], bits)) = x509_parser::der_parser::der::parse_der_bitstring(bytes) else {
        finding(findings, Area::KeyUsage, Kind::Malformed);
        return;
    };
    if let Ok(bits) = bits.as_bitstring() {
        if bits.data.len() > 2 || usage.flags & !0x01ff != 0 {
            finding(findings, Area::KeyUsage, Kind::Unsupported);
        }
    }
    if usage.flags == 0 {
        finding(findings, Area::KeyUsage, Kind::Malformed);
    }
    for (bit, name) in [
        "digital_signature",
        "content_commitment",
        "key_encipherment",
        "data_encipherment",
        "key_agreement",
        "key_cert_sign",
        "crl_sign",
        "encipher_only",
        "decipher_only",
    ]
    .iter()
    .enumerate()
    {
        if usage.flags & (1 << bit) != 0 {
            value(output, findings, Area::KeyUsage, name);
        }
    }
}
fn eku(bytes: &[u8], output: &mut ScExtensionInspection, findings: &mut Vec<ScInspectionFinding>) {
    let Ok(([], usage)) = ExtendedKeyUsage::from_der(bytes) else {
        finding(findings, Area::ExtendedKeyUsage, Kind::Malformed);
        return;
    };
    for (set, name) in [
        (usage.any, "any"),
        (usage.server_auth, "server_auth"),
        (usage.client_auth, "client_auth"),
        (usage.code_signing, "code_signing"),
        (usage.email_protection, "email_protection"),
        (usage.time_stamping, "time_stamping"),
        (usage.ocsp_signing, "ocsp_signing"),
    ] {
        if set {
            value(output, findings, Area::ExtendedKeyUsage, &name);
        }
    }
    for oid in usage.other {
        finding(findings, Area::ExtendedKeyUsage, Kind::Unsupported);
        value(output, findings, Area::ExtendedKeyUsage, &oid);
    }
    if output.values.is_empty() {
        finding(findings, Area::ExtendedKeyUsage, Kind::Malformed);
    }
}

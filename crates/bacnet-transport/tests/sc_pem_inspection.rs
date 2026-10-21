//! Independent fixture encodings exercise the public offline boundary.
#![cfg(feature = "sc-tls")]

use bacnet_transport::sc_tls::{inspection::*, pem};
use rustls::pki_types::PrivateKeyDer;
#[path = "support/sc_pem_materials.rs"]
mod fixture;
use fixture::*;
#[path = "../examples/support/sc_inspection_display.rs"]
mod example;

const NOW: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z

fn inspect(bytes: &[u8]) -> ScInspectedCertificate {
    inspect_sc_pem(bytes, None, None, Some(NOW))
        .unwrap()
        .certificates
        .remove(0)
}
fn has(entry: &ScInspectedCertificate, area: ScFindingArea, kind: ScFindingKind) -> bool {
    entry.findings.contains(&ScInspectionFinding { area, kind })
}
fn extension(oid: &[u64], bytes: Vec<u8>) -> rcgen::CustomExtension {
    rcgen::CustomExtension::from_oid_content(oid, bytes)
}

#[test]
fn decoder_retains_order_empty_unknown_and_first_key_contracts() {
    let mut chain = pem("UNKNOWN", b"ignored");
    chain.extend(pem("CERTIFICATE", b"first"));
    chain.extend(pem("CERTIFICATE", b"second"));
    let certs = pem::certificates(&chain).unwrap();
    assert_eq!(
        certs.iter().map(|c| c.as_ref()).collect::<Vec<_>>(),
        [b"first".as_slice(), b"second"]
    );
    assert!(pem::certificates(b"").unwrap().is_empty());
    for (label, expected) in [
        ("RSA PRIVATE KEY", 1),
        ("EC PRIVATE KEY", 2),
        ("PRIVATE KEY", 3),
    ] {
        let mut bytes = pem(label, b"unusable DER still decodes");
        bytes.extend(b"-----BEGIN SECRET malformed\n");
        let key = pem::private_key(&bytes).unwrap();
        assert!(matches!(
            (expected, key),
            (1, PrivateKeyDer::Pkcs1(_))
                | (2, PrivateKeyDer::Sec1(_))
                | (3, PrivateKeyDer::Pkcs8(_))
        ));
    }
    assert!(pem::private_key(&pem("ENCRYPTED PRIVATE KEY", b"encrypted")).is_err());
    let mut invalid_tail = pem("CERTIFICATE", b"first");
    invalid_tail.extend(b"-----BEGIN CERTIFICATE-----\n***\n-----END CERTIFICATE-----\n");
    let mut iter = pem::certificate_iter(&invalid_tail);
    assert_eq!(iter.next().unwrap().unwrap().as_ref(), b"first");
    assert!(iter.next().unwrap().is_err());
    assert!(pem::certificates(&invalid_tail).is_err());
    let read = pem::certificate_reader(std::io::Cursor::new(&chain))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(read, certs);
}

#[test]
fn inventory_roles_dates_extensions_and_owned_outputs() {
    let mut p = params();
    p.subject_alt_names = vec![
        rcgen::SanType::DnsName("localhost".try_into().unwrap()),
        rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap()),
    ];
    p.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![
        rcgen::ExtendedKeyUsagePurpose::ServerAuth,
        rcgen::ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let (mut bytes, key) = certificate(p);
    let trust = certificate(params()).0;
    let report = inspect_sc_pem(&bytes, Some(&trust), Some(&key), Some(NOW)).unwrap();
    bytes.fill(0);
    drop(key);
    assert_eq!(report.inspected_at_unix, NOW);
    assert!(report.peer_trust_supplied);
    assert_eq!(report.key_consistency, ScKeyConsistency::Match);
    assert_eq!(report.key_file_permissions, None);
    assert_eq!(report.certificates.len(), 2);
    assert_eq!(
        (report.certificates[1].role, report.certificates[1].index),
        (ScInputRole::PeerTrust, 0)
    );
    let entry = &report.certificates[0];
    assert_eq!(
        entry.metadata.as_ref().unwrap().subject(),
        "CN=offline leaf"
    );
    assert_eq!(entry.metadata.as_ref().unwrap().not_before_unix(), NOW);
    assert_eq!(entry.validity, ScValidity::WithinInterval);
    assert_eq!(entry.expiry, ScExpiry::None);
    assert_eq!(entry.san.values, ["DNS:localhost", "IP:127.0.0.1"]);
    assert_eq!(entry.key_usage.values, ["digital_signature"]);
    assert_eq!(
        entry.extended_key_usage.values,
        ["server_auth", "client_auth"]
    );
    assert!(entry.findings.is_empty());
    assert!(!format!("{report:?}").contains("offline leaf"));
}

#[test]
fn malformed_der_keeps_exact_fingerprint_but_no_fabricated_metadata() {
    let entry = inspect(&pem("CERTIFICATE", b"abc"));
    // Published SHA-256 test vector, independent of the implementation's hash call.
    assert_eq!(
        entry.sha256,
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad
        ]
    );
    assert!(entry.metadata.is_none());
    assert_eq!(entry.validity, ScValidity::Unavailable);
    assert!(!entry.san.assessed);
    assert!(has(
        &entry,
        ScFindingArea::Certificate,
        ScFindingKind::Malformed
    ));
    let (valid, _) = certificate(params());
    let mut der = pem::certificates(&valid).unwrap().remove(0).to_vec();
    der.push(0);
    assert!(inspect(&pem("CERTIFICATE", &der)).metadata.is_none());
}

#[test]
fn exact_seconds_control_thresholds_and_both_interval_endpoints() {
    for (seconds, expected) in [
        (5 * 86400, ScExpiry::Alarm),
        (7 * 86400, ScExpiry::Alarm),
        (7 * 86400 + 1, ScExpiry::Warning),
        (25 * 86400, ScExpiry::Warning),
        (30 * 86400, ScExpiry::Warning),
        (30 * 86400 + 1, ScExpiry::None),
        (400 * 86400, ScExpiry::None),
    ] {
        let mut p = params();
        p.not_after = p.not_before + std::time::Duration::from_secs(seconds);
        let (bytes, _) = certificate(p);
        let entry = inspect(&bytes);
        assert_eq!(
            (entry.validity, entry.expiry),
            (ScValidity::WithinInterval, expected)
        );
        for (now, expected) in [
            (NOW - 1, ScValidity::NotYetValid),
            (NOW, ScValidity::WithinInterval),
            (NOW + seconds as i64, ScValidity::WithinInterval),
            (NOW + seconds as i64 + 1, ScValidity::Expired),
        ] {
            let report = inspect_sc_pem(&bytes, None, None, Some(now)).unwrap();
            assert_eq!(report.certificates[0].validity, expected);
        }
    }
    let mut p = params();
    p.not_after = p.not_before - std::time::Duration::from_secs(1);
    let entry = inspect(&certificate(p).0);
    assert_eq!(
        (entry.validity, entry.expiry),
        (ScValidity::InvalidInterval, ScExpiry::NotAssessed)
    );
}

#[test]
fn key_results_distinguish_mismatch_unusable_omission_and_unavailable_leaf() {
    let (bytes, key) = certificate(params());
    let other_key = rcgen::KeyPair::generate().unwrap().serialize_pem();
    for (key, expected) in [
        (None, ScKeyConsistency::NotAssessed),
        (Some(key.as_slice()), ScKeyConsistency::Match),
        (Some(other_key.as_bytes()), ScKeyConsistency::Mismatch),
    ] {
        assert_eq!(
            inspect_sc_pem(&bytes, None, key, Some(NOW))
                .unwrap()
                .key_consistency,
            expected
        );
    }
    assert_eq!(
        inspect_sc_pem(&bytes, None, Some(&pem("PRIVATE KEY", b"abc")), Some(NOW))
            .unwrap()
            .key_consistency,
        ScKeyConsistency::Unusable
    );
    assert_eq!(
        inspect_sc_pem(&pem("CERTIFICATE", b"abc"), None, Some(&key), Some(NOW))
            .unwrap()
            .key_consistency,
        ScKeyConsistency::Unknown
    );
}

#[test]
fn malformed_and_duplicate_extensions_are_never_absent_or_silently_valid() {
    use ScFindingArea::{ExtendedKeyUsage, KeyUsage, San};
    for (oid, area, valid_value) in [
        (vec![2, 5, 29, 17], San, vec![0x30, 0x03, 0x82, 0x01, b'a']),
        (vec![2, 5, 29, 15], KeyUsage, vec![0x03, 0x02, 0x07, 0x80]),
        (
            vec![2, 5, 29, 37],
            ExtendedKeyUsage,
            vec![
                0x30, 0x0a, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01,
            ],
        ),
    ] {
        for value in [vec![0xff], [valid_value.as_slice(), &[0]].concat()] {
            let mut p = params();
            p.custom_extensions.push(extension(&oid, value));
            let entry = inspect(&certificate(p).0);
            assert!(has(&entry, area, ScFindingKind::Malformed), "{area:?}");
            let output = match area {
                San => entry.san,
                KeyUsage => entry.key_usage,
                _ => entry.extended_key_usage,
            };
            assert!(output.present);
        }
        let mut p = params();
        p.custom_extensions = vec![
            extension(&oid, valid_value.clone()),
            extension(&oid, valid_value),
        ];
        assert!(has(
            &inspect(&certificate(p).0),
            area,
            ScFindingKind::Duplicate
        ));
    }
    let absent = inspect(&certificate(params()).0);
    assert!(absent.san.assessed && !absent.san.present && absent.san.values.is_empty());
    assert!(absent.findings.is_empty());
}

#[test]
fn malformed_san_members_unsupported_purposes_and_hostile_text_are_bounded() {
    for member in [
        tlv(0x82, &[0xff]),
        tlv(0x87, &[1, 2, 3]),
        tlv(0x82, b"bad..dns"),
        tlv(0x82, b"name\n\x1b[2J"),
    ] {
        let mut p = params();
        p.custom_extensions
            .push(extension(&[2, 5, 29, 17], tlv(0x30, &member)));
        let entry = inspect(&certificate(p).0);
        assert!(has(&entry, ScFindingArea::San, ScFindingKind::Malformed));
        assert!(entry
            .san
            .values
            .iter()
            .all(|s| s.is_ascii() && !s.contains('\n') && !s.contains('\x1b')));
    }
    let mut p = params();
    p.custom_extensions
        .push(extension(&[2, 5, 29, 17], vec![0x30, 0x02, 0xa3, 0x00])); // X.400: deliberately unsupported
    p.custom_extensions.push(extension(
        &[2, 5, 29, 37],
        vec![0x30, 0x04, 0x06, 0x02, 0x2a, 0x03],
    )); // 1.2.3
    let entry = inspect(&certificate(p).0);
    assert!(has(&entry, ScFindingArea::San, ScFindingKind::Unsupported));
    assert!(has(
        &entry,
        ScFindingArea::ExtendedKeyUsage,
        ScFindingKind::Unsupported
    ));
    assert_eq!(entry.extended_key_usage.values, ["1.2.3"]);
    let mut p = params();
    p.distinguished_name
        .push(rcgen::DnType::CommonName, "x\n\u{202e}".repeat(1000));
    let members = tlv(0x86, &vec![b'a'; 1000]).repeat(20);
    p.custom_extensions
        .push(extension(&[2, 5, 29, 17], tlv(0x30, &members)));
    let entry = inspect(&certificate(p).0);
    assert!(entry.metadata.as_ref().unwrap().text_truncated());
    assert!(has(&entry, ScFindingArea::San, ScFindingKind::Truncated));
    assert!(!entry.san.assessed); // extension-work bound, before interpretation
    assert!(entry.san.values.is_empty());
}

#[test]
fn extension_count_values_and_display_limits_are_explicit() {
    let mut p = params();
    let members = (0..20)
        .flat_map(|_| tlv(0x86, &vec![b'a'; 300]))
        .collect::<Vec<_>>();
    p.custom_extensions
        .push(extension(&[2, 5, 29, 17], tlv(0x30, &members)));
    let entry = inspect(&certificate(p).0);
    assert_eq!(entry.san.values.len(), 16);
    assert!(entry.san.values.iter().all(|s| s.len() <= 256));
    assert!(has(&entry, ScFindingArea::San, ScFindingKind::Truncated));
    let mut p = params();
    for n in 0..65 {
        p.custom_extensions
            .push(extension(&[1, 2, 3, n], vec![0x05, 0]));
    }
    let entry = inspect(&certificate(p).0);
    assert!(!entry.san.assessed);
    assert!(has(
        &entry,
        ScFindingArea::Certificate,
        ScFindingKind::Truncated
    ));
}

#[test]
fn input_limits_empty_optionals_and_secret_pem_errors_are_explicit() {
    let (bytes, _) = certificate(params());
    for (trust, key) in [(Some(b"".as_slice()), None), (None, Some(b"".as_slice()))] {
        assert!(inspect_sc_pem(&bytes, trust, key, Some(NOW)).is_err());
    }
    assert!(matches!(
        inspect_sc_pem(&vec![b' '; MAX_INPUT_BYTES + 1], None, None, Some(NOW)),
        Err(ScInspectionError::Input {
            kind: ScInputErrorKind::InputTooLarge,
            ..
        })
    ));
    assert!(matches!(
        inspect_sc_pem(
            &pem("CERTIFICATE", &vec![0; MAX_CERTIFICATE_BYTES + 1]),
            None,
            None,
            Some(NOW)
        ),
        Err(ScInspectionError::Input {
            kind: ScInputErrorKind::CertificateTooLarge,
            ..
        })
    ));
    let many = pem("CERTIFICATE", b"abc").repeat(65);
    assert!(matches!(
        inspect_sc_pem(&many, None, None, Some(NOW)),
        Err(ScInspectionError::Input {
            index: Some(64),
            kind: ScInputErrorKind::TooManyCertificates,
            ..
        })
    ));
    let at_limit = pem("CERTIFICATE", b"abc").repeat(MAX_CERTIFICATES);
    assert_eq!(
        inspect_sc_pem(&at_limit, None, None, Some(NOW))
            .unwrap()
            .certificates
            .len(),
        MAX_CERTIFICATES
    );
    let mut padded = bytes.clone();
    padded.resize(MAX_INPUT_BYTES, b' ');
    assert!(inspect_sc_pem(&padded, None, None, Some(NOW)).is_ok());
    assert!(matches!(
        inspect_sc_pem(&padded, None, Some(b"x"), Some(NOW)),
        Err(ScInspectionError::Input {
            role: ScInputRole::PrivateKey,
            kind: ScInputErrorKind::InputTooLarge,
            ..
        })
    ));
    for secret in [
        b"-----BEGIN SENTINEL-SECRET invalid\n".as_slice(),
        b"-----BEGIN SENTINEL-SECRET-----\n",
    ] {
        let error = inspect_sc_pem(&bytes, None, Some(secret), Some(NOW)).unwrap_err();
        let display = format!("{error} {error:?}");
        assert!(!display.contains("SENTINEL"));
        assert!(!display.contains("83, 69, 78, 84, 73, 78, 69, 76"));
        assert!(display.len() < 250);
    }
}

#[test]
fn file_reads_preserve_errno_bound_size_and_retain_only_owned_observations() {
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temp =
        Temp(std::env::temp_dir().join(format!("sc-offline-materials-{}", std::process::id())));
    std::fs::create_dir(&temp.0).unwrap();
    let cert_path = temp.0.join("cert.pem");
    let key_path = temp.0.join("key.pem");
    let (bytes, key) = certificate(params());
    std::fs::write(&cert_path, &bytes).unwrap();
    std::fs::write(&key_path, &key).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let report = inspect_sc_pem_files(&cert_path, None, Some(&key_path), Some(NOW)).unwrap();
    let mut output = Vec::new();
    let example_report =
        example::inspect_and_write(&mut output, &cert_path, None, Some(&key_path), Some(NOW))
            .unwrap();
    assert_eq!(example_report, report);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("key consistency: match"));
    assert!(output.contains("CN=offline leaf"));
    assert!(!output.contains("PRIVATE KEY"));
    let bytes_report = inspect_sc_pem(&bytes, None, Some(&key), Some(NOW)).unwrap();
    assert_eq!(report.certificates, bytes_report.certificates);
    assert_eq!(report.key_consistency, bytes_report.key_consistency);
    #[cfg(unix)]
    assert_eq!(
        report.key_file_permissions,
        Some(ScKeyFilePermissions {
            group_readable: true,
            world_readable: false
        })
    );
    #[cfg(not(unix))]
    assert_eq!(report.key_file_permissions, None);
    std::fs::remove_file(&cert_path).unwrap();
    std::fs::remove_file(&key_path).unwrap();
    assert_eq!(
        report.certificates[0].metadata.as_ref().unwrap().subject(),
        "CN=offline leaf"
    );
    let err = inspect_sc_pem_files(&cert_path, None, None, Some(NOW)).unwrap_err();
    assert!(
        matches!(err, ScInspectionError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound && source.raw_os_error().is_some())
    );
    let file = std::fs::File::create(&cert_path).unwrap();
    file.set_len(MAX_INPUT_BYTES as u64 + 1).unwrap();
    assert!(matches!(
        inspect_sc_pem_files(&cert_path, None, None, Some(NOW)),
        Err(ScInspectionError::Input {
            kind: ScInputErrorKind::InputTooLarge,
            ..
        })
    ));
}

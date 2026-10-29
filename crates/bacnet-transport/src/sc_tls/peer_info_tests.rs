use super::*;

#[test]
fn display_metadata_is_owned_bounded_and_not_verification() {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "fixture\n\u{1b}[2J\u{202e}name");
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2025, 1, 1);
    let cert = params.self_signed(&key).unwrap();
    let info = ScPeerCertificate::new(cert.der());
    assert_eq!(info.der(), cert.der().as_ref());
    let meta = info.metadata().unwrap();
    assert_eq!(meta.subject(), "CN=fixture\\n\\u{1b}[2J\\u{202e}name");
    assert_eq!(meta.issuer(), meta.subject());
    assert_eq!(meta.not_before_unix(), 1_704_067_200);
    assert_eq!(meta.not_after_unix(), 1_735_689_600);
    assert_eq!(meta.public_key_algorithm_oid(), "1.2.840.10045.2.1");
    assert_eq!(meta.signature_algorithm_oid(), "1.2.840.10045.4.3.2");
    assert!(!meta.text_truncated());
    assert!(!format!("{info:?} {meta:?}").contains("fixture"));
    assert!(!format!("{info:?}").contains(&format!("{:?}", cert.der().as_ref())));
    // No reference into the producer's DER or key survives.
    drop(cert);
    drop(key);
    assert_eq!(info.metadata().unwrap().not_after_unix(), 1_735_689_600);

    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "A".repeat(5000));
    let cert = params
        .self_signed(&rcgen::KeyPair::generate().unwrap())
        .unwrap();
    let info = ScPeerCertificate::new(cert.der());
    assert!(info.metadata().unwrap().text_truncated());
    assert_eq!(info.metadata().unwrap().subject().len(), 1024);
    assert_eq!(info.der(), cert.der().as_ref());
}

#[test]
fn parser_failure_and_trailing_bytes_preserve_exact_der() {
    let malformed = [0xff, 0, 1];
    let info = ScPeerCertificate::new(&malformed);
    assert_eq!(info.der(), malformed);
    assert!(info.metadata().is_none());
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let mut der = cert.cert.der().to_vec();
    der.push(0);
    let info = ScPeerCertificate::new(&der);
    assert_eq!(info.der(), der);
    assert!(info.metadata().is_none());
}

#[test]
fn subject_alt_names_are_read_in_order_and_hidden_from_debug() {
    use rcgen::{OtherNameValue, SanType};
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.subject_alt_names = vec![
        SanType::DnsName("operator.example".try_into().unwrap()),
        SanType::OtherName((vec![1, 2, 3], OtherNameValue::Utf8String("skipped".into()))),
        SanType::IpAddress("10.0.0.7".parse().unwrap()),
        SanType::IpAddress("fd00::7".parse().unwrap()),
        SanType::URI("urn:site:operator".try_into().unwrap()),
        SanType::Rfc822Name("ops@example.com".try_into().unwrap()),
    ];
    let cert = params.self_signed(&key).unwrap();
    let info = ScPeerCertificate::new(cert.der());
    let meta = info.metadata().unwrap();
    assert_eq!(
        meta.subject_alt_names(),
        [
            ScSubjectAltName::Dns("operator.example".into()),
            ScSubjectAltName::Ip("10.0.0.7".parse().unwrap()),
            ScSubjectAltName::Ip("fd00::7".parse().unwrap()),
            ScSubjectAltName::Uri("urn:site:operator".into()),
            ScSubjectAltName::Email("ops@example.com".into()),
        ]
    );
    let debug = format!("{info:?} {meta:?} {:?}", meta.subject_alt_names());
    for secret in ["operator", "10.0.0.7", "fd00", "ops@"] {
        assert!(!debug.contains(secret), "{debug}");
    }

    // No extension, no names.
    params.subject_alt_names.clear();
    let cert = params.self_signed(&key).unwrap();
    let info = ScPeerCertificate::new(cert.der());
    assert!(info.metadata().unwrap().subject_alt_names().is_empty());
}

/// A raw Subject Alternative Name extension (OID 2.5.29.17) with `content`.
fn raw_san(content: &[u8]) -> rcgen::CustomExtension {
    rcgen::CustomExtension::from_oid_content(&[2, 5, 29, 17], content.to_vec())
}

// GeneralNames holding the one dNSName "b".
const SAN_B: [u8; 5] = [0x30, 0x03, 0x82, 0x01, b'b'];

#[test]
fn a_repeated_subject_alt_name_extension_gives_no_names() {
    // The parser accepts the certificate; the names it would give are
    // ambiguous, so none are returned.
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["a".into()]).unwrap();
    params.custom_extensions = vec![raw_san(&SAN_B)];
    let cert = params.self_signed(&key).unwrap();
    let info = ScPeerCertificate::new(cert.der());
    assert!(info.metadata().unwrap().subject_alt_names().is_empty());

    // Either extension alone gives its name.
    params.custom_extensions.clear();
    let cert = params.self_signed(&key).unwrap();
    let names = ScPeerCertificate::new(cert.der());
    assert_eq!(
        names.metadata().unwrap().subject_alt_names(),
        [ScSubjectAltName::Dns("a".into())]
    );
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.custom_extensions = vec![raw_san(&SAN_B)];
    let cert = params.self_signed(&key).unwrap();
    let names = ScPeerCertificate::new(cert.der());
    assert_eq!(
        names.metadata().unwrap().subject_alt_names(),
        [ScSubjectAltName::Dns("b".into())]
    );
}

#[test]
fn trailing_bytes_after_the_subject_alt_names_give_no_names() {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    let mut trailing = SAN_B.to_vec();
    trailing.push(0);
    params.custom_extensions = vec![raw_san(&trailing)];
    let cert = params.self_signed(&key).unwrap();
    let info = ScPeerCertificate::new(cert.der());
    assert!(info.metadata().unwrap().subject_alt_names().is_empty());
}

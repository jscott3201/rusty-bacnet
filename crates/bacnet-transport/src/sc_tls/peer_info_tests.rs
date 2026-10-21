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

use super::*;

#[test]
fn exact_time_boundaries_and_extreme_seconds_do_not_overflow() {
    for (now, expected) in [
        (0, ScExpiry::Warning),
        (1, ScExpiry::Alarm),
        (604_801, ScExpiry::Alarm),
    ] {
        assert_eq!(
            assess_time(0, 604_801, now),
            (ScValidity::WithinInterval, expected)
        );
    }
    assert_eq!(
        assess_time(0, 2_592_001, 0),
        (ScValidity::WithinInterval, ScExpiry::None)
    );
    assert_eq!(
        assess_time(0, 2_592_001, 1),
        (ScValidity::WithinInterval, ScExpiry::Warning)
    );
    assert_eq!(
        assess_time(2, 1, 1),
        (ScValidity::InvalidInterval, ScExpiry::NotAssessed)
    );
    assert_eq!(
        assess_time(0, 1, -1),
        (ScValidity::NotYetValid, ScExpiry::NotAssessed)
    );
    assert_eq!(
        assess_time(0, 1, 2),
        (ScValidity::Expired, ScExpiry::NotAssessed)
    );
    assert_eq!(
        assess_time(i64::MIN, i64::MAX, i64::MIN),
        (ScValidity::WithinInterval, ScExpiry::None)
    );
    assert_eq!(
        assess_time(i64::MIN, i64::MAX, i64::MAX),
        (ScValidity::WithinInterval, ScExpiry::Alarm)
    );
}

#[test]
fn unknown_consistency_is_never_promoted_to_match() {
    #[derive(Debug)]
    struct OpaqueKey;
    impl rustls::sign::SigningKey for OpaqueKey {
        fn choose_scheme(
            &self,
            _: &[rustls::SignatureScheme],
        ) -> Option<Box<dyn rustls::sign::Signer>> {
            None
        }
        fn algorithm(&self) -> rustls::SignatureAlgorithm {
            rustls::SignatureAlgorithm::ED25519
        }
    }
    // Exercise Rustls's real unavailable-SPKI branch, not constructor success.
    let key = CertifiedKey::new(
        vec![CertificateDer::from(vec![0])],
        std::sync::Arc::new(OpaqueKey),
    );
    assert_eq!(classify_match(key.keys_match()), ScKeyConsistency::Unknown);
    assert_eq!(
        classify_match(Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::BadEncoding
        ))),
        ScKeyConsistency::Unknown
    );
}

fn ca(name: &str) -> (rcgen::CertificateParams, rcgen::KeyPair) {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, name);
    (params, rcgen::KeyPair::generate().unwrap())
}

fn signed(
    params: &rcgen::CertificateParams,
    key: &rcgen::KeyPair,
    by: &(rcgen::CertificateParams, rcgen::KeyPair),
) -> String {
    params
        .signed_by(key, &rcgen::Issuer::from_params(&by.0, &by.1))
        .unwrap()
        .pem()
}

#[test]
fn direct_issuer_needs_a_bundle_ca_that_signed_the_leaf_itself() {
    let root = ca("site root");
    let root_pem = root.0.self_signed(&root.1).unwrap().pem();
    let intermediate = ca("site issuing");
    let intermediate_pem = signed(&intermediate.0, &intermediate.1, &root);
    let node = || rcgen::CertificateParams::new(vec!["node".into()]).unwrap();
    let node_key = rcgen::KeyPair::generate().unwrap();
    let leaf = signed(&node(), &node_key, &intermediate);
    let direct = signed(&node(), &node_key, &root);
    let check = |identity: &str, trust: Option<&str>| {
        let report =
            inspect_sc_pem(identity.as_bytes(), trust.map(str::as_bytes), None, Some(0)).unwrap();
        (report.direct_issuer, report.direct_issuer_index)
    };
    let both = format!("{root_pem}{intermediate_pem}");
    assert_eq!(
        check(&direct, Some(&root_pem)),
        (ScDirectIssuer::Direct, Some(0))
    );
    // Signed by an intermediate the bundle lacks: it chains to the root, but
    // not directly.
    assert_eq!(
        check(&leaf, Some(&root_pem)),
        (ScDirectIssuer::NotDirect, None)
    );
    // A chain in the identity file does not change the leaf's issuer.
    assert_eq!(
        check(&format!("{leaf}{intermediate_pem}"), Some(&root_pem)),
        (ScDirectIssuer::NotDirect, None)
    );
    assert_eq!(check(&leaf, Some(&both)), (ScDirectIssuer::Direct, Some(1)));
    // Same subject name, another key: the signature decides.
    let namesake = ca("site root");
    let namesake_pem = namesake.0.self_signed(&namesake.1).unwrap().pem();
    assert_eq!(
        check(&direct, Some(&namesake_pem)),
        (ScDirectIssuer::NotDirect, None)
    );
    assert_eq!(check(&direct, None), (ScDirectIssuer::NotAssessed, None));
    let malformed = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n";
    assert_eq!(
        check(malformed, Some(&root_pem)),
        (ScDirectIssuer::Unknown, None)
    );
}

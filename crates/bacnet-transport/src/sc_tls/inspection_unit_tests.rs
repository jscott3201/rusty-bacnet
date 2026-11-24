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
fn a_pair_rustls_cannot_compare_is_unknown_and_a_refused_load_unusable() {
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
    // Rustls's real unavailable-SPKI verdict, as an error.
    let key = CertifiedKey::new(
        vec![CertificateDer::from(vec![0])],
        std::sync::Arc::new(OpaqueKey),
    );
    assert_eq!(classify_match(key.keys_match()), ScKeyConsistency::Unknown);
    for unknown in [
        rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding),
        rustls::Error::NoCertificatesPresented,
    ] {
        assert_eq!(classify_match(Err(unknown)), ScKeyConsistency::Unknown);
    }
    assert_eq!(
        classify_match(Err(rustls::Error::General("no key".into()))),
        ScKeyConsistency::Unusable
    );
}

/// `from_der` takes a key that can't name its public key as a match, so
/// `compare_key` reports success as Match only because every key type the
/// pinned provider loads names one. A provider upgrade that drops one of
/// these fails here. (ML-DSA keys, which rcgen can't make here, name theirs
/// too in rustls 0.23.46.)
#[test]
fn every_key_type_the_pinned_provider_loads_names_its_public_key() {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    for alg in [
        &rcgen::PKCS_RSA_SHA256,
        &rcgen::PKCS_ECDSA_P256_SHA256,
        &rcgen::PKCS_ECDSA_P384_SHA384,
        &rcgen::PKCS_ECDSA_P521_SHA512,
        &rcgen::PKCS_ED25519,
    ] {
        let pair = rcgen::KeyPair::generate_for(alg).unwrap();
        let der = rustls::pki_types::PrivatePkcs8KeyDer::from(pair.serialize_der());
        let key = provider.key_provider.load_private_key(der.into()).unwrap();
        assert!(key.public_key().is_some(), "{alg:?}");
    }
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

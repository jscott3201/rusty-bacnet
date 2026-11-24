//! The PEM constructors of both SC TLS configurations (#2600): the key forms
//! they read, the text around PEM they ignore, each typed error, the order
//! inputs are checked in, and no key material in any error. Reading files
//! and a real connect are in `sc_tls_pem_files.rs`.
#![cfg(feature = "sc-tls")]

use bacnet_transport::sc_tls::{
    ScCertificateForm, ScKeyForm, ScNodeTlsConfig, ScPemFault, ScTlsConfigError, ScTlsInput,
};
use rcgen::KeyPair;

#[allow(dead_code)]
#[path = "support/sc_pem_materials.rs"]
mod fixture;
use fixture::{legacy_encrypted_pem, pem};
#[allow(dead_code)]
#[path = "support/sc_tls_site.rs"]
mod site;
use site::{Site, both, cr, crlf, hub, lf, node, text, unwrap_pkcs8};

#[test]
fn pem_constructors_read_pkcs8_sec1_and_pkcs1_keys_with_an_intermediate() {
    let site = Site::new();
    let ec = site.leaf();
    let rsa = site.leaf_with(KeyPair::generate_for(&rcgen::PKCS_RSA_SHA256).unwrap());
    let ed = site.leaf_with(KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap());
    let sec1 = text(pem(
        "EC PRIVATE KEY",
        &unwrap_pkcs8(&ec.key.serialize_der()),
    ));
    // ECParameters naming prime256v1 (OID 1.2.840.10045.3.1.7).
    const EC_P256_PARAMETERS: [u8; 10] =
        [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
    let pkcs1 = text(pem(
        "RSA PRIVATE KEY",
        &unwrap_pkcs8(&rsa.key.serialize_der()),
    ));
    for (form, leaf, key) in [
        ("PKCS#8 P-256", &ec, ec.key.serialize_pem()),
        ("SEC1 P-256", &ec, sec1.clone()),
        ("PKCS#8 RSA", &rsa, rsa.key.serialize_pem()),
        ("PKCS#1 RSA", &rsa, pkcs1),
        ("PKCS#8 Ed25519", &ed, ed.key.serialize_pem()),
        // openssl ecparam -genkey writes the curve before the key.
        (
            "EC PARAMETERS then SEC1",
            &ec,
            format!("{}{sec1}", text(pem("EC PARAMETERS", &EC_P256_PARAMETERS))),
        ),
    ] {
        both(&site.root, &leaf.chain, &key).unwrap_or_else(|e| panic!("{form}: {e}"));
    }
}

#[test]
fn pem_text_around_sections_crlf_and_other_sections_are_taken() {
    let site = Site::new();
    let leaf = site.leaf();
    let key = leaf.key.serialize_pem();
    // openssl pkcs12 writes "Bag Attributes" before each section; text after
    // the last section and a certificate beside the key are ignored too.
    let ca = format!(
        "Bag Attributes\n    friendlyName: root\n{}trailing text\n",
        site.root
    );
    let chain = format!("subject=CN = localhost\n{}\n\ntrailing", leaf.chain);
    let key_file = format!("{}{key}-- end of file --\n", site.root);
    both(&ca, &chain, &key_file).unwrap();
    both(&crlf(&ca), &crlf(&chain), &crlf(&key_file)).unwrap();
    // The parser takes a lone CR as a line end too, so PEM with old Mac
    // line ends loads: well-formed PEM, nothing to refuse.
    both(&cr(&ca), &cr(&chain), &cr(&key_file)).unwrap();
    // A bundle of several CAs, the one that issued last.
    let other = Site::new();
    both(&format!("{}{}", other.root, site.root), &leaf.chain, &key).unwrap();
    // A revocation list and a key in the bundle or chain are skipped.
    let crl = text(pem("X509 CRL", b"not applied"));
    both(
        &format!("{crl}{}{key}", site.root),
        &format!("{}{key}", leaf.chain),
        &key,
    )
    .unwrap();
}

/// OpenSSL's older encrypted keys keep the SEC1 or PKCS#1 label with
/// `Proc-Type`/`DEK-Info` headers: an encrypted key, not malformed PEM.
/// Made-up bodies; no key material.
#[test]
fn legacy_encrypted_keys_are_named_encrypted() {
    let site = Site::new();
    let leaf = site.leaf();
    for label in ["RSA PRIVATE KEY", "EC PRIVATE KEY"] {
        let legacy = legacy_encrypted_pem(label, "bm90IGEga2V5OyBhIHRlc3QgZml4dHVyZSBib2R5");
        for key in [legacy.clone(), crlf(&legacy)] {
            let error = both(&site.root, &leaf.chain, &key).unwrap_err();
            assert!(
                matches!(
                    error,
                    ScTlsConfigError::UnsupportedKey {
                        form: ScKeyForm::Encrypted,
                        ..
                    }
                ),
                "{label}: {error:?}"
            );
        }
    }
    // Beside a readable key, in either order, it is a second key.
    let legacy = legacy_encrypted_pem("RSA PRIVATE KEY", "bm90IGEga2V5");
    let key = leaf.key.serialize_pem();
    for both_keys in [format!("{key}{legacy}"), format!("{legacy}{key}")] {
        let error = both(&site.root, &leaf.chain, &both_keys).unwrap_err();
        assert!(
            matches!(
                error,
                ScTlsConfigError::MultiplePrivateKeys { count: 2, .. }
            ),
            "{error:?}"
        );
    }
    // The header counts only as a key section's first line: in leading
    // text, or opening a corrupt certificate beside a good key, it leaves
    // that certificate malformed PEM.
    let header = "Proc-Type: 4,ENCRYPTED\n";
    let corrupt = "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n";
    let in_certificate =
        format!("-----BEGIN CERTIFICATE-----\n{header}AQID\n-----END CERTIFICATE-----\n");
    for key_file in [
        format!("{header}{corrupt}{key}"),
        format!("{in_certificate}{key}"),
    ] {
        let error = both(&site.root, &leaf.chain, &key_file).unwrap_err();
        assert!(
            matches!(
                error,
                ScTlsConfigError::MalformedPem {
                    input: ScTlsInput::PrivateKey,
                    fault: ScPemFault::Base64,
                    ..
                }
            ),
            "{error:?}"
        );
    }
    // Without the header, the same bad body is malformed PEM.
    // The label is joined at run time, as in `legacy_encrypted_pem`.
    let rsa = "RSA PRIVATE KEY";
    let bad = format!("-----BEGIN {rsa}-----\nDEK-Info: x\n-----END {rsa}-----\n");
    let error = both(&site.root, &leaf.chain, &bad).unwrap_err();
    assert!(
        matches!(
            error,
            ScTlsConfigError::MalformedPem {
                fault: ScPemFault::Base64,
                ..
            }
        ),
        "{error:?}"
    );
}

/// A bundle or chain whose only certificates are in a container the stack
/// doesn't read says which container.
#[test]
fn trusted_certificate_and_pkcs7_only_inputs_name_their_form() {
    let site = Site::new();
    let leaf = site.leaf();
    let key = leaf.key.serialize_pem();
    let trusted = text(pem("TRUSTED CERTIFICATE", b"cert and trust"));
    let pkcs7 = text(pem("PKCS7", b"signed data"));
    let cms = text(pem("CMS", b"signed data"));
    for (ca, chain, input, form) in [
        (
            trusted.as_str(),
            leaf.chain.as_str(),
            ScTlsInput::CaBundle,
            ScCertificateForm::Trusted,
        ),
        (
            site.root.as_str(),
            pkcs7.as_str(),
            ScTlsInput::CertificateChain,
            ScCertificateForm::Pkcs7,
        ),
        (
            cms.as_str(),
            leaf.chain.as_str(),
            ScTlsInput::CaBundle,
            ScCertificateForm::Pkcs7,
        ),
    ] {
        let error = both(ca, chain, &key).unwrap_err();
        assert!(
            matches!(&error, ScTlsConfigError::UnsupportedCertificate { input: i, form: f, .. }
                if *i == input && *f == form),
            "{error:?}"
        );
        assert_eq!(error.input(), Some(input));
    }
    // Beside a plain certificate they are skipped.
    both(&format!("{trusted}{}", site.root), &leaf.chain, &key).unwrap();
}

/// What each kind of bad input is refused with, through `from_pem`; the
/// input it names; and that the node and hub agree.
#[test]
fn each_pem_fault_is_its_own_variant_naming_the_input() {
    let site = Site::new();
    let leaf = site.leaf();
    let key = leaf.key.serialize_pem();
    let bad_der = "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n";
    let unended = "-----BEGIN CERTIFICATE-----\nAQID\n";
    let bad_begin = "-----BEGIN CERTIFICATE----\nAQID\n-----END CERTIFICATE-----\n";
    let bad_base64 = "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n";
    let (ca, chain) = (site.root.as_str(), leaf.chain.as_str());
    use ScTlsInput::{CaBundle as Ca, CertificateChain as Chain, PrivateKey as Key};
    let malformed = |input, fault| {
        move |e: &ScTlsConfigError| {
            matches!(e, ScTlsConfigError::MalformedPem { input: i, fault: f, .. }
                if *i == input && *f == fault)
        }
    };
    let no_certificate = |input| move |e: &ScTlsConfigError| matches!(e, ScTlsConfigError::NoCertificate { input: i, .. } if *i == input);
    let unusable = |input, index| {
        move |e: &ScTlsConfigError| {
            matches!(e, ScTlsConfigError::UnusableCertificate { input: i, index: n, .. }
                if *i == input && *n == index)
        }
    };
    let other_key = KeyPair::generate().unwrap().serialize_pem();
    let encrypted = text(pem("ENCRYPTED PRIVATE KEY", b"sealed"));
    let openssh = text(pem("OPENSSH PRIVATE KEY", b"openssh"));
    let dsa = text(pem("DSA PRIVATE KEY", b"dsa"));
    let garbage_key = text(pem("PRIVATE KEY", &[1, 2, 3]));
    // Key labels are joined at run time, as in `pem`: no tracked file
    // carries a key BEGIN line (check-no-secrets.sh).
    let pkcs8 = "PRIVATE KEY";
    let bad_base64_key = format!("-----BEGIN {pkcs8}-----\n!\n-----END {pkcs8}-----\n");
    type Check = Box<dyn Fn(&ScTlsConfigError) -> bool>;
    let cases: Vec<(&str, [String; 3], Option<ScTlsInput>, Check)> = vec![
        (
            "CA missing END",
            [unended.into(), chain.into(), key.clone()],
            Some(Ca),
            Box::new(malformed(Ca, ScPemFault::MissingEnd)),
        ),
        (
            "CA bad BEGIN",
            [bad_begin.into(), chain.into(), key.clone()],
            Some(Ca),
            Box::new(malformed(Ca, ScPemFault::MalformedBegin)),
        ),
        (
            "CA bad base64",
            [bad_base64.into(), chain.into(), key.clone()],
            Some(Ca),
            Box::new(malformed(Ca, ScPemFault::Base64)),
        ),
        (
            "CA empty",
            [String::new(), chain.into(), key.clone()],
            Some(Ca),
            Box::new(no_certificate(Ca)),
        ),
        (
            "CA not PEM",
            ["not PEM".into(), chain.into(), key.clone()],
            Some(Ca),
            Box::new(no_certificate(Ca)),
        ),
        (
            "CA only a key",
            [key.clone(), chain.into(), key.clone()],
            Some(Ca),
            Box::new(no_certificate(Ca)),
        ),
        (
            "CA bad DER second",
            [format!("{ca}{bad_der}"), chain.into(), key.clone()],
            Some(Ca),
            Box::new(unusable(Ca, 1)),
        ),
        (
            "chain empty",
            [ca.into(), String::new(), key.clone()],
            Some(Chain),
            Box::new(no_certificate(Chain)),
        ),
        (
            "chain bad base64",
            [ca.into(), format!("{chain}{bad_base64}"), key.clone()],
            Some(Chain),
            Box::new(malformed(Chain, ScPemFault::Base64)),
        ),
        (
            "chain bad DER after the intermediate",
            [ca.into(), format!("{chain}{bad_der}"), key.clone()],
            Some(Chain),
            Box::new(unusable(Chain, 2)),
        ),
        (
            "key empty",
            [ca.into(), chain.into(), String::new()],
            Some(Key),
            Box::new(|e| matches!(e, ScTlsConfigError::NoPrivateKey)),
        ),
        (
            "key only a certificate",
            [ca.into(), chain.into(), chain.into()],
            Some(Key),
            Box::new(|e| matches!(e, ScTlsConfigError::NoPrivateKey)),
        ),
        (
            "key bad base64",
            [ca.into(), chain.into(), bad_base64_key],
            Some(Key),
            Box::new(malformed(Key, ScPemFault::Base64)),
        ),
        (
            "two keys",
            [ca.into(), chain.into(), format!("{key}{other_key}")],
            Some(Key),
            Box::new(|e| matches!(e, ScTlsConfigError::MultiplePrivateKeys { count: 2, .. })),
        ),
        (
            "key and an encrypted key",
            [ca.into(), chain.into(), format!("{encrypted}{key}")],
            Some(Key),
            Box::new(|e| matches!(e, ScTlsConfigError::MultiplePrivateKeys { count: 2, .. })),
        ),
        (
            "encrypted key",
            [ca.into(), chain.into(), encrypted.clone()],
            Some(Key),
            Box::new(|e| {
                matches!(
                    e,
                    ScTlsConfigError::UnsupportedKey {
                        form: ScKeyForm::Encrypted,
                        ..
                    }
                )
            }),
        ),
        (
            "OpenSSH key",
            [ca.into(), chain.into(), openssh],
            Some(Key),
            Box::new(|e| {
                matches!(
                    e,
                    ScTlsConfigError::UnsupportedKey {
                        form: ScKeyForm::OpenSsh,
                        ..
                    }
                )
            }),
        ),
        (
            "DSA key",
            [ca.into(), chain.into(), crlf(&dsa)],
            Some(Key),
            Box::new(|e| {
                matches!(
                    e,
                    ScTlsConfigError::UnsupportedKey {
                        form: ScKeyForm::Other,
                        ..
                    }
                )
            }),
        ),
        (
            "key DER garbage",
            [ca.into(), chain.into(), garbage_key],
            Some(Key),
            Box::new(|e| matches!(e, ScTlsConfigError::UnusableKey { .. })),
        ),
        (
            "another leaf's key",
            [ca.into(), chain.into(), other_key.clone()],
            Some(Key),
            Box::new(|e| matches!(e, ScTlsConfigError::KeyMismatch)),
        ),
    ];
    for (name, [ca, chain, key], input, check) in cases {
        let error = both(&ca, &chain, &key).expect_err(name);
        assert!(check(&error), "{name}: {error:?}");
        assert_eq!(error.input(), input, "{name}");
        assert!(!error.to_string().is_empty(), "{name}");
    }
}

/// The order inputs are checked in: the CA bundle whole and first, block by
/// block; then both identity inputs decoded before either is checked.
#[test]
fn inputs_are_checked_ca_first_then_decoded_before_der_checks() {
    let site = Site::new();
    let leaf = site.leaf();
    let key = leaf.key.serialize_pem();
    let bad_der = "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n";
    let bad_pem = "-----BEGIN CERTIFICATE-----\n!\n-----END CERTIFICATE-----\n";
    // A later CA block is not decoded before an earlier one is checked.
    let error = both(&format!("{bad_der}{bad_pem}"), &leaf.chain, &key).unwrap_err();
    assert!(
        matches!(
            error,
            ScTlsConfigError::UnusableCertificate {
                input: ScTlsInput::CaBundle,
                index: 0,
                ..
            }
        ),
        "{error:?}"
    );
    let error = both(&format!("{bad_pem}{bad_der}"), &leaf.chain, &key).unwrap_err();
    assert!(
        matches!(
            error,
            ScTlsConfigError::MalformedPem {
                input: ScTlsInput::CaBundle,
                ..
            }
        ),
        "{error:?}"
    );
    // A bad CA wins over every identity fault.
    let error = both(bad_der, "", "").unwrap_err();
    assert_eq!(error.input(), Some(ScTlsInput::CaBundle));
    // An undecodable key wins over a chain certificate that decodes but
    // can't be used, and an empty chain over any key fault.
    let error = both(&site.root, bad_der, "").unwrap_err();
    assert!(matches!(error, ScTlsConfigError::NoPrivateKey), "{error:?}");
    let error = both(&site.root, "", "").unwrap_err();
    assert!(
        matches!(
            error,
            ScTlsConfigError::NoCertificate {
                input: ScTlsInput::CertificateChain,
                ..
            }
        ),
        "{error:?}"
    );
    // DER too: an empty CA list before an empty chain.
    let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(leaf.key.serialize_der());
    let error = ScNodeTlsConfig::from_der(vec![], vec![], key_der.into()).unwrap_err();
    assert!(
        matches!(
            error,
            ScTlsConfigError::NoCertificate {
                input: ScTlsInput::CaBundle,
                ..
            }
        ),
        "{error:?}"
    );
}

/// No error and no configuration shows key material: neither the key's
/// base64 nor its PEM label, in `Debug` or `Display`, whatever is wrong.
#[test]
fn no_error_or_configuration_shows_key_material() {
    let site = Site::new();
    let leaf = site.leaf();
    let wrong_leaf = site.leaf();
    // rcgen writes CRLF on Windows and LF elsewhere; both renderings run
    // here on every platform. The malformed forms start from LF text: the
    // parser takes a lone CR as a line end, so removing only the LFs from
    // CRLF text would leave a key that still loads.
    let generated = leaf.key.serialize_pem();
    for (ending, key) in [("LF", lf(&generated)), ("CRLF", crlf(&generated))] {
        let body: String = key.lines().nth(1).unwrap().chars().take(24).collect();
        let one_line = key.replace(['\r', '\n'], "");
        let key_lf = lf(&key);
        let cases = [
            (site.root.clone(), leaf.chain.clone(), one_line.clone()),
            (site.root.clone(), leaf.chain.clone(), format!("{key}{key}")),
            (
                site.root.clone(),
                leaf.chain.clone(),
                key_lf.replace("-----END", "-----BEGIN"),
            ),
            (
                site.root.clone(),
                leaf.chain.clone(),
                key_lf.replacen('\n', "!\n", 2),
            ),
            (site.root.clone(), wrong_leaf.chain.clone(), key.clone()),
            (key.clone(), leaf.chain.clone(), key.clone()),
            (one_line.clone(), leaf.chain.clone(), key.clone()),
            (site.root.clone(), one_line.clone(), key.clone()),
            (
                site.root.clone(),
                leaf.chain.clone(),
                key_lf.replace("PRIVATE KEY", "ENCRYPTED PRIVATE KEY"),
            ),
        ];
        for (index, (ca, chain, key_text)) in cases.into_iter().enumerate() {
            let Err(error) = both(&ca, &chain, &key_text) else {
                panic!("{ending} case {index} loaded, but every case here is malformed");
            };
            for shown in [format!("{error}"), format!("{error:?}")] {
                assert!(!shown.contains(&body), "key material shown: {shown}");
                assert!(!shown.contains("PRIVATE KEY"), "key label shown: {shown}");
            }
        }
        let config = node(&site.root, &leaf.chain, &key).unwrap();
        assert_eq!(format!("{config:?}"), "ScNodeTlsConfig { .. }");
        let config = hub(&site.root, &leaf.chain, &key).unwrap();
        assert_eq!(format!("{config:?}"), "ScHubTlsConfig { .. }");
    }
}

/// The key check is rustls's own (#2621). For the leaf's key, another leaf's
/// key and a key the provider can't load, `CertifiedKey::from_der` and the
/// rustls builders the configurations are built with reach one verdict; both
/// configurations, from PEM and from DER, reach that verdict mapped to our
/// errors, and the offline inspection reports that verdict. A rustls
/// upgrade that changes the verdict for any of these fails here.
#[test]
fn key_verdicts_are_the_ones_rustls_reaches() {
    use bacnet_transport::sc_hub::ScHubTlsConfig;
    use bacnet_transport::sc_tls::inspection::{ScKeyConsistency, inspect_sc_pem};
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::{Error, InconsistentKeys, sign::CertifiedKey};
    use std::sync::Arc;

    let site = Site::new();
    let leaf = site.leaf();
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(leaf.chain.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    let root = CertificateDer::from_pem_slice(site.root.as_bytes()).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root.clone()).unwrap();
    let roots = Arc::new(roots);
    let unloadable = "-----BEGIN PRIVATE KEY-----\nAQID\n-----END PRIVATE KEY-----\n";
    let cases = [
        (
            "the leaf's key",
            leaf.key.serialize_pem(),
            ScKeyConsistency::Match,
        ),
        (
            "another leaf's key",
            site.leaf().key.serialize_pem(),
            ScKeyConsistency::Mismatch,
        ),
        (
            "an unloadable key",
            unloadable.to_owned(),
            ScKeyConsistency::Unusable,
        ),
    ];
    for (name, key_pem, expected) in cases {
        let key = || PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let tls13 = [&rustls::version::TLS13];
        let library = CertifiedKey::from_der(chain.clone(), key(), &provider).map(drop);
        let client = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&tls13)
            .unwrap()
            .with_root_certificates(Arc::clone(&roots))
            .with_client_auth_cert(chain.clone(), key())
            .map(drop);
        let server = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_protocol_versions(&tls13)
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain.clone(), key())
            .map(drop);
        assert_eq!(client, library, "{name}: the client builder");
        assert_eq!(server, library, "{name}: the server builder");
        let verdict = match &library {
            Ok(()) => ScKeyConsistency::Match,
            Err(Error::InconsistentKeys(InconsistentKeys::KeyMismatch)) => {
                ScKeyConsistency::Mismatch
            }
            Err(_) => ScKeyConsistency::Unusable,
        };
        assert_eq!(verdict, expected, "{name}: rustls's verdict {library:?}");
        // Ours: a mismatch by name, any other refusal as an unusable key
        // carrying rustls's reason.
        let ours = both(&site.root, &leaf.chain, &key_pem);
        let agrees = match (&library, &ours) {
            (Ok(()), Ok(())) => true,
            (
                Err(Error::InconsistentKeys(InconsistentKeys::KeyMismatch)),
                Err(ScTlsConfigError::KeyMismatch),
            ) => true,
            (Err(Error::InconsistentKeys(_)), _) => false,
            (Err(reason), Err(ScTlsConfigError::UnusableKey { source, .. })) => reason == source,
            _ => false,
        };
        assert!(agrees, "{name}: rustls {library:?}, ours {ours:?}");
        let ours = ours.map_err(|error| error.to_string());
        let from_der = [
            ScNodeTlsConfig::from_der(vec![root.clone()], chain.clone(), key()).map(drop),
            ScHubTlsConfig::from_der(vec![root.clone()], chain.clone(), key()).map(drop),
        ];
        for result in from_der {
            assert_eq!(result.map_err(|e| e.to_string()), ours, "{name}: from_der");
        }
        let report = inspect_sc_pem(
            leaf.chain.as_bytes(),
            None,
            Some(key_pem.as_bytes()),
            Some(0),
        )
        .unwrap();
        // The offline check (the CLI's pre-dial lint) calls `from_der` too.
        assert_eq!(report.key_consistency, verdict, "{name}: the inspection");
    }
}

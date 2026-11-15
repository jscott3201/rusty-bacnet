//! The PEM constructors of both SC TLS configurations (#2600): the key forms
//! they read, the text around PEM they ignore, each typed error, the order
//! inputs are checked in, no key material in any error, and a real hub
//! connect through intermediates built from PEM files.
#![cfg(feature = "sc-tls")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::{
    ScKeyForm, ScNodeTlsConfig, ScPemFault, ScTlsConfigError, ScTlsInput, TlsWebSocket,
};
use rcgen::{CertificateParams, Issuer, KeyPair};

#[allow(dead_code)]
#[path = "support/sc_pem_materials.rs"]
mod fixture;
use fixture::pem;

/// A root CA, an intermediate it issued, and leaves the intermediate issued.
struct Site {
    root: String,
    intermediate: String,
    issuer: Issuer<'static, KeyPair>,
}

/// A leaf's certificate PEM, its chain (leaf then intermediate) and its key.
struct Leaf {
    chain: String,
    key: KeyPair,
}

impl Site {
    fn new() -> Self {
        let mut root = CertificateParams::new(Vec::<String>::new()).unwrap();
        root.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        root.distinguished_name
            .push(rcgen::DnType::CommonName, "PEM root");
        let root_key = KeyPair::generate().unwrap();
        let root_pem = root.self_signed(&root_key).unwrap().pem();
        let root_issuer = Issuer::from_params(&root, &root_key);
        let mut middle = CertificateParams::new(Vec::<String>::new()).unwrap();
        middle.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        middle
            .distinguished_name
            .push(rcgen::DnType::CommonName, "PEM intermediate");
        let middle_key = KeyPair::generate().unwrap();
        let intermediate = middle.signed_by(&middle_key, &root_issuer).unwrap().pem();
        Self {
            root: root_pem,
            intermediate,
            issuer: Issuer::new(middle, middle_key),
        }
    }

    fn leaf_with(&self, key: KeyPair) -> Leaf {
        let params = CertificateParams::new(vec!["localhost".into()]).unwrap();
        let cert = params.signed_by(&key, &self.issuer).unwrap().pem();
        Leaf {
            chain: format!("{cert}{}", self.intermediate),
            key,
        }
    }

    fn leaf(&self) -> Leaf {
        self.leaf_with(KeyPair::generate().unwrap())
    }
}

/// The key a PKCS#8 PrivateKeyInfo wraps (its privateKey OCTET STRING): a
/// SEC1 ECPrivateKey for an EC key, a PKCS#1 RSAPrivateKey for an RSA key.
fn unwrap_pkcs8(der: &[u8]) -> Vec<u8> {
    fn tlv(input: &[u8]) -> (u8, &[u8], &[u8]) {
        let (len, header) = match input[1] {
            n if n < 0x80 => (usize::from(n), 2),
            0x81 => (usize::from(input[2]), 3),
            0x82 => ((usize::from(input[2]) << 8) | usize::from(input[3]), 4),
            other => panic!("unexpected DER length form {other:#x}"),
        };
        (
            input[0],
            &input[header..header + len],
            &input[header + len..],
        )
    }
    let (_, info, _) = tlv(der);
    let (_, _, rest) = tlv(info); // version
    let (_, _, rest) = tlv(rest); // privateKeyAlgorithm
    let (tag, key, _) = tlv(rest);
    assert_eq!(tag, 0x04, "privateKey is an OCTET STRING");
    key.to_vec()
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}

fn crlf(pem: &str) -> String {
    pem.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn node(ca: &str, chain: &str, key: &str) -> Result<ScNodeTlsConfig, ScTlsConfigError> {
    ScNodeTlsConfig::from_pem(ca.as_bytes(), chain.as_bytes(), key.as_bytes())
}

fn hub(ca: &str, chain: &str, key: &str) -> Result<ScHubTlsConfig, ScTlsConfigError> {
    ScHubTlsConfig::from_pem(ca.as_bytes(), chain.as_bytes(), key.as_bytes())
}

/// Both configurations' verdict on the same PEM, with the node's error.
fn both(ca: &str, chain: &str, key: &str) -> Result<(), ScTlsConfigError> {
    let node = node(ca, chain, key).map(drop);
    let hub = hub(ca, chain, key).map(drop);
    assert_eq!(
        node.as_ref().map_err(ToString::to_string),
        hub.as_ref().map_err(ToString::to_string),
        "the node and hub configurations disagree"
    );
    node
}

/// A directory of files, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sc-tls-pem-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
    let pkcs1 = text(pem(
        "RSA PRIVATE KEY",
        &unwrap_pkcs8(&rsa.key.serialize_der()),
    ));
    for (form, leaf, key) in [
        ("PKCS#8 P-256", &ec, ec.key.serialize_pem()),
        ("SEC1 P-256", &ec, sec1),
        ("PKCS#8 RSA", &rsa, rsa.key.serialize_pem()),
        ("PKCS#1 RSA", &rsa, pkcs1),
        ("PKCS#8 Ed25519", &ed, ed.key.serialize_pem()),
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
    // A bundle of several CAs, the one that issued last.
    let other = Site::new();
    both(&format!("{}{}", other.root, site.root), &leaf.chain, &key).unwrap();
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
            [
                ca.into(),
                chain.into(),
                "-----BEGIN PRIVATE KEY-----\n!\n-----END PRIVATE KEY-----\n".into(),
            ],
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

#[test]
fn files_are_read_ca_first_and_an_unreadable_one_names_its_path() {
    let site = Site::new();
    let leaf = site.leaf();
    let dir = Dir::new("files");
    let ca = dir.write("site ca é.pem", &crlf(&site.root));
    let chain = dir.write("node.pem", &leaf.chain);
    let key = dir.write("node.key", &leaf.key.serialize_pem());
    ScNodeTlsConfig::from_pem_files(&ca, &chain, &key).unwrap();
    ScHubTlsConfig::from_pem_files(&ca, &chain, &key).unwrap();
    let missing = dir.0.join("missing.pem");
    let read = |error: ScTlsConfigError, input: ScTlsInput, path: &Path| match error {
        ScTlsConfigError::Read {
            input: i,
            path: p,
            source,
            ..
        } => {
            assert_eq!((i, p.as_path()), (input, path));
            source
        }
        other => panic!("expected a read error, got {other:?}"),
    };
    for (paths, input) in [
        ([&missing, &chain, &key], ScTlsInput::CaBundle),
        ([&ca, &missing, &key], ScTlsInput::CertificateChain),
        ([&ca, &chain, &missing], ScTlsInput::PrivateKey),
    ] {
        let error = ScNodeTlsConfig::from_pem_files(paths[0], paths[1], paths[2]).unwrap_err();
        assert!(error.to_string().contains("missing.pem"), "{error}");
        let source = read(error, input, &missing);
        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        let error = ScHubTlsConfig::from_pem_files(paths[0], paths[1], paths[2]).unwrap_err();
        read(error, input, &missing);
    }
    // A directory is not a readable file, and the CA bundle is checked
    // before the identity files are opened at all.
    let error = ScNodeTlsConfig::from_pem_files(&dir.0, &missing, &missing).unwrap_err();
    read(error, ScTlsInput::CaBundle, &dir.0);
    let bad = dir.write(
        "bad.pem",
        "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
    );
    let error = ScNodeTlsConfig::from_pem_files(&bad, &missing, &missing).unwrap_err();
    assert!(
        matches!(
            error,
            ScTlsConfigError::UnusableCertificate {
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
    let key = leaf.key.serialize_pem();
    let body: String = key.lines().nth(1).unwrap().chars().take(24).collect();
    let one_line = key.replace('\n', "");
    let wrong_leaf = site.leaf();
    let cases = [
        (site.root.clone(), leaf.chain.clone(), one_line.clone()),
        (site.root.clone(), leaf.chain.clone(), format!("{key}{key}")),
        (
            site.root.clone(),
            leaf.chain.clone(),
            key.replace("-----END", "-----BEGIN"),
        ),
        (
            site.root.clone(),
            leaf.chain.clone(),
            key.replacen('\n', "!\n", 2),
        ),
        (site.root.clone(), wrong_leaf.chain.clone(), key.clone()),
        (key.clone(), leaf.chain.clone(), key.clone()),
        (one_line.clone(), leaf.chain.clone(), key.clone()),
        (site.root.clone(), one_line.clone(), key.clone()),
        (
            site.root.clone(),
            leaf.chain.clone(),
            key.replace("PRIVATE KEY", "ENCRYPTED PRIVATE KEY"),
        ),
    ];
    for (ca, chain, key_text) in cases {
        let error = both(&ca, &chain, &key_text).unwrap_err();
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

/// A hub and a node built from PEM files, each with an intermediate in its
/// chain, complete TLS 1.3 mutual authentication and the BACnet/SC connect.
#[tokio::test]
async fn hub_and_node_from_pem_files_connect_through_intermediates() {
    let site = Site::new();
    let dir = Dir::new("connect");
    let ca = dir.write("ca.pem", &site.root);
    let files = |name: &str| {
        let leaf = site.leaf();
        (
            dir.write(&format!("{name}.pem"), &crlf(&leaf.chain)),
            dir.write(&format!("{name}.key"), &leaf.key.serialize_pem()),
        )
    };
    let (hub_chain, hub_key) = files("hub");
    let (node_chain, node_key) = files("node");
    let hub_tls = ScHubTlsConfig::from_pem_files(&ca, &hub_chain, &hub_key).unwrap();
    let node_tls = ScNodeTlsConfig::from_pem_files(&ca, &node_chain, &node_key).unwrap();
    let mut hub = ScHub::start("127.0.0.1:0", hub_tls, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    let url = format!("wss://localhost:{}", hub.local_addr().unwrap().port());
    let connect = async {
        let ws = TlsWebSocket::connect(&url, node_tls).await.unwrap();
        assert_eq!(
            ws.peer_info().certificates().len(),
            2,
            "the hub sent its intermediate"
        );
        let mut transport = ScTransport::new(ws, [0x22; 6]).with_device_uuid([0x22; 16]);
        let _rx = transport.start().await.unwrap();
        assert!(transport.hub_peer_info().await.is_some());
        transport.stop().await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(10), connect)
        .await
        .expect("the PEM-built node connects to the PEM-built hub");
    hub.stop().await;
}

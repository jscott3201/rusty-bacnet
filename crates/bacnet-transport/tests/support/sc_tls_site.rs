//! Shared fixtures of the SC TLS PEM constructor tests (#2600): a CA with
//! an intermediate and its leaves, key re-encodings, and a temporary
//! directory.

use std::path::PathBuf;

use bacnet_transport::sc_hub::ScHubTlsConfig;
use bacnet_transport::sc_tls::{ScNodeTlsConfig, ScTlsConfigError};
use rcgen::{CertificateParams, Issuer, KeyPair};

/// A root CA, an intermediate it issued, and leaves the intermediate issued.
pub struct Site {
    pub root: String,
    pub intermediate: String,
    pub issuer: Issuer<'static, KeyPair>,
}

/// A leaf's certificate PEM, its chain (leaf then intermediate) and its key.
pub struct Leaf {
    pub chain: String,
    pub key: KeyPair,
}

impl Site {
    pub fn new() -> Self {
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

    pub fn leaf_with(&self, key: KeyPair) -> Leaf {
        let params = CertificateParams::new(vec!["localhost".into()]).unwrap();
        let cert = params.signed_by(&key, &self.issuer).unwrap().pem();
        Leaf {
            chain: format!("{cert}{}", self.intermediate),
            key,
        }
    }

    pub fn leaf(&self) -> Leaf {
        self.leaf_with(KeyPair::generate().unwrap())
    }
}

/// The key a PKCS#8 PrivateKeyInfo wraps (its privateKey OCTET STRING): a
/// SEC1 ECPrivateKey for an EC key, a PKCS#1 RSAPrivateKey for an RSA key.
pub fn unwrap_pkcs8(der: &[u8]) -> Vec<u8> {
    pub fn tlv(input: &[u8]) -> (u8, &[u8], &[u8]) {
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

pub fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}

/// `pem` with LF line ends. rcgen writes CRLF on Windows and LF elsewhere,
/// so a test that edits lines starts from this to mean the same everywhere.
pub fn lf(pem: &str) -> String {
    pem.replace("\r\n", "\n")
}

pub fn crlf(pem: &str) -> String {
    lf(pem).replace('\n', "\r\n")
}

/// `pem` with lone CR line ends, as old Mac files have them.
pub fn cr(pem: &str) -> String {
    lf(pem).replace('\n', "\r")
}

pub fn node(ca: &str, chain: &str, key: &str) -> Result<ScNodeTlsConfig, ScTlsConfigError> {
    ScNodeTlsConfig::from_pem(ca.as_bytes(), chain.as_bytes(), key.as_bytes())
}

pub fn hub(ca: &str, chain: &str, key: &str) -> Result<ScHubTlsConfig, ScTlsConfigError> {
    ScHubTlsConfig::from_pem(ca.as_bytes(), chain.as_bytes(), key.as_bytes())
}

/// Both configurations' verdict on the same PEM, with the node's error.
pub fn both(ca: &str, chain: &str, key: &str) -> Result<(), ScTlsConfigError> {
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
pub struct Dir(pub PathBuf);

impl Dir {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sc-tls-pem-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn write(&self, name: &str, contents: &str) -> PathBuf {
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

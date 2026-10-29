//! Independent file fixtures and literal process-output assertions.
use super::support::{Files, Process, command};
use rcgen::{CertificateParams, KeyPair};
use serde_json::Value;
use std::{
    path::PathBuf,
    process::{Command, Output},
};

pub const NOW: i64 = 1_704_067_200;
pub const MALFORMED_DER: &str = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n";
pub const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

pub fn params(days: u64) -> CertificateParams {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.not_before = rcgen::date_time_ymd(2024, 1, 1);
    p.not_after = p.not_before + std::time::Duration::from_secs(days * 86400);
    p.distinguished_name
        .push(rcgen::DnType::CommonName, "CLI offline fixture");
    p
}

pub struct Fixture {
    pub files: Files,
    pub cert: PathBuf,
    pub key: PathBuf,
    pub pem: String,
    pub key_pem: String,
}
impl Fixture {
    pub fn new(params: CertificateParams) -> Self {
        let pair = KeyPair::generate().unwrap();
        let pem = params.self_signed(&pair).unwrap().pem();
        let key_pem = pair.serialize_pem();
        let files = Files::new();
        let cert = files.write("identity ü with spaces.pem", &pem);
        let key = files.write("private ü with spaces.key", &key_pem);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self {
            files,
            cert,
            key,
            pem,
            key_pem,
        }
    }
    pub fn command(&self, now: i64) -> Command {
        let mut cmd = command();
        cmd.arg("sc-check")
            .arg("--sc-cert")
            .arg(&self.cert)
            .arg("--at-unix")
            .arg(now.to_string());
        cmd
    }
    pub async fn run(&self, now: i64) -> Output {
        Process::spawn(&mut self.command(now)).output().await
    }
}

pub fn document(output: &Output, status: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(status),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "JSON execution emitted stderr");
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    assert!(!output.stdout.contains(&0x1b));
    serde_json::from_slice(&output.stdout).expect("one complete JSON document")
}
pub fn has(document: &Value, role: &str, index: Option<usize>, code: &str, severity: &str) -> bool {
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| {
            entry["role"] == role
                && entry["index"].as_u64() == index.map(|v| v as u64)
                && entry["code"] == code
                && entry["severity"] == severity
        })
}

pub fn tlv(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut bytes = vec![tag];
    if contents.len() < 128 {
        bytes.push(contents.len() as u8);
    } else {
        bytes.extend([0x82, (contents.len() >> 8) as u8, contents.len() as u8]);
    }
    bytes.extend(contents);
    bytes
}

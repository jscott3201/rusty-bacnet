//! Private file-loading boundary shared only by the two standalone binaries.

use bacnet_transport::sc_tls::{ScTlsConfigError, ScTlsInput};

pub(super) fn required<'a>(value: Option<&'a str>, flag: &str) -> Result<&'a str, String> {
    value.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        format!("{flag} is required and must be nonempty; see examples/docker/README.md")
    })
}

/// The three PEM files as read, for the TLS configurations' `from_pem`, so
/// the binaries decode them by the same rules as every other SC user (one
/// key, PEM well formed throughout). They read the files themselves rather
/// than through `from_pem_files` so that no diagnostic echoes a path.
pub(super) struct Credentials {
    pub ca: Vec<u8>,
    pub chain: Vec<u8>,
    pub key: Vec<u8>,
    flags: [&'static str; 3],
}

impl Credentials {
    pub fn load(ca: &str, cert: &str, key: &str, flags: [&'static str; 3]) -> Result<Self, String> {
        Ok(Self {
            ca: read(ca, flags[0])?,
            chain: read(cert, flags[1])?,
            key: read(key, flags[2])?,
            flags,
        })
    }

    /// A configuration error, led by the flag of the file at fault.
    pub fn refused(&self, error: ScTlsConfigError) -> String {
        let flag = match error.input() {
            Some(ScTlsInput::CaBundle) => self.flags[0],
            Some(ScTlsInput::CertificateChain) => self.flags[1],
            Some(ScTlsInput::PrivateKey) => self.flags[2],
            _ => "SC TLS configuration",
        };
        format!("{flag}: {error}")
    }
}

fn read(path: &str, flag: &str) -> Result<Vec<u8>, String> {
    // Do not echo supplied contents (or a malicious path) in diagnostics.
    std::fs::read(path).map_err(|e| format!("{flag}: cannot read PEM file ({})", e.kind()))
}

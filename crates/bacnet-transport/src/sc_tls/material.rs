//! The checked TLS material both SC configurations are built from (#2600):
//! the PEM decoding, the file reads, and the DER checks every constructor
//! makes before rustls assembles a configuration.
//!
//! Inputs are checked in a fixed order: the CA bundle, then the chain, then
//! the key. From PEM, each CA certificate is checked as it is decoded, so the
//! first fault in the bundle is the one reported; the identity files are read
//! only after the whole bundle passed. The chain and key are both decoded
//! (an empty chain, no key or two keys are decoding faults) before either is
//! checked as DER.
//!
//! The key and leaf are checked by rustls's own [`CertifiedKey::from_der`],
//! and the configurations are built by rustls's own builders from the same
//! chain and key; this module only maps rustls's verdict to
//! [`ScTlsConfigError`].
//!
//! Private-key bytes (#2621): the key file's text and every decoded key are
//! held in [`Zeroizing`] wrappers, so they are cleared on every exit,
//! error paths included. rustls's key loader clears the copy of the DER it
//! is handed. What this module cannot clear is listed on the constructors'
//! rustdoc.
//!
//! On a rustls or aws-lc upgrade (last checked: rustls 0.23.46, aws-lc-sys
//! 0.45.0), recheck that `CertifiedKey::from_der` still refuses only with
//! the errors `identity_refused` (and the inspection's `classify_match`)
//! sort; that the providers' `load_private_key` still wraps the DER it is
//! handed in `Zeroizing`; that aws-lc's `OPENSSL_free` still clears memory
//! before freeing it; and that the tests `key_verdicts_are_the_ones_rustls_reaches`
//! and `every_key_type_the_pinned_provider_loads_names_its_public_key` pass.

use std::{fs::File, io::Read, path::Path, sync::Arc};

use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::{PemObject, SectionKind};
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
};
use rustls::sign::CertifiedKey;
use rustls::{InconsistentKeys, RootCertStore};
use zeroize::Zeroizing;

use super::config_error::{ScCertificateForm, ScKeyForm, ScPemFault, ScTlsConfigError, ScTlsInput};

/// The largest file the `from_pem_files` constructors read, in bytes:
/// 1 MiB, several times a public CA bundle of every root. Reading stops at
/// the limit, so a stream that never ends stops there and is refused. The
/// read can still block: opening a FIFO that no writer has opened waits for
/// one, and a writer that stops writing stalls the read.
pub const MAX_PEM_FILE_BYTES: usize = 1024 * 1024;

/// Material every check passed: explicit trust, a leaf-first chain rustls
/// can use, the leaf's own key, and the fixed provider to build with.
pub(crate) struct Material {
    /// The CA bundle as trust anchors; nonempty.
    pub(crate) roots: Arc<RootCertStore>,
    /// This end's chain, leaf first; nonempty.
    pub(crate) chain: Vec<CertificateDer<'static>>,
    /// The leaf's private key, cleared when it drops. The builders take
    /// copies of it ([`PrivateKeyDer::clone_key`]); rustls clears those.
    pub(crate) key: Zeroizing<PrivateKeyDer<'static>>,
    /// The built-in aws-lc provider, fixed so that a caller's or
    /// process-wide key provider can't weaken the certificate and key
    /// consistency check.
    pub(crate) provider: Arc<CryptoProvider>,
}

impl Material {
    /// Owned DER, as `from_der` takes it.
    pub(crate) fn from_der(
        ca_certs: Vec<CertificateDer<'static>>,
        cert_chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, ScTlsConfigError> {
        let key = Zeroizing::new(key);
        let mut roots = RootCertStore::empty();
        for (index, cert) in ca_certs.into_iter().enumerate() {
            trust(&mut roots, index, cert)?;
        }
        Self::identity(nonempty(roots)?, cert_chain, key)
    }

    /// PEM text, as `from_pem` takes it.
    pub(crate) fn from_pem(
        ca_bundle: &[u8],
        cert_chain: &[u8],
        private_key: &[u8],
    ) -> Result<Self, ScTlsConfigError> {
        let roots = trust_pem(ca_bundle)?;
        let chain = certificates(cert_chain)?;
        Self::identity(roots, chain, single_key(private_key)?)
    }

    /// PEM files, as `from_pem_files` takes them: each read whole, once.
    pub(crate) fn from_pem_files(
        ca_bundle: &Path,
        cert_chain: &Path,
        private_key: &Path,
    ) -> Result<Self, ScTlsConfigError> {
        let roots = trust_pem(&read(ca_bundle, ScTlsInput::CaBundle)?)?;
        let chain = certificates(&read(cert_chain, ScTlsInput::CertificateChain)?)?;
        // The key file's text is cleared when this statement ends.
        let key = single_key(&read(private_key, ScTlsInput::PrivateKey)?)?;
        Self::identity(roots, chain, key)
    }

    /// Check the chain's DER and that the key loads and is the leaf's.
    fn identity(
        roots: RootCertStore,
        chain: Vec<CertificateDer<'static>>,
        key: Zeroizing<PrivateKeyDer<'static>>,
    ) -> Result<Self, ScTlsConfigError> {
        if chain.is_empty() {
            return Err(ScTlsConfigError::NoCertificate {
                input: ScTlsInput::CertificateChain,
            });
        }
        for (index, cert) in chain.iter().enumerate() {
            rustls::server::ParsedCertificate::try_from(cert).map_err(|source| {
                ScTlsConfigError::UnusableCertificate {
                    input: ScTlsInput::CertificateChain,
                    index,
                    source,
                }
            })?;
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        // The same check, by the same function, that the config builders
        // (`with_client_auth_cert`, `with_single_cert`) make; run here so
        // that its refusal comes out typed. The loaded key is dropped.
        CertifiedKey::from_der(chain.clone(), key.clone_key(), &provider)
            .map_err(identity_refused)?;
        Ok(Self {
            roots: Arc::new(roots),
            chain,
            key,
            provider,
        })
    }
}

/// [`CertifiedKey::from_der`]'s refusal of the leaf and key as ours: a
/// key that is not the leaf's is [`ScTlsConfigError::KeyMismatch`]; a
/// verdict on the certificate, or on the pair other than a mismatch, is
/// rustls's own ([`ScTlsConfigError::Tls`]; the chain was checked first, so
/// none is expected); anything else is the key provider's refusal to load
/// the key ([`ScTlsConfigError::UnusableKey`]).
fn identity_refused(source: rustls::Error) -> ScTlsConfigError {
    use rustls::Error as E;
    match source {
        E::InconsistentKeys(InconsistentKeys::KeyMismatch) => ScTlsConfigError::KeyMismatch,
        E::InconsistentKeys(_) | E::InvalidCertificate(_) | E::NoCertificatesPresented => {
            ScTlsConfigError::Tls { source }
        }
        source => ScTlsConfigError::UnusableKey { source },
    }
}

/// rustls's refusal of an assembled configuration.
pub(crate) fn refused(source: rustls::Error) -> ScTlsConfigError {
    ScTlsConfigError::Tls { source }
}

/// A verifier builder's refusal. With a nonempty bundle and no revocation
/// lists neither of its causes can arise; mapped rather than unwrapped.
pub(crate) fn verifier_refused(error: rustls::client::VerifierBuilderError) -> ScTlsConfigError {
    use rustls::client::VerifierBuilderError as E;
    match error {
        E::NoRootAnchors => ScTlsConfigError::NoCertificate {
            input: ScTlsInput::CaBundle,
        },
        E::InvalidCrl(crl) => refused(crl.into()),
        other => refused(rustls::Error::General(other.to_string())),
    }
}

/// Read `source` up to `limit` bytes and one byte more, into a buffer that
/// is cleared when it drops, and never into one that is freed holding the
/// text. The buffer starts at `hint + 1` bytes (the file's length, 0 when
/// unknown, as for a FIFO or a growing file); when it fills it is replaced
/// by a larger [`Zeroizing`] one the text is copied into, and the old one
/// clears itself. A result longer than `limit` means the input was too long.
pub(crate) fn read_limited(
    mut source: impl Read,
    hint: usize,
    limit: usize,
) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let max = limit.saturating_add(1);
    let mut cap = hint.saturating_add(1).clamp(1, max);
    let mut buf = Zeroizing::new(vec![0u8; cap]);
    let mut len = 0;
    loop {
        if len == cap {
            if cap == max {
                break;
            }
            cap = cap.saturating_mul(2).clamp(1, max);
            let mut grown = Zeroizing::new(vec![0u8; cap]);
            grown[..len].copy_from_slice(&buf[..len]);
            buf = grown;
        }
        match source.read(&mut buf[len..cap]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    buf.truncate(len);
    Ok(buf)
}

/// The file at `path`, read up to [`MAX_PEM_FILE_BYTES`] and one byte more
/// (see [`read_limited`]); the text is cleared when the result drops.
fn read(path: &Path, input: ScTlsInput) -> Result<Zeroizing<Vec<u8>>, ScTlsConfigError> {
    let failed = |source| ScTlsConfigError::Read {
        input,
        path: path.to_path_buf(),
        source,
    };
    let file = File::open(path).map_err(failed)?;
    let hint = file
        .metadata()
        .map_or(0, |meta| meta.len().min(MAX_PEM_FILE_BYTES as u64) as usize);
    let bytes = read_limited(file, hint, MAX_PEM_FILE_BYTES).map_err(failed)?;
    if bytes.len() > MAX_PEM_FILE_BYTES {
        return Err(ScTlsConfigError::TooLarge {
            input,
            path: path.to_path_buf(),
            limit: MAX_PEM_FILE_BYTES,
        });
    }
    Ok(bytes)
}

fn malformed(input: ScTlsInput, error: &rustls::pki_types::pem::Error) -> ScTlsConfigError {
    ScTlsConfigError::MalformedPem {
        input,
        fault: ScPemFault::of(error),
    }
}

/// Add CA certificate `index` as a trust anchor.
fn trust(
    roots: &mut RootCertStore,
    index: usize,
    cert: CertificateDer<'static>,
) -> Result<(), ScTlsConfigError> {
    roots
        .add(cert)
        .map_err(|source| ScTlsConfigError::UnusableCertificate {
            input: ScTlsInput::CaBundle,
            index,
            source,
        })
}

fn nonempty(roots: RootCertStore) -> Result<RootCertStore, ScTlsConfigError> {
    if roots.is_empty() {
        return Err(ScTlsConfigError::NoCertificate {
            input: ScTlsInput::CaBundle,
        });
    }
    Ok(roots)
}

/// Why `input`'s PEM held no certificate: only certificates in a container
/// this stack doesn't read, or none at all.
fn no_certificate(pem: &[u8], input: ScTlsInput) -> ScTlsConfigError {
    let form = labels(pem).find_map(|label| match label {
        b"TRUSTED CERTIFICATE" => Some(ScCertificateForm::Trusted),
        b"PKCS7" | b"CMS" => Some(ScCertificateForm::Pkcs7),
        _ => None,
    });
    match form {
        Some(form) => ScTlsConfigError::UnsupportedCertificate { input, form },
        None => ScTlsConfigError::NoCertificate { input },
    }
}

/// The bundle's certificates as trust anchors, each checked as it is
/// decoded: a later block is not decoded before an earlier one passed.
fn trust_pem(pem: &[u8]) -> Result<RootCertStore, ScTlsConfigError> {
    let mut roots = RootCertStore::empty();
    for (index, cert) in CertificateDer::pem_slice_iter(pem).enumerate() {
        let cert = cert.map_err(|error| malformed(ScTlsInput::CaBundle, &error))?;
        trust(&mut roots, index, cert)?;
    }
    if roots.is_empty() {
        return Err(no_certificate(pem, ScTlsInput::CaBundle));
    }
    Ok(roots)
}

/// The chain's certificates, in order and at least one; sections of other
/// kinds are skipped.
fn certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, ScTlsConfigError> {
    let chain: Vec<_> = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<_, _>>()
        .map_err(|error| malformed(ScTlsInput::CertificateChain, &error))?;
    if chain.is_empty() {
        return Err(no_certificate(pem, ScTlsInput::CertificateChain));
    }
    Ok(chain)
}

/// The one private key in `pem`, the rule both the TLS configurations and
/// the offline inspection apply. Every section is decoded, so a second key
/// or a malformed section anywhere is refused; certificates and parameters
/// beside the key are skipped.
///
/// Every decoded key (the first, extra ones, and those an error discards)
/// is cleared when it drops.
pub(crate) fn single_key(
    pem: &[u8],
) -> Result<Zeroizing<PrivateKeyDer<'static>>, ScTlsConfigError> {
    let mut first: Option<Zeroizing<PrivateKeyDer<'static>>> = None;
    let mut readable = 0usize;
    for section in <(SectionKind, Vec<u8>)>::pem_slice_iter(pem) {
        let (kind, der) = section.map_err(|error| {
            // OpenSSL's older encrypted keys keep the SEC1 or PKCS#1 label
            // and put RFC 1421 headers before the body, which then isn't
            // base64. Beside another key it is still a second key.
            let fault = ScPemFault::of(&error);
            if fault == ScPemFault::Base64 && legacy_encrypted(pem) {
                let count = labels(pem)
                    .filter(|label| label.ends_with(b"PRIVATE KEY"))
                    .count();
                return match count {
                    0 | 1 => ScTlsConfigError::UnsupportedKey {
                        form: ScKeyForm::Encrypted,
                    },
                    count => ScTlsConfigError::MultiplePrivateKeys { count },
                };
            }
            ScTlsConfigError::MalformedPem {
                input: ScTlsInput::PrivateKey,
                fault,
            }
        })?;
        // Cleared at the end of this turn unless moved into a key below.
        let mut der = Zeroizing::new(der);
        let key = match kind {
            SectionKind::PrivateKey => {
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(std::mem::take(&mut *der)))
            }
            SectionKind::EcPrivateKey => {
                PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(std::mem::take(&mut *der)))
            }
            SectionKind::RsaPrivateKey => {
                PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(std::mem::take(&mut *der)))
            }
            _ => continue,
        };
        readable += 1;
        first.get_or_insert(Zeroizing::new(key));
    }
    let mut unreadable = unreadable_keys(pem);
    let unreadable_first = unreadable.next();
    let count = readable + usize::from(unreadable_first.is_some()) + unreadable.count();
    match (count, first, unreadable_first) {
        (0, ..) => Err(ScTlsConfigError::NoPrivateKey),
        (1, Some(key), _) => Ok(key),
        (1, None, Some(form)) => Err(ScTlsConfigError::UnsupportedKey { form }),
        (count, ..) => Err(ScTlsConfigError::MultiplePrivateKeys { count }),
    }
}

/// The private key sections the PEM parser skips, by the label of their
/// `BEGIN` line.
fn unreadable_keys(pem: &[u8]) -> impl Iterator<Item = ScKeyForm> + '_ {
    labels(pem).filter_map(|label| match label {
        b"PRIVATE KEY" | b"EC PRIVATE KEY" | b"RSA PRIVATE KEY" => None,
        b"ENCRYPTED PRIVATE KEY" => Some(ScKeyForm::Encrypted),
        b"OPENSSH PRIVATE KEY" => Some(ScKeyForm::OpenSsh),
        _ if label.ends_with(b"PRIVATE KEY") => Some(ScKeyForm::Other),
        _ => None,
    })
}

/// Whether a SEC1 or PKCS#1 key section starts with a `Proc-Type` header
/// that marks it encrypted (RFC 1421): the header must be the first
/// non-empty line after that section's `BEGIN` line, so the same text in
/// leading prose or in another kind of section doesn't count.
fn legacy_encrypted(pem: &[u8]) -> bool {
    let mut lines = lines(pem).filter(|line| !line.is_empty()).peekable();
    while let Some(line) = lines.next() {
        let key_begin = label(line)
            .is_some_and(|label| matches!(label, b"EC PRIVATE KEY" | b"RSA PRIVATE KEY"));
        if key_begin
            && lines.peek().is_some_and(|next| {
                next.strip_prefix(b"Proc-Type:")
                    .is_some_and(|value| value.windows(9).any(|w| w == b"ENCRYPTED"))
            })
        {
            return true;
        }
    }
    false
}

/// The labels of the `BEGIN` lines, as the PEM parser reads them. These
/// scans read `BEGIN` and header lines only, never a section's body, and
/// keep nothing of what they read.
fn labels(pem: &[u8]) -> impl Iterator<Item = &[u8]> {
    lines(pem).filter_map(label)
}

/// The label of a `BEGIN` line, if `line` is one.
fn label(line: &[u8]) -> Option<&[u8]> {
    line.strip_prefix(b"-----BEGIN ")?
        .trim_ascii_end()
        .strip_suffix(b"-----")
}

fn lines(pem: &[u8]) -> impl Iterator<Item = &[u8]> {
    pem.split(|byte| matches!(byte, b'\n' | b'\r'))
}

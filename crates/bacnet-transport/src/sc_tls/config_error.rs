//! Why a BACnet/SC TLS configuration could not be built from the material it
//! was given (#2600): the one error of [`super::ScNodeTlsConfig`]'s and
//! [`crate::sc_hub::ScHubTlsConfig`]'s constructors, from DER, PEM bytes or
//! PEM files alike.

use std::{fmt, io, path::PathBuf};

/// Which of the three inputs a configuration error is about.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScTlsInput {
    /// The site CA certificates this end trusts.
    CaBundle,
    /// This end's own certificate chain, leaf first.
    CertificateChain,
    /// The private key of the chain's leaf.
    PrivateKey,
}

impl fmt::Display for ScTlsInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CaBundle => "CA bundle",
            Self::CertificateChain => "certificate chain",
            Self::PrivateKey => "private key",
        })
    }
}

/// How an input fails to be PEM. Carries none of the input's text: a key
/// file's lines can hold key material.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScPemFault {
    /// A `BEGIN` line with no matching `END` line after it.
    MissingEnd,
    /// A `BEGIN` line that is not well formed.
    MalformedBegin,
    /// A section whose body is not base64.
    Base64,
    /// A section larger than the PEM parser takes (256 MiB).
    TooLarge,
    /// Any other fault the PEM parser reports.
    Other,
}

impl fmt::Display for ScPemFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingEnd => "a BEGIN line has no matching END line",
            Self::MalformedBegin => "a BEGIN line is malformed",
            Self::Base64 => "a section's body is not base64",
            Self::TooLarge => "a section is too large",
            Self::Other => "the PEM parser refused it",
        })
    }
}

impl ScPemFault {
    /// The fault a parser error names, without the text it may quote.
    pub(crate) fn of(error: &rustls::pki_types::pem::Error) -> Self {
        use rustls::pki_types::pem::Error;
        match error {
            Error::MissingSectionEnd { .. } => Self::MissingEnd,
            Error::IllegalSectionStart { .. } => Self::MalformedBegin,
            Error::Base64Decode(_) => Self::Base64,
            Error::SectionTooLarge => Self::TooLarge,
            _ => Self::Other,
        }
    }
}

/// A private key in a PEM form this stack can't read. The forms it reads are
/// an unencrypted PKCS#8, SEC1 (EC) or PKCS#1 (RSA) key.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScKeyForm {
    /// An encrypted key: an encrypted PKCS#8 key (`ENCRYPTED PRIVATE KEY`),
    /// or OpenSSL's older encrypted SEC1 or PKCS#1 key, whose section starts
    /// with a `Proc-Type: 4,ENCRYPTED` header. Decrypt it to unencrypted
    /// PKCS#8 first: `openssl pkcs8 -topk8 -nocrypt -in key.pem -out
    /// plain.pem` reads both forms and asks for the passphrase.
    Encrypted,
    /// An OpenSSH key (`OPENSSH PRIVATE KEY`). Convert it to unencrypted
    /// PKCS#8 PEM first: `ssh-keygen -p -m PKCS8 -f key` rewrites an RSA or
    /// ECDSA key in place; OpenSSH can't write an Ed25519 key as PKCS#8, so
    /// such a key needs another tool or a new key.
    OpenSsh,
    /// Another key form, such as a DSA key.
    Other,
}

impl fmt::Display for ScKeyForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Encrypted => {
                "encrypted; decrypt it to unencrypted PKCS#8 first \
                 (openssl pkcs8 -topk8 -nocrypt -in <key> -out <plain key>)"
            }
            Self::OpenSsh => "an OpenSSH key; convert it to unencrypted PKCS#8 PEM first",
            Self::Other => "a key form other than PKCS#8, SEC1 or PKCS#1",
        })
    }
}

/// A certificate container this stack can't read as a CA bundle or chain.
/// The form it reads is plain `CERTIFICATE` sections.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScCertificateForm {
    /// OpenSSL's `TRUSTED CERTIFICATE`, a certificate with trust settings
    /// appended. `openssl x509 -in ca.pem -out plain.pem` writes the plain
    /// certificate.
    Trusted,
    /// A PKCS#7 or CMS bundle (`PKCS7`, `CMS`).
    /// `openssl pkcs7 -print_certs -in bundle.p7b -out certs.pem` lists its
    /// certificates as plain PEM.
    Pkcs7,
}

impl fmt::Display for ScCertificateForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Trusted => {
                "OpenSSL TRUSTED CERTIFICATE sections; write them as plain certificates \
                 (openssl x509 -in <file> -out <plain file>)"
            }
            Self::Pkcs7 => {
                "a PKCS#7 bundle; list its certificates as plain PEM \
                 (openssl pkcs7 -print_certs -in <file> -out <plain file>)"
            }
        })
    }
}

/// Why an SC TLS configuration could not be built.
///
/// Every variant names the input at fault ([`ScTlsConfigError::input`]).
/// None carries key bytes or a key file's text, in `Debug` or `Display`.
/// The constructors check the CA bundle first, then the certificate chain,
/// then the private key; from PEM, both identity inputs are decoded before
/// either is checked as DER, so a file that can't be read or decoded is
/// reported before a certificate or key that decodes but can't be used.
#[non_exhaustive]
#[derive(Debug)]
pub enum ScTlsConfigError {
    /// A file could not be read.
    #[non_exhaustive]
    Read {
        /// The input the file holds.
        input: ScTlsInput,
        /// The file, as given.
        path: PathBuf,
        /// The operating system's error.
        source: io::Error,
    },
    /// A file is larger than [`MAX_PEM_FILE_BYTES`](super::MAX_PEM_FILE_BYTES).
    /// Reading stopped at the limit, so a stream that never ends stops there
    /// and is refused this way too (a read can still block, as on a FIFO no
    /// writer has opened).
    #[non_exhaustive]
    TooLarge {
        /// The input the file holds.
        input: ScTlsInput,
        /// The file, as given.
        path: PathBuf,
        /// The limit, in bytes.
        limit: usize,
    },
    /// An input is not well-formed PEM.
    #[non_exhaustive]
    MalformedPem {
        /// The input at fault.
        input: ScTlsInput,
        /// How it fails.
        fault: ScPemFault,
    },
    /// The CA bundle or the certificate chain holds no certificate. For the
    /// CA bundle this is an empty bundle: nothing would be trusted.
    #[non_exhaustive]
    NoCertificate {
        /// The input at fault.
        input: ScTlsInput,
    },
    /// The CA bundle or the certificate chain holds no plain certificate,
    /// only certificates in a container this stack doesn't read.
    #[non_exhaustive]
    UnsupportedCertificate {
        /// The input at fault.
        input: ScTlsInput,
        /// The container they are in.
        form: ScCertificateForm,
    },
    /// A certificate decoded but rustls can't use it, as a trust anchor
    /// for the CA bundle or as an end-entity or intermediate certificate for
    /// the chain.
    #[non_exhaustive]
    UnusableCertificate {
        /// The input at fault.
        input: ScTlsInput,
        /// The certificate's position in that input, from 0.
        index: usize,
        /// rustls's reason.
        source: rustls::Error,
    },
    /// The private key input holds no PKCS#8, SEC1 or PKCS#1 private key.
    NoPrivateKey,
    /// The private key input holds more than one private key, so which one
    /// is meant is unknown.
    #[non_exhaustive]
    MultiplePrivateKeys {
        /// How many it holds.
        count: usize,
    },
    /// The private key is in a form this stack can't read.
    #[non_exhaustive]
    UnsupportedKey {
        /// The form it is in.
        form: ScKeyForm,
    },
    /// The private key decoded, but the TLS provider can't load it: its
    /// algorithm or curve isn't one the provider signs with, or its DER is
    /// malformed.
    #[non_exhaustive]
    UnusableKey {
        /// rustls's reason.
        source: rustls::Error,
    },
    /// The private key does not belong to the chain's leaf certificate.
    KeyMismatch,
    /// rustls refused the assembled configuration for another reason.
    #[non_exhaustive]
    Tls {
        /// rustls's reason.
        source: rustls::Error,
    },
}

impl ScTlsConfigError {
    /// The input at fault: the private key for a key that doesn't match the
    /// leaf, and `None` only for [`ScTlsConfigError::Tls`].
    pub fn input(&self) -> Option<ScTlsInput> {
        match self {
            Self::Read { input, .. }
            | Self::TooLarge { input, .. }
            | Self::MalformedPem { input, .. }
            | Self::NoCertificate { input }
            | Self::UnsupportedCertificate { input, .. }
            | Self::UnusableCertificate { input, .. } => Some(*input),
            Self::NoPrivateKey
            | Self::MultiplePrivateKeys { .. }
            | Self::UnsupportedKey { .. }
            | Self::UnusableKey { .. }
            | Self::KeyMismatch => Some(ScTlsInput::PrivateKey),
            Self::Tls { .. } => None,
        }
    }
}

impl fmt::Display for ScTlsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read {
                input,
                path,
                source,
            } => write!(f, "can't read the {input} file {}: {source}", Shown(path)),
            Self::TooLarge { input, path, limit } => write!(
                f,
                "the {input} file {} is larger than {limit} bytes",
                Shown(path)
            ),
            Self::MalformedPem { input, fault } => {
                write!(f, "the {input} is not valid PEM: {fault}")
            }
            Self::NoCertificate {
                input: ScTlsInput::CaBundle,
            } => f.write_str("the CA bundle holds no certificate, so nothing would be trusted"),
            Self::NoCertificate { input } => write!(f, "the {input} holds no certificate"),
            Self::UnsupportedCertificate { input, form } => {
                write!(f, "the {input} holds no plain certificate, only {form}")
            }
            Self::UnusableCertificate {
                input,
                index,
                source,
            } => write!(
                f,
                "certificate {index} of the {input} can't be used: {source}"
            ),
            Self::NoPrivateKey => f.write_str("no PKCS#8, SEC1 or PKCS#1 private key found"),
            Self::MultiplePrivateKeys { count } => {
                write!(
                    f,
                    "the private key input holds {count} keys; it must hold one"
                )
            }
            Self::UnsupportedKey { form } => write!(f, "the private key is {form}"),
            Self::UnusableKey { source } => {
                write!(f, "the TLS provider can't load the private key: {source}")
            }
            Self::KeyMismatch => {
                f.write_str("the private key doesn't belong to the chain's leaf certificate")
            }
            Self::Tls { source } => write!(f, "TLS configuration refused: {source}"),
        }
    }
}

/// A path as text, with control characters, bidirectional formatting
/// characters (U+202A to U+202E, U+2066 to U+2069) and the line and
/// paragraph separators (U+2028, U+2029) escaped: a file name can't move the
/// cursor, start a new line or reorder the text of a message.
struct Shown<'a>(&'a std::path::Path);

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        for c in self.0.display().to_string().chars() {
            let hidden = matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                || matches!(c, '\u{2028}' | '\u{2029}');
            if c.is_control() || hidden {
                write!(f, "{}", c.escape_default())?;
            } else {
                f.write_char(c)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ScTlsConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::UnusableCertificate { source, .. }
            | Self::UnusableKey { source }
            | Self::Tls { source } => Some(source),
            _ => None,
        }
    }
}

impl From<ScTlsConfigError> for bacnet_types::error::Error {
    /// The error as [`InvalidConfig`](bacnet_types::error::Error::InvalidConfig)
    /// with its text, for code that reports one error type for the whole
    /// stack. Match on [`ScTlsConfigError`] itself to tell the cases apart.
    fn from(error: ScTlsConfigError) -> Self {
        Self::InvalidConfig(bacnet_types::error::Refusal::text(error.to_string()))
    }
}

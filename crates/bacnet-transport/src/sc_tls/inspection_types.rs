//! Owned values for offline inspection; Debug intentionally omits text.

use std::{fmt, io};

use crate::sc_tls::ScCertificateMetadata;

macro_rules! vocabulary {
    ($(#[$attr:meta])* $name:ident { $($(#[$vattr:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$attr])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($(#[$vattr])* #[doc = $text] $variant),+ }
        impl $name {
            /// Stable spelling used by the Python report.
            pub fn as_str(self) -> &'static str { match self { $(Self::$variant => $text),+ } }
        }
    };
}

vocabulary! {
    /// The purpose assigned by the caller, without an issuer-authorization claim.
    ScInputRole { Identity => "identity", PeerTrust => "peer_trust", PrivateKey => "private_key" }
}
vocabulary! {
    /// Safe categories; no original PEM error or input bytes are retained.
    ScInputErrorKind {
        InputTooLarge => "input_too_large", CertificateTooLarge => "certificate_too_large",
        TooManyCertificates => "too_many_certificates", EmptyCertificates => "empty_certificates",
        MalformedPem => "malformed_pem", MissingPrivateKey => "missing_private_key",
        ClockOutOfRange => "clock_out_of_range"
    }
}
vocabulary! {
    /// Relationship of inspection time to the certificate's stated interval.
    ScValidity {
        Unavailable => "unavailable", InvalidInterval => "invalid_interval",
        NotYetValid => "not_yet_valid", Expired => "expired", WithinInterval => "within_interval"
    }
}
vocabulary! {
    /// Expiry threshold, assessed only while within the stated validity interval.
    ScExpiry { NotAssessed => "not_assessed", None => "none", Warning => "warning", Alarm => "alarm" }
}
vocabulary! {
    /// Local SPKI comparison, independent of chain validation and remote acceptance.
    ScKeyConsistency {
        NotAssessed => "not_assessed", Match => "match", Mismatch => "mismatch",
        Unusable => "unusable", Unknown => "unknown"
    }
}
vocabulary! {
    /// Whether a CA in the peer-trust input signed the identity leaf itself,
    /// with no intermediate between them (the Annex AB.7.4 default check).
    ScDirectIssuer {
        NotAssessed => "not_assessed", Direct => "direct", NotDirect => "not_direct",
        Unknown => "unknown"
    }
}
vocabulary! {
    /// The component to which a finding applies.
    ScFindingArea {
        Certificate => "certificate", San => "san", KeyUsage => "key_usage",
        ExtendedKeyUsage => "extended_key_usage"
    }
}
vocabulary! {
    /// Informational interpretation defects; none changes TLS acceptance.
    ScFindingKind {
        Malformed => "malformed", Duplicate => "duplicate", Unsupported => "unsupported",
        Truncated => "truncated"
    }
}

/// PEM/I/O errors stop inspection. DER and policy observations remain in reports.
#[derive(Debug)]
pub enum ScInspectionError {
    /// Bounded input category and zero-based certificate index, where applicable.
    Input {
        /// Input purpose supplied by the caller.
        role: ScInputRole,
        /// Zero-based certificate position, if applicable.
        index: Option<usize>,
        /// Sanitized category without original input or PEM error.
        kind: ScInputErrorKind,
    },
    /// Native file error, preserving errno without storing a supplied path.
    Io {
        /// Input purpose supplied by the caller.
        role: ScInputRole,
        /// Native file error, including errno where available.
        source: io::Error,
    },
}

impl fmt::Display for ScInspectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input { role, index, kind } => {
                write!(f, "SC inspection {}", role.as_str())?;
                if let Some(index) = index {
                    write!(f, "[{index}]")?;
                }
                write!(f, ": {}", kind.as_str())
            }
            Self::Io { role, source } => write!(f, "SC inspection {}: {source}", role.as_str()),
        }
    }
}
impl std::error::Error for ScInspectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// A bounded typed observation. Certificate role/index belongs to its entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScInspectionFinding {
    /// Interpreted certificate component.
    pub area: ScFindingArea,
    /// Interpretation limitation or defect.
    pub kind: ScFindingKind,
}

/// One interpreted extension. Absence is distinct from present but unparsed.
/// Related malformed/duplicate/unsupported/truncated findings are on the entry.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ScExtensionInspection {
    /// False when DER or interpretation bounds prevent establishing presence.
    pub assessed: bool,
    /// The extension was encountered, even if its value could not be interpreted.
    pub present: bool,
    /// Bounded escaped values; inspect findings before interpreting an empty list.
    pub values: Vec<String>,
}
impl fmt::Debug for ScExtensionInspection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScExtensionInspection")
            .field("assessed", &self.assessed)
            .field("present", &self.present)
            .field("value_count", &self.values.len())
            .finish()
    }
}

/// One original DER fingerprint with optional interpretation. No DER is retained.
#[derive(Clone, PartialEq, Eq)]
pub struct ScInspectedCertificate {
    /// Local identity chain or peer-validation trust bundle.
    pub role: ScInputRole,
    /// Original zero-based position within this input role.
    pub index: usize,
    /// SHA-256 over the exact decoded DER, including any malformed content.
    pub sha256: [u8; 32],
    /// Generic bounded metadata, absent on certificate parse failure.
    pub metadata: Option<ScCertificateMetadata>,
    /// Stated interval compared with the report's exact inspection time.
    pub validity: ScValidity,
    /// Seven/thirty-day threshold while within the interval.
    pub expiry: ScExpiry,
    /// Subject alternative names; this does not match any intended peer name.
    pub san: ScExtensionInspection,
    /// Interpreted key-usage bits, without usage-policy enforcement.
    pub key_usage: ScExtensionInspection,
    /// Extended purposes, with client/server authentication listed separately.
    pub extended_key_usage: ScExtensionInspection,
    /// Deduplicated component findings, with no trust/acceptance verdict.
    pub findings: Vec<ScInspectionFinding>,
}
impl fmt::Debug for ScInspectedCertificate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScInspectedCertificate")
            .field("role", &self.role)
            .field("index", &self.index)
            .field("validity", &self.validity)
            .field("expiry", &self.expiry)
            .field("findings", &self.findings)
            .finish_non_exhaustive()
    }
}

/// Opened key-file Unix mode-bit observations, without an ACL/security verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScKeyFilePermissions {
    /// Group-read mode bit observed on the opened file descriptor.
    pub group_readable: bool,
    /// Other-read mode bit observed on the opened file descriptor.
    pub world_readable: bool,
}

/// Owned offline report. Contains no keys, original PEM, TLS policy or resources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScPemInspection {
    /// One sampled or explicitly supplied time in signed Unix seconds.
    pub inspected_at_unix: i64,
    /// Whether peer-trust input was provided, independent of identity issuers.
    pub peer_trust_supplied: bool,
    /// Identity entries followed by trust entries, preserving per-role order.
    pub certificates: Vec<ScInspectedCertificate>,
    /// Proven SPKI equality, discrepancy, unusable key or unavailable comparison.
    pub key_consistency: ScKeyConsistency,
    /// None for bytes, omitted key, and platforms without Unix mode bits.
    pub key_file_permissions: Option<ScKeyFilePermissions>,
    /// Whether a peer-trust certificate signed the identity leaf directly.
    /// `NotAssessed` when no peer-trust input was supplied; `Unknown` when the
    /// leaf could not be parsed or a candidate CA's signature algorithm is
    /// unsupported by the verifier.
    pub direct_issuer: ScDirectIssuer,
    /// Zero-based peer-trust index of the CA that signed the leaf, when
    /// `direct_issuer` is `Direct`.
    pub direct_issuer_index: Option<usize>,
}

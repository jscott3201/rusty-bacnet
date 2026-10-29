//! Subject Alternative Names read from a certificate (#1965).

use std::fmt;
use std::net::IpAddr;

use x509_parser::extensions::{GeneralName, SubjectAlternativeName};
use x509_parser::oid_registry::OID_X509_EXT_SUBJECT_ALT_NAME;
use x509_parser::prelude::{FromDer, X509Certificate};

/// One Subject Alternative Name of a certificate, as written in it.
///
/// Names keep the certificate's exact text, unescaped, for comparing
/// against configured values; nothing here checks their syntax. Compare for
/// exact equality, byte for byte: DNS case is not folded, a wildcard such as
/// `*.example.com` is returned as written and never expanded, names may hold
/// UTF-8 outside the IA5 range, and an IP name compares as an [`IpAddr`], so
/// an IPv4-mapped IPv6 name is not equal to the IPv4 one. Don't match by
/// suffix. `Debug` shows only the kind, never the name.
#[derive(Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScSubjectAltName {
    /// A DNS name (dNSName).
    Dns(String),
    /// A URI (uniformResourceIdentifier).
    Uri(String),
    /// An email address (rfc822Name).
    Email(String),
    /// An IPv4 or IPv6 address (iPAddress).
    Ip(IpAddr),
}

impl fmt::Debug for ScSubjectAltName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dns(_) => "Dns(..)",
            Self::Uri(_) => "Uri(..)",
            Self::Email(_) => "Email(..)",
            Self::Ip(_) => "Ip(..)",
        })
    }
}

/// The DNS, URI, email and IP names of `cert`'s Subject Alternative Name
/// extension, in certificate order. Other name forms, entries that don't
/// decode, and a certificate whose extension is absent, repeated or
/// malformed give no names.
pub(super) fn subject_alt_names(cert: &X509Certificate<'_>) -> Vec<ScSubjectAltName> {
    let mut extensions = cert
        .extensions()
        .iter()
        .filter(|extension| extension.oid == OID_X509_EXT_SUBJECT_ALT_NAME);
    let (Some(extension), None) = (extensions.next(), extensions.next()) else {
        return Vec::new();
    };
    let Ok(([], names)) = SubjectAlternativeName::from_der(extension.value) else {
        return Vec::new();
    };
    names
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::DNSName(dns) => Some(ScSubjectAltName::Dns((*dns).to_owned())),
            GeneralName::URI(uri) => Some(ScSubjectAltName::Uri((*uri).to_owned())),
            GeneralName::RFC822Name(email) => Some(ScSubjectAltName::Email((*email).to_owned())),
            GeneralName::IPAddress(octets) => match octets.len() {
                4 => <[u8; 4]>::try_from(*octets).ok().map(IpAddr::from),
                16 => <[u8; 16]>::try_from(*octets).ok().map(IpAddr::from),
                _ => None,
            }
            .map(ScSubjectAltName::Ip),
            _ => None,
        })
        .collect()
}

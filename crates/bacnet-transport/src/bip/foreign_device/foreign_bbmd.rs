//! Where a configured foreign device's BBMD is: an address, or a host name
//! with a port (#2308).

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::str::FromStr;

use bacnet_types::error::Error;

use super::super::DEFAULT_BACNET_PORT;
use crate::bbmd_lookup::BbmdName;

/// The BBMD a foreign device registers with
/// (the `bbmd` of a [`ForeignDeviceConfig`](super::ForeignDeviceConfig)).
///
/// A host name is looked up as one written to FD_BBMD_Address is
/// ([`BipTransport::set_bbmd_resolver`](crate::bip::BipTransport::set_bbmd_resolver),
/// #1852): at start, then alongside the registration attempts at most once
/// every 30 seconds. Until its first lookup succeeds the transport runs
/// with no BBMD: foreign registration waits (the registration snapshot's
/// `waiting_for_bbmd`), broadcasts are held and then refused, and a failed
/// lookup is counted in that snapshot
/// (`bbmd_lookups`, `bbmd_lookup_failures`, `last_bbmd_lookup_failure`)
/// and tried again every 30 seconds.
///
/// It parses from `ip`, `ip:port`, `host` or `host:port`, the port 47808
/// when none is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignBbmd {
    /// The BBMD's B/IP address.
    Address(SocketAddrV4),
    /// A host name, in the RFC 1123 form, and the BBMD's UDP port.
    Name {
        /// The host name.
        host: String,
        /// The UDP port.
        port: u16,
    },
}

impl ForeignBbmd {
    /// The address, unless the BBMD is known by name.
    #[must_use]
    pub fn address(&self) -> Option<SocketAddrV4> {
        match self {
            Self::Address(address) => Some(*address),
            Self::Name { .. } => None,
        }
    }

    /// The UDP port.
    #[must_use]
    pub fn port(&self) -> u16 {
        match self {
            Self::Address(address) => address.port(),
            Self::Name { port, .. } => *port,
        }
    }

    /// The host name as a lookup takes it, when it is one in the RFC 1123
    /// form with a nonzero port.
    pub(in crate::bip) fn name(&self) -> Option<BbmdName> {
        match self {
            Self::Address(_) => None,
            Self::Name { host, port } => BbmdName::new(host, *port),
        }
    }
}

impl From<SocketAddrV4> for ForeignBbmd {
    fn from(address: SocketAddrV4) -> Self {
        Self::Address(address)
    }
}

impl From<(Ipv4Addr, u16)> for ForeignBbmd {
    fn from((ip, port): (Ipv4Addr, u16)) -> Self {
        Self::Address(SocketAddrV4::new(ip, port))
    }
}

impl fmt::Display for ForeignBbmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(address) => address.fmt(f),
            Self::Name { host, port } => write!(f, "{host}:{port}"),
        }
    }
}

impl FromStr for ForeignBbmd {
    type Err = Error;

    /// `ip`, `ip:port`, `host` or `host:port`, the port 47808 when none is
    /// given. A host name takes the RFC 1123 form, and one of digits and
    /// dots alone must be an IPv4 address. Port 0 and the unspecified
    /// address are refused: no device can register with them.
    fn from_str(text: &str) -> Result<Self, Error> {
        let refused = || {
            Error::Encoding(format!(
                "'{text}' is not a BBMD address, host name or either with :port"
            ))
        };
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().map_err(|_| refused())?),
            None => (text, DEFAULT_BACNET_PORT),
        };
        if port == 0 {
            return Err(refused());
        }
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            if ip.is_unspecified() {
                return Err(refused());
            }
            return Ok(Self::Address(SocketAddrV4::new(ip, port)));
        }
        let numeric = host
            .bytes()
            .all(|octet| octet.is_ascii_digit() || octet == b'.');
        if numeric || BbmdName::new(host, port).is_none() {
            return Err(refused());
        }
        Ok(Self::Name {
            host: host.to_owned(),
            port,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use super::ForeignBbmd;

    #[test]
    fn addresses_and_host_names_parse_with_or_without_a_port() {
        let ip = Ipv4Addr::new(192, 0, 2, 10);
        let parse = |text: &str| text.parse::<ForeignBbmd>().ok();
        assert_eq!(
            parse("192.0.2.10"),
            Some(SocketAddrV4::new(ip, 47808).into())
        );
        assert_eq!(parse("192.0.2.10:47809"), Some((ip, 47809).into()));
        let named = |host: &str, port| ForeignBbmd::Name {
            host: host.into(),
            port,
        };
        assert_eq!(parse("bbmd.example"), Some(named("bbmd.example", 47808)));
        assert_eq!(parse("bbmd:4700"), Some(named("bbmd", 4700)));
        for bad in [
            "",
            "0.0.0.0",
            "192.0.2.10:0",
            "bbmd:0",
            "bbmd:port",
            "999.1.1.1",
            "bb md",
            "-bbmd",
            "::1",
            "[::1]:47808",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        let display = |text: &str| parse(text).unwrap().to_string();
        assert_eq!(display("bbmd.example"), "bbmd.example:47808");
        assert_eq!(display("192.0.2.10"), "192.0.2.10:47808");
    }

    /// A start refuses a host name it couldn't look up, or port 0, before
    /// any I/O; a good name passes, its address checked once it resolves.
    #[test]
    fn a_start_checks_a_names_form() {
        let check = |host: &str, port| {
            let config = super::super::ForeignDeviceConfig {
                bbmd: ForeignBbmd::Name {
                    host: host.into(),
                    port,
                },
                ttl: 60,
                renewal_interval: None,
            };
            config.check_bbmd(Ipv4Addr::BROADCAST, Ipv4Addr::UNSPECIFIED)
        };
        assert!(check("bbmd.example", 47808).is_ok());
        assert!(check("bb md", 47808).is_err());
        assert!(check("bbmd.example", 0).is_err());
    }

    #[test]
    fn only_an_address_has_one() {
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47808);
        assert_eq!(ForeignBbmd::from(address).address(), Some(address));
        let named = ForeignBbmd::Name {
            host: "bbmd".into(),
            port: 47809,
        };
        assert_eq!((named.address(), named.port()), (None, 47809));
    }
}

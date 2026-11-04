//! Where a configured B/IPv6 foreign device's BBMD is: an address, or a
//! host name with a port (#2365), as B/IP's
//! [`ForeignBbmd`](crate::bip::ForeignBbmd) is.

use std::fmt;
use std::net::{Ipv6Addr, SocketAddrV6};
use std::str::FromStr;

use bacnet_types::error::Error;

use super::port::DEFAULT_BACNET6_PORT;
use crate::bbmd_lookup::BbmdName;

/// The BBMD a B/IPv6 foreign device registers with
/// (the `bbmd` of a [`Bip6ForeignDeviceConfig`](super::Bip6ForeignDeviceConfig)).
///
/// A host name is looked up as one written to FD_BBMD_Address is
/// ([`Bip6Transport::set_bbmd_resolver`](super::Bip6Transport::set_bbmd_resolver)):
/// at start, then alongside the registrations at most once every 30
/// seconds, and the device registers with the first IPv6 address it stands
/// for that it can use. A transport bound to `::` picks its source address
/// from the route to the BBMD, so its start looks the name up first and
/// fails if it doesn't resolve. On an explicit interface the start goes
/// ahead and the device waits for the name: nothing is registered,
/// broadcasts are refused, and the name is tried again 30 seconds after
/// each failure.
///
/// It parses from `ip`, `[ip]`, `[ip]:port`, `host` or `host:port`, the
/// port 47808 when none is given; a port after an IPv6 address needs the
/// brackets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bip6ForeignBbmd {
    /// The BBMD's B/IPv6 address.
    Address(SocketAddrV6),
    /// A host name, in the RFC 1123 form, and the BBMD's UDP port.
    Name {
        /// The host name.
        host: String,
        /// The UDP port.
        port: u16,
    },
}

impl Bip6ForeignBbmd {
    /// The address, unless the BBMD is known by name.
    #[must_use]
    pub fn address(&self) -> Option<SocketAddrV6> {
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
    pub(super) fn name(&self) -> Option<BbmdName> {
        match self {
            Self::Address(_) => None,
            Self::Name { host, port } => BbmdName::new(host, *port),
        }
    }
}

impl From<SocketAddrV6> for Bip6ForeignBbmd {
    fn from(address: SocketAddrV6) -> Self {
        Self::Address(address)
    }
}

impl From<(Ipv6Addr, u16)> for Bip6ForeignBbmd {
    fn from((ip, port): (Ipv6Addr, u16)) -> Self {
        Self::Address(SocketAddrV6::new(ip, port, 0, 0))
    }
}

impl fmt::Display for Bip6ForeignBbmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(address) => write!(f, "[{}]:{}", address.ip(), address.port()),
            Self::Name { host, port } => write!(f, "{host}:{port}"),
        }
    }
}

impl FromStr for Bip6ForeignBbmd {
    type Err = Error;

    /// `ip`, `[ip]`, `[ip]:port`, `host` or `host:port`, the port 47808
    /// when none is given. A host name takes the RFC 1123 form. Port 0, a
    /// zone index and a flow label are refused: none travels with a B/IPv6
    /// address. Whether an address can be registered with is
    /// [`Bip6ForeignDeviceConfig::check`](super::Bip6ForeignDeviceConfig::check)'s
    /// to say.
    fn from_str(text: &str) -> Result<Self, Error> {
        let refused = || {
            Error::Encoding(format!(
                "'{text}' is not a B/IPv6 BBMD address ('ip', '[ip]' or '[ip]:port'), \
                 host name or host name with :port"
            ))
        };
        if let Ok(address) = text.parse::<SocketAddrV6>() {
            if address.port() == 0 || address.scope_id() != 0 || address.flowinfo() != 0 {
                return Err(refused());
            }
            return Ok(Self::Address(address));
        }
        let bare = text
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or(text);
        if let Ok(ip) = bare.parse::<Ipv6Addr>() {
            return Ok((ip, DEFAULT_BACNET6_PORT).into());
        }
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().map_err(|_| refused())?),
            None => (text, DEFAULT_BACNET6_PORT),
        };
        // Digits and dots alone would be an IPv4 address, which B/IP serves.
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
    use std::net::{Ipv6Addr, SocketAddrV6};

    use super::Bip6ForeignBbmd;

    #[test]
    fn addresses_and_host_names_parse_with_or_without_a_port() {
        let ip: Ipv6Addr = "2001:db8::10".parse().unwrap();
        let parse = |text: &str| text.parse::<Bip6ForeignBbmd>().ok();
        let at = |port| Some(Bip6ForeignBbmd::Address(SocketAddrV6::new(ip, port, 0, 0)));
        assert_eq!(parse("2001:db8::10"), at(47808));
        assert_eq!(parse("[2001:db8::10]"), at(47808));
        assert_eq!(parse("[2001:db8::10]:47809"), at(47809));
        // No brackets: the last group is part of the address, not a port.
        let whole = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0x10, 0x4780);
        assert_eq!(
            parse("2001:db8::10:4780"),
            Some(Bip6ForeignBbmd::Address(SocketAddrV6::new(
                whole, 47808, 0, 0
            )))
        );
        assert_eq!(
            parse("2001:db8::10:4780").unwrap().to_string(),
            "[2001:db8::10:4780]:47808"
        );
        let name = |host: &str, port| {
            Some(Bip6ForeignBbmd::Name {
                host: host.into(),
                port,
            })
        };
        assert_eq!(parse("bbmd.example"), name("bbmd.example", 47808));
        assert_eq!(parse("bbmd.example:47809"), name("bbmd.example", 47809));
        for refused in [
            "",
            "[2001:db8::10]:0",
            "[fe80::1%2]:47808",
            "bbmd.example:0",
            "bbmd example",
            "192.0.2.1",
            "192.0.2.1:47808",
            "bbmd.example:port",
        ] {
            assert_eq!(parse(refused), None, "{refused}");
        }
        for text in ["[2001:db8::10]:47809", "bbmd.example:47809"] {
            assert_eq!(parse(text).unwrap().to_string(), text);
        }
    }
}

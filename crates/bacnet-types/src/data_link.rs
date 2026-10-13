//! The BACnet data links a transport can carry, as named in errors.

use core::fmt;

/// A BACnet data link.
///
/// Errors use it to say which data link an operation needs and which one an
/// endpoint actually has, such as
/// [`Error::UnsupportedTransport`](crate::error::Error::UnsupportedTransport).
/// `Display` gives the short name a person would expect in a message.
///
/// ```
/// use bacnet_types::data_link::DataLink;
///
/// assert_eq!(DataLink::Bip.to_string(), "BACnet/IP");
/// assert_eq!(DataLink::Mstp.to_string(), "MS/TP");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataLink {
    /// BACnet/IP over UDP (Annex J).
    Bip,
    /// BACnet/IPv6 over UDP (Annex U).
    Bip6,
    /// MS/TP over RS-485 (Clause 9).
    Mstp,
    /// BACnet Secure Connect over TLS WebSockets (Annex AB).
    Sc,
    /// BACnet over Ethernet LLC frames (Clause 7).
    Ethernet,
    /// An in-process loopback transport, not a wire data link.
    Loopback,
}

impl DataLink {
    /// The short name `Display` writes.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bip => "BACnet/IP",
            Self::Bip6 => "BACnet/IPv6",
            Self::Mstp => "MS/TP",
            Self::Sc => "BACnet/SC",
            Self::Ethernet => "BACnet Ethernet",
            Self::Loopback => "loopback",
        }
    }
}

impl fmt::Display for DataLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_data_link_displays_its_short_name() {
        let names = [
            (DataLink::Bip, "BACnet/IP"),
            (DataLink::Bip6, "BACnet/IPv6"),
            (DataLink::Mstp, "MS/TP"),
            (DataLink::Sc, "BACnet/SC"),
            (DataLink::Ethernet, "BACnet Ethernet"),
            (DataLink::Loopback, "loopback"),
        ];
        for (link, name) in names {
            assert_eq!(link.name(), name);
            assert_eq!(link.to_string(), name);
        }
    }
}

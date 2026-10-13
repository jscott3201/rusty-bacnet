//! Messages between the UI loop and the worker.
//!
//! The UI sends [`Command`]s; the worker answers with [`WorkerEvent`]s. Both are
//! plain data with no transport type parameter, so `App` and the views stay
//! non-generic and only the worker is instantiated per transport.

use std::net::{Ipv4Addr, SocketAddrV6};
use std::time::{Duration, Instant};

use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_types::enums::Segmentation;

/// Identifies one user-started operation, so late results can be dropped.
pub(crate) type OpId = u64;

/// Highest valid device instance (the 22-bit object instance field).
pub(crate) const MAX_INSTANCE: u32 = 4_194_303;

/// Where a Who-Is goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WhoIsScope {
    /// Broadcast on the local network only.
    Local,
    /// Global broadcast: every reachable network.
    Global,
    /// Unicast to one address.
    Directed {
        /// The destination.
        to: DirectedTo,
        /// The address as the user typed it, for display.
        label: String,
    },
    /// Broadcast on one remote network.
    Network(u16),
}

/// The destination of a directed Who-Is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectedTo {
    /// A MAC on the local network.
    Mac(Vec<u8>),
    /// A B/IPv6 address. The worker finds the VMAC of the node there before
    /// sending, since a B/IPv6 node is addressed by its VMAC (Clause U.1.3).
    Bip6(SocketAddrV6),
}

/// A validated Who-Is request from the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WhoIsSpec {
    /// Destination.
    pub(crate) scope: WhoIsScope,
    /// Inclusive instance range; `None` asks every device.
    pub(crate) range: Option<DeviceInstanceRange>,
    /// How long to show the request as running while replies arrive.
    pub(crate) listen: Duration,
}

impl WhoIsSpec {
    /// True when the request has no effective instance limit.
    pub(crate) fn is_full_range(&self) -> bool {
        self.range
            .is_none_or(|range| (range.low(), range.high()) == (0, MAX_INSTANCE))
    }

    /// This request with a directed B/IPv6 destination replaced by the VMAC
    /// found for it; the label the user typed stays.
    pub(crate) fn with_vmac(mut self, vmac: Vec<u8>) -> Self {
        if let WhoIsScope::Directed { to, .. } = &mut self.scope {
            *to = DirectedTo::Mac(vmac);
        }
        self
    }

    /// True for a global broadcast.
    pub(crate) fn is_global(&self) -> bool {
        self.scope == WhoIsScope::Global
    }

    /// True when `instance` is one the request asks to answer.
    pub(crate) fn wants(&self, instance: u32) -> bool {
        self.range.is_none_or(|range| range.contains(instance))
    }

    /// Short description, such as `local 101-103` or `network 5, all`.
    pub(crate) fn describe(&self) -> String {
        let scope = match &self.scope {
            WhoIsScope::Local => "local".to_string(),
            WhoIsScope::Global => "global".to_string(),
            WhoIsScope::Directed { label, .. } => format!("to {label}"),
            WhoIsScope::Network(dnet) => format!("network {dnet}"),
        };
        match self.range {
            Some(range) if range.low() == range.high() => format!("{scope} {}", range.low()),
            Some(range) if !self.is_full_range() => {
                format!("{scope} {}-{}", range.low(), range.high())
            }
            _ => format!("{scope}, all"),
        }
    }
}

/// UI to worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// Bind BACnet/IP to this interface (the answer to the interface picker).
    Connect {
        /// Interface address.
        interface: Ipv4Addr,
        /// Its broadcast address.
        broadcast: Ipv4Addr,
    },
    /// Send a Who-Is and report when its listen window ends.
    WhoIs {
        /// Operation id.
        op: OpId,
        /// The request.
        spec: WhoIsSpec,
    },
    /// Stop the running operation.
    Cancel {
        /// Operation id.
        op: OpId,
    },
}

/// One row of the device table, already formatted for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceRow {
    /// Device instance.
    pub(crate) instance: u32,
    /// Where it answered from: its own address, or for a routed device its
    /// remote MAC and the router.
    pub(crate) address: String,
    /// Remote network number for a routed device.
    pub(crate) network: Option<u16>,
    /// Vendor identifier.
    pub(crate) vendor_id: u16,
    /// Max APDU length accepted.
    pub(crate) max_apdu: u32,
    /// Segmentation support.
    pub(crate) segmentation: Segmentation,
    /// When the last I-Am arrived.
    pub(crate) last_seen: Instant,
}

/// How an operation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpOutcome {
    /// The listen window ran out.
    Completed,
    /// Cancelled by the user or replaced by a newer request.
    Cancelled,
    /// Sending failed.
    Failed(String),
}

/// Worker to UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerEvent {
    /// The client is up; `local` is its own address.
    Connected {
        /// Local address for the status bar.
        local: String,
    },
    /// Building the client failed. The TUI exits and prints the error.
    ConnectFailed {
        /// The error.
        error: String,
    },
    /// A new device answered.
    Discovered(DeviceRow),
    /// A known device answered again.
    Updated(DeviceRow),
    /// The client purged a stale device.
    Lost(DeviceRow),
    /// Two endpoints claim the same instance; the client kept `retained`.
    Collision {
        /// The row the client's table kept.
        retained: DeviceRow,
        /// The conflicting answer.
        incoming: DeviceRow,
    },
    /// The client's whole table, sent after events were dropped.
    Snapshot(Vec<DeviceRow>),
    /// The Who-Is left; its listen window has started.
    WhoIsSent {
        /// Operation id.
        op: OpId,
    },
    /// An operation ended.
    OpFinished {
        /// Operation id.
        op: OpId,
        /// How.
        outcome: OpOutcome,
    },
}

/// How a transport's MAC addresses are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressStyle {
    /// 4-byte IPv4 address and 2-byte port.
    Bip,
    /// 3-byte B/IPv6 VMAC, as `bip6:0x0004D2` (Clause U.1.3).
    Bip6,
    /// Hex bytes (BACnet/SC VMACs and anything else).
    Hex,
}

impl AddressStyle {
    /// Format a MAC for display, in a form the directed Who-Is reads back.
    pub(crate) fn format(self, mac: &[u8]) -> String {
        match (self, mac.len()) {
            (Self::Bip, 6) | (Self::Bip6, 3) => crate::output::format_mac(mac),
            _ => hex(mac),
        }
    }

    /// The addresses a directed Who-Is takes on this transport.
    pub(crate) fn directed_forms(self) -> &'static str {
        match self {
            Self::Bip => "IP or IP:port",
            Self::Bip6 => "bip6:N, bip6:0xVMAC or [IPv6]:port",
            Self::Hex => "VMAC as 12 hex digits",
        }
    }
}

/// Colon-separated lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(scope: WhoIsScope, range: Option<(u32, u32)>) -> WhoIsSpec {
        WhoIsSpec {
            scope,
            range: range.map(|(low, high)| DeviceInstanceRange::new(low, high).unwrap()),
            listen: Duration::from_secs(3),
        }
    }

    #[test]
    fn full_range_and_wants_follow_the_limits() {
        assert!(spec(WhoIsScope::Local, None).is_full_range());
        assert!(spec(WhoIsScope::Local, Some((0, MAX_INSTANCE))).is_full_range());
        let bounded = spec(WhoIsScope::Network(5), Some((101, 103)));
        assert!(!bounded.is_full_range());
        assert!(bounded.wants(101) && bounded.wants(103));
        assert!(!bounded.wants(100) && !bounded.wants(104));
        assert_eq!(bounded.describe(), "network 5 101-103");
        assert_eq!(spec(WhoIsScope::Global, None).describe(), "global, all");
    }

    #[test]
    fn addresses_format_per_transport() {
        assert_eq!(
            AddressStyle::Bip.format(&[10, 0, 0, 1, 0xBA, 0xC0]),
            "10.0.0.1:47808"
        );
        // A B/IPv6 MAC is the node's VMAC (Clause U.1.3).
        assert_eq!(AddressStyle::Bip6.format(&[0, 4, 0xD2]), "bip6:0x0004D2");
        assert_eq!(
            AddressStyle::Bip6.format(&[2, 0, 0, 0, 0, 1]),
            "02:00:00:00:00:01"
        );
        assert_eq!(
            AddressStyle::Hex.format(&[2, 0, 0, 0, 0, 1]),
            "02:00:00:00:00:01"
        );
    }
}

//! Messages between the UI loop and the worker.
//!
//! The UI sends [`Command`]s; the worker answers with [`WorkerEvent`]s. Both are
//! plain data with no transport type parameter, so `App` and the views stay
//! non-generic and only the worker is instantiated per transport.

use std::net::{Ipv4Addr, SocketAddrV6};
use std::time::{Duration, Instant};

use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_types::enums::Segmentation;

use super::point::{BrowseRead, BrowseResult, ReqId};

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
    /// Load, save or start a named B/IP profile.
    Profile(super::profile::service::Request),
    /// Start the most recently selected flags-only configuration again.
    Reconnect,
    /// Retire the current connection and wait for resource release.
    Disconnect,
    /// Bind BACnet/IP to this interface (the answer to the interface picker).
    Connect {
        /// Interface address.
        interface: Ipv4Addr,
        /// Its broadcast address.
        broadcast: Ipv4Addr,
    },
    /// Join the MS/TP trunk with these settings, on the port chosen: the
    /// operator confirmed it (T2). From a monitor session, the monitor ends
    /// first.
    JoinTrunk {
        /// The settings, port included.
        settings: crate::mstp::MstpArgs,
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
    /// Read for the device browser.
    Browse {
        /// Request id, echoed in the results.
        req: ReqId,
        /// Device instance, from the discovery table.
        device: u32,
        /// What to read.
        read: BrowseRead,
    },
    /// Object names for the rows on screen, every device's in one command
    /// so a tick never fills the queue (#1896).
    Names {
        /// Request id, echoed in a refusal.
        req: ReqId,
        /// Each device whose ask changed.
        asks: Vec<super::point::NameAsk>,
    },
    /// Stop reading `device`: drop its queued reads and end the running one
    /// after its outstanding request.
    StopBrowse {
        /// Device instance.
        device: u32,
    },
    /// A watch list request.
    Watch(super::watched::WatchCommand),
    /// A write session request.
    Safety(super::safety::SafetyCommand),
    /// A Network screen request.
    Network(super::network::NetworkCommand),
    /// A Who-Has or router discovery request.
    Discovery(super::network::discovery::DiscoveryCommand),
    /// A capture request for the session.
    Capture(super::capture::CaptureCommand),
    /// Write the ring to pcap files; the coordinator does it, on a
    /// blocking thread.
    CaptureExport(Box<super::capture::ExportJob>),
}

/// One row of the device table, already formatted for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceRow {
    /// Device instance.
    pub(crate) instance: u32,
    /// Where it is, as a target the CLI reads back: its own address, or
    /// for a routed device its routed station, `2:0x05`.
    pub(crate) address: String,
    /// The router a routed device answered through, on the local network.
    pub(crate) via: Option<String>,
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

impl DeviceRow {
    /// The address with the router a routed device answered through,
    /// `2:0x05 via 10.0.1.1:47808`, for the log and the duplicate banner.
    pub(crate) fn located(&self) -> String {
        match &self.via {
            Some(router) => format!("{} via {router}", self.address),
            None => self.address.clone(),
        }
    }
}

/// Why a connection failed (#1894): a short summary the footer shows with
/// the recovery keys, and the whole error for the device screen and the
/// log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    /// A few words, such as `address in use`.
    pub(crate) summary: String,
    /// The whole error.
    pub(crate) detail: String,
}

impl Failure {
    /// From the stack's typed error: an I/O failure by its kind, a timeout
    /// as such, anything else as its message.
    pub(crate) fn of(error: &bacnet_types::error::Error) -> Self {
        use bacnet_types::error::Error;
        let summary = match error {
            Error::Transport(io) => io_summary(io),
            Error::Timeout(_) => "timed out".to_string(),
            other => other.to_string(),
        };
        Self {
            summary,
            detail: error.to_string(),
        }
    }

    /// `error` with `context` (a serial port, say) in front of both parts.
    pub(crate) fn within(context: &str, error: &bacnet_types::error::Error) -> Self {
        let Self { summary, detail } = Self::of(error);
        Self {
            summary: format!("{context}: {summary}"),
            detail: format!("{context}: {detail}"),
        }
    }

    /// From an error that is only text (a profile the service refused,
    /// say): the whole message without an OS error number, which the
    /// footer shortens with `…`. Its
    /// clauses aren't split, as a `field: value` error needs both.
    pub(crate) fn text(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            summary: strip_os_error(&detail).to_string(),
            detail,
        }
    }
}

#[cfg(test)]
impl From<&str> for Failure {
    fn from(detail: &str) -> Self {
        Self::text(detail)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

/// An OS error by its kind (`address in use`); one the stack wrote itself
/// (`own address not local`) by its own message.
fn io_summary(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(_) => error.kind().to_string(),
        None => strip_os_error(&error.to_string()).to_string(),
    }
}

/// `text` without a trailing ` (os error N)`.
fn strip_os_error(text: &str) -> &str {
    match text.rfind(" (os error ") {
        Some(at) if text.ends_with(')') => &text[..at],
        _ => text,
    }
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
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WorkerEvent {
    /// An unexpected worker failure makes resource ownership uncertain.
    Fatal { error: String },
    /// A profile I/O operation completed.
    Profile(Result<super::profile::service::Reply, String>),
    /// An attempted command was not admitted.
    Rejected { error: String },
    /// A new session owns all subsequent tagged events.
    SessionStarted { generation: u64 },
    /// Every resource of this session has been retired.
    Disconnected { generation: u64 },
    /// Events from a single connection attempt, isolated across reconnects.
    Session {
        generation: u64,
        event: Box<WorkerEvent>,
    },
    /// The client is up; `local` is its own address.
    Connected {
        /// Local address for the status bar.
        local: String,
    },
    /// Building the client failed. The connection can be edited and retried.
    ConnectFailed {
        /// The error.
        error: Failure,
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
    /// An operation command could not enter the session's command queue.
    OpRejected {
        /// Operation identity, checked even after optimistic cancellation.
        op: OpId,
        /// Whether the refused command was a cancellation.
        cancel: bool,
        /// Admission failure; this is not an operation completion.
        error: String,
    },
    /// An operation ended.
    OpFinished {
        /// Operation id.
        op: OpId,
        /// How.
        outcome: OpOutcome,
    },
    /// A device browser result.
    Browse {
        /// The request it answers.
        req: ReqId,
        /// The result.
        result: BrowseResult,
    },
    /// A watch list report.
    Watch(super::watched::WatchEvent),
    /// A write session report.
    Safety(super::safety::SafetyEvent),
    /// A passive session started: there is no client.
    Passive,
    /// The client holds, or keeps asking for, an entry in this BBMD's
    /// foreign-device table.
    ForeignDevice(super::network::Foreign),
    /// A Network screen report.
    Network(super::network::NetworkEvent),
    /// An I-Have, a router announcement or another network report.
    Discovery(super::network::discovery::DiscoveryEvent),
    /// An SC screen report.
    #[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
    Sc(super::sc::ScReport),
    /// An MS/TP screen report.
    #[cfg_attr(not(feature = "serial"), allow(dead_code))]
    Mstp(super::mstp::MstpReport),
    /// An MS/TP monitor report (#970).
    Monitor(super::monitor::MonitorReport),
    /// Captured frames and capture reports (#971).
    Capture(super::capture::CaptureEvent),
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
}

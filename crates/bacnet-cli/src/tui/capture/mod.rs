//! Live capture (#971): frames from the stack's own traffic, from the
//! MS/TP monitor, and with `pcap` from libpcap, kept in a bounded ring for
//! the Capture screen and exported to pcap files Wireshark opens.
//!
//! - [`worker`]: the pumps that read a source, summarise each frame and
//!   hand them to the UI in batches, counting every frame lost on the way.
//! - [`ring`]: the bounded store of frames and their summaries, with the
//!   index of those the filter keeps.
//! - [`filter`]: the filter line's terms.
//! - [`pcap`]: the export, with IPv4 and UDP headers made up for B/IP
//!   frames the tap saw inside the process.
//! - `libpcap` (`pcap` feature): wire capture on a blocking thread.
//! - [`redact`]: DeviceCommunicationControl and ReinitializeDevice
//!   passwords masked before any frame is kept, whichever way it went.
//!
//! Sources:
//!
//! - The frame tap of `bacnet-transport` (#957) on B/IP, B/IPv6, BACnet/SC
//!   and MS/TP: what this node sends and receives, and on MS/TP what it
//!   hears on the line. BACnet/SC frames are the plaintext, before TLS.
//! - The MS/TP monitor (#958, #970): every frame on the trunk, through its
//!   own lossy subscription beside the monitor screen's.
//! - libpcap (`tui --capture-device`), B/IP datagrams on the `-p` port.
//!
//! Frames arrive summarised: the worker reads the headers, so the UI thread
//! only stores and draws. The Capture screen decodes the selected frame in
//! full, and only that one.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;

pub(crate) use bacnet_ops::decode::frame::{FrameSummary, Framing};

pub(crate) mod filter;
#[cfg(feature = "pcap")]
pub(crate) mod libpcap;
pub(crate) mod pcap;
pub(crate) mod redact;
pub(crate) mod ring;
pub(crate) mod worker;

/// Frames the ring holds by default: past that the oldest go.
pub(crate) const DEFAULT_RING: usize = 50_000;

/// Which way a frame went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Received by this node.
    In,
    /// Sent by this node.
    Out,
    /// Heard on the wire between other nodes (the monitor, libpcap).
    Wire,
}

impl Direction {
    /// The list's short form.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::Wire => "--",
        }
    }
}

/// One captured frame, summarised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureFrame {
    /// When the source saw it.
    pub(crate) time: SystemTime,
    /// The link, which decides how `bytes` are read.
    pub(crate) framing: Framing,
    /// Which way it went.
    pub(crate) direction: Direction,
    /// The UDP source and destination of a B/IP datagram, when known: the
    /// export writes them into the IPv4 and UDP headers it makes up.
    pub(crate) udp: Option<(SocketAddr, SocketAddr)>,
    /// Where it came from, as the device table writes addresses.
    pub(crate) source: String,
    /// Where it went.
    pub(crate) destination: String,
    /// For a routed NPDU, whose ends `source` and `destination` give as
    /// routed stations (`2:0x05`): the link peer it went through.
    pub(crate) via: Option<String>,
    /// The frame as the link carries it; empty for a line error the
    /// monitor reports.
    pub(crate) bytes: Bytes,
    /// Its headers, read by the worker.
    pub(crate) summary: FrameSummary,
    /// A password in it was masked before it reached the ring ([`redact`]).
    pub(crate) redacted: bool,
}

/// UI to worker: start capturing in this session. Idempotent; the capture
/// runs until the session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureCommand {
    /// Subscribe to the session's source.
    Start,
}

/// A pcap export of the ring, handed to the coordinator, which writes it
/// on a blocking thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportJob {
    /// The frames, oldest first, shared with the ring.
    pub(crate) frames: Vec<Arc<CaptureFrame>>,
    /// B/IP as raw IPv4 (LINKTYPE 228) rather than Ethernet (LINKTYPE 1).
    pub(crate) raw_ipv4: bool,
    /// The file name stem, `capture-<time>`.
    pub(crate) stem: String,
}

/// Worker to UI.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CaptureEvent {
    /// A source is running.
    Started {
        /// What it captures.
        what: String,
        /// It belongs to the session, and ends with it; libpcap doesn't.
        session: bool,
    },
    /// New frames, and frames lost since the last batch that got through.
    Frames(Box<Batch>),
    /// A source ended, or never started.
    Ended {
        /// What it captured, as [`CaptureEvent::Started`] named it.
        what: String,
        /// Why it ended.
        why: String,
    },
    /// An export finished: the files written, and the error that stopped
    /// the rest, if one did.
    Exported(pcap::Exported),
}

/// Frames for the ring.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Batch {
    /// Oldest first.
    pub(crate) frames: Vec<CaptureFrame>,
    /// Frames lost before reaching the UI since the last batch it got.
    pub(crate) lost: u64,
}

//! What the Linux receive loop does with one raw frame before it hands an
//! NPDU up: a pure decision ([`classify_frame`]), and the work that follows
//! it ([`receive_frame`]) with the socket's send injected, so every OS can
//! test both.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_types::data_link::DataLink;
use tracing::debug;

use super::egress::{LlcReply, ReplyFailures};
use super::{
    EthernetFrame, LLC_CONTROL_TEST_CMD, LLC_CONTROL_XID_CMD, accepts_ethernet_destination,
    build_test_response, build_xid_response, check_llc_control, decode_ethernet_frame,
    is_group_mac,
};
use crate::tap::{FrameTap, TapDirection, TapPeer};
use crate::warn_throttle::WarnThrottle;

/// Log a frame dropped because the network layer's NPDU channel is full:
/// a warning at most once per
/// [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL) of
/// `warning`, with the drops since the last one, and debug for the rest
/// (#1950). Returns the drops reported, when it warns.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn npdu_channel_full(warning: &WarnThrottle) -> Option<u64> {
    crate::warn_throttled!(
        warning,
        "Ethernet: NPDU channel full, dropping incoming frame"
    )
}

/// The receive loop's decision on one raw frame.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) enum FrameIngress<'a> {
    /// Not addressed to this station, sent by it, or too short to name a
    /// source: dropped without a trace.
    Ignore,
    /// Its source MAC is a group address (#1492). An IEEE 802 station never
    /// sends from one, so the frame is forged or broken; answering it, or a
    /// confirmed request in it, would go to every station in the group. It
    /// is dropped and counted, ahead of the XID and TEST handlers.
    GroupSource {
        /// The group address the frame claims to come from.
        source: [u8; 6],
    },
    /// An XID command (Clause 7.1): answer `source`.
    Xid {
        /// The station to answer.
        source: [u8; 6],
    },
    /// A TEST command (Clause 7.1): echo `data` to `source`.
    Test {
        /// The station to answer.
        source: [u8; 6],
        /// The octets after the LLC header, sent back as they came.
        data: &'a [u8],
    },
    /// Anything else: decode it as a UI frame, which drops what isn't one.
    Decode {
        /// The station that sent it.
        source: [u8; 6],
    },
}

impl FrameIngress<'_> {
    /// The sender of a frame the transport takes: every one but an
    /// ignored frame.
    fn taken_from(&self) -> Option<[u8; 6]> {
        match *self {
            Self::Ignore => None,
            Self::GroupSource { source }
            | Self::Xid { source }
            | Self::Test { source, .. }
            | Self::Decode { source } => Some(source),
        }
    }
}

/// Decide what to do with `data`, a raw frame received by the station at
/// `local_mac`. The destination check comes first, then the source checks,
/// and only then the LLC commands, so no command from a group source is
/// ever answered.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn classify_frame<'a>(data: &'a [u8], local_mac: &[u8; 6]) -> FrameIngress<'a> {
    if !accepts_ethernet_destination(data, local_mac) {
        return FrameIngress::Ignore;
    }
    let Some(Ok(source)) = data.get(6..12).map(<[u8; 6]>::try_from) else {
        return FrameIngress::Ignore;
    };
    if is_group_mac(&source) {
        return FrameIngress::GroupSource { source };
    }
    if source == *local_mac {
        return FrameIngress::Ignore;
    }
    match check_llc_control(data) {
        Some(LLC_CONTROL_XID_CMD) => FrameIngress::Xid { source },
        Some(LLC_CONTROL_TEST_CMD) => FrameIngress::Test {
            source,
            data: data.get(17..).unwrap_or_default(),
        },
        _ => FrameIngress::Decode { source },
    }
}

/// Carry out the receive loop's decision on `data`, a raw frame received by
/// the station at `local_mac`, and return the frame whose NPDU goes up, if
/// any. A frame the transport takes goes to `tap` first (#1856), before the
/// node acts on it, so an XID or TEST command shows before its response.
/// Then a frame from a group source is counted in `group_source_drops`, an
/// XID or TEST command is answered with `send` (a failure counted in
/// `reply_failures`), and anything else is decoded as a UI frame.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn receive_frame(
    data: &[u8],
    local_mac: &[u8; 6],
    tap: &FrameTap,
    group_source_drops: &AtomicU64,
    reply_failures: &ReplyFailures,
    send: impl FnMut(&[u8]) -> io::Result<usize>,
) -> Option<EthernetFrame> {
    let ingress = classify_frame(data, local_mac);
    if let Some(source) = ingress.taken_from() {
        let peer = || TapPeer::Ethernet(source);
        tap.record(TapDirection::Inbound, DataLink::Ethernet, peer, data);
    }
    match ingress {
        FrameIngress::Ignore => None,
        FrameIngress::GroupSource { source } => {
            group_source_drops.fetch_add(1, Ordering::Relaxed);
            debug!(
                src = ?source,
                "Dropping frame whose source MAC is a group address"
            );
            None
        }
        FrameIngress::Xid { source } => {
            debug!(src = ?source, "XID command, sending response");
            let response = build_xid_response(local_mac, &source);
            reply_failures.send_reply(LlcReply::Xid, &source, &response, tap, send);
            None
        }
        FrameIngress::Test { source, data } => {
            debug!(src = ?source, len = data.len(), "TEST command, sending response");
            let response = build_test_response(local_mac, &source, data);
            reply_failures.send_reply(LlcReply::Test, &source, &response, tap, send);
            None
        }
        FrameIngress::Decode { .. } => decode_ethernet_frame(data).ok(),
    }
}

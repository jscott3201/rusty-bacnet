//! What the Linux receive loop does with one raw frame before it answers an
//! LLC command or decodes the frame, as a pure decision every OS can test.

use super::{
    LLC_CONTROL_TEST_CMD, LLC_CONTROL_XID_CMD, accepts_ethernet_destination, check_llc_control,
    is_group_mac,
};
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
    Decode,
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
        _ => FrameIngress::Decode,
    }
}

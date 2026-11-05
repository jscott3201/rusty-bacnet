//! Sending one whole raw frame and handing it to the frame tap, and what the
//! receive loop does when an XID or TEST reply fails to go out (#1918). The
//! send itself is injected, so every OS can test a failed or short send
//! without a packet socket.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_types::data_link::DataLink;
use tracing::{debug, warn};

use crate::tap::{FrameTap, TapDirection, TapPeer};
use crate::warn_throttle::WarnThrottle;

/// Send `frame` with `send`, which returns how many octets went, as a
/// socket's `send` does. A packet socket sends a frame whole or not at all,
/// so fewer octets than the frame is an error (`WriteZero`), not a partial
/// success to finish later. An interrupted call is retried.
pub(super) fn send_whole_frame(
    frame: &[u8],
    mut send: impl FnMut(&[u8]) -> io::Result<usize>,
) -> io::Result<()> {
    loop {
        match send(frame) {
            Ok(sent) if sent == frame.len() => return Ok(()),
            Ok(sent) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!("short Ethernet send: {sent} of {} octets", frame.len()),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

/// Send `frame`, addressed to `destination`, whole with `send`
/// ([`send_whole_frame`]), and once it went, hand it to `tap` (#1856). Every
/// frame the transport sends goes through here: NPDUs, and XID and TEST
/// responses. A frame that failed to go out is not tapped.
pub(super) fn send_tapped(
    tap: &FrameTap,
    destination: &[u8; 6],
    frame: &[u8],
    send: impl FnMut(&[u8]) -> io::Result<usize>,
) -> io::Result<()> {
    send_whole_frame(frame, send)?;
    let peer = || TapPeer::Ethernet(*destination);
    tap.record(TapDirection::Outbound, DataLink::Ethernet, peer, frame);
    Ok(())
}

/// Which LLC command a reply answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LlcReply {
    /// An XID response.
    Xid,
    /// A TEST response.
    Test,
}

impl LlcReply {
    fn name(self) -> &'static str {
        match self {
            LlcReply::Xid => "XID",
            LlcReply::Test => "TEST",
        }
    }
}

/// The count of XID and TEST replies that failed to go out, and the
/// throttle on their warning. A reply is fire-and-forget, so nothing else
/// learns of the failure; a send error such as a full queue tends to repeat
/// for every reply, so the warning goes out at most once per
/// [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL) with the
/// failures since the last one, and the rest log at debug.
#[derive(Debug, Default)]
pub(super) struct ReplyFailures {
    count: AtomicU64,
    warning: WarnThrottle,
}

impl ReplyFailures {
    /// Replies that failed to go out so far.
    pub(super) fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Send `reply`, a `kind` reply to `destination`, with `send`, handing
    /// it to `tap` once it went; on a failed or short send, count and log
    /// it. Returns the failures reported when it warns.
    pub(super) fn send_reply(
        &self,
        kind: LlcReply,
        destination: &[u8; 6],
        reply: &[u8],
        tap: &FrameTap,
        send: impl FnMut(&[u8]) -> io::Result<usize>,
    ) -> Option<u64> {
        let error = send_tapped(tap, destination, reply, send).err()?;
        self.count.fetch_add(1, Ordering::Relaxed);
        let failed = self.warning.note();
        let kind = kind.name();
        match failed {
            Some(failed) => warn!(
                %error,
                kind,
                dst = ?destination,
                failed,
                "Ethernet: failed to send LLC replies since the last warning"
            ),
            None => {
                debug!(%error, kind, dst = ?destination, "Ethernet: failed to send an LLC reply")
            }
        }
        failed
    }
}

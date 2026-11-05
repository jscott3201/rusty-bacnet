//! The MS/TP side of the frame tap (#957).
//!
//! The core consumes Token, Poll For Master and other control frames
//! itself and hands up only data, so the adapter finds frame boundaries
//! for the tap on its own: while someone is subscribed, every octet the
//! core is fed (after the echo filter, so this station's own frames are
//! not seen twice) also goes through a [`StreamDecoder`], and each frame it
//! completes is reported with the octets it arrived as. A frame this
//! station sends is reported as it was written. With nobody subscribed an
//! octet costs one atomic load, and the decoder is reset so that a later
//! subscriber starts clean.
//!
//! # Timing (#1856)
//!
//! The decoder gives a partial frame up when the core does, so a frame the
//! node aborted is not reported whole, and one it took is reported once:
//!
//! - when the silence before an octet is longer than T_frame_abort. Each
//!   octet comes with the stamp the core gets for it, the time of the read
//!   that returned it, and the rule is the core's: strictly longer, with
//!   the core's T_frame_abort.
//! - when the core confirms the start of a frame this station sends, since
//!   the receiver goes off as the driver goes on.
//!
//! A frame given up is not reported at all, as the passive monitor's line
//! reader (`mstp::monitor`) gives a partial frame up: a tapped frame is
//! always a whole one. The monitor's own timing doesn't fit here: it works
//! each octet's arrival back from the end of its read at the baud rate,
//! which shortens the silence it measures, so a frame the core aborted for
//! a silence just past T_frame_abort would pass it.

use bacnet_mstp_codec::{DecodeEvent, StreamDecoder, StreamPhase};
use bacnet_mstp_core::{Duration, Instant};
use bacnet_types::data_link::DataLink;

use crate::tap::{FrameTap, TapDirection, TapPeer};

/// Offset of the destination octet in a frame: after the two preamble
/// octets and the frame type.
const DESTINATION: usize = 3;

/// The decoder the tap reads received octets with, and the octets it holds,
/// as they arrived (an extended frame's data is decoded in its storage).
pub(super) struct TapDecoder<'a> {
    decoder: StreamDecoder<'a>,
    /// Ends with the octets the decoder buffers.
    raw: Vec<u8>,
    /// The core's T_frame_abort.
    frame_abort: Duration,
    /// The stamp of the last octet taken while someone was subscribed.
    last_octet: Option<Instant>,
}

impl<'a> TapDecoder<'a> {
    /// A decoder over `storage`, which must hold the longest frame the port
    /// can receive, that gives a partial frame up after a silence longer
    /// than `frame_abort`, the core's T_frame_abort.
    pub(super) fn new(storage: &'a mut [u8], frame_abort: Duration) -> Self {
        Self {
            decoder: StreamDecoder::new(storage).with_reserved_cobs(),
            raw: Vec::new(),
            frame_abort,
            last_octet: None,
        }
    }

    /// Take one octet the core was fed, stamped `now` as the core's was,
    /// reporting a frame it completes. After a silence longer than
    /// T_frame_abort, a partial frame is given up first, as the core gives
    /// it up.
    pub(super) fn push(&mut self, octet: u8, now: Instant, tap: &FrameTap) {
        if !tap.is_active() {
            if !self.raw.is_empty() {
                self.abandon();
            }
            return;
        }
        if self.decoder.phase() != StreamPhase::Sync
            && self
                .last_octet
                .is_some_and(|last| now - last > self.frame_abort)
        {
            self.abandon();
        }
        self.last_octet = Some(now);
        self.raw.push(octet);
        let mut input: &[u8] = &[octet];
        loop {
            let event = self.decoder.push_slice(input);
            input = &[];
            let (source, consumed) = match event {
                DecodeEvent::NeedMore => break,
                DecodeEvent::Frame { frame, consumed } => (frame.header.source, consumed),
                DecodeEvent::Invalid { .. } | DecodeEvent::Malformed { .. } => {
                    // More may be buffered behind the discard: look again.
                    self.trim();
                    continue;
                }
                DecodeEvent::Error(_) => {
                    self.abandon();
                    return;
                }
            };
            // The frame is the first of the buffered octets.
            let start = self.raw.len().saturating_sub(self.decoder.buffered_len());
            if let Some(wire) = self.raw.get(start..start + consumed) {
                let peer = || TapPeer::Mstp(source);
                tap.record(TapDirection::Inbound, DataLink::Mstp, peer, wire);
            }
        }
        self.trim();
    }

    /// Give up a partial frame, reporting nothing: after a silence, or
    /// because the core confirmed the start of a frame this station sends.
    pub(super) fn abandon(&mut self) {
        self.decoder.reset();
        self.raw.clear();
    }

    /// Drop what the decoder no longer buffers.
    fn trim(&mut self) {
        let stale = self.raw.len().saturating_sub(self.decoder.buffered_len());
        self.raw.drain(..stale);
    }
}

/// Report a frame this station wrote.
pub(super) fn record_sent(tap: &FrameTap, frame: &[u8]) {
    let peer = || TapPeer::Mstp(frame.get(DESTINATION).copied().unwrap_or_default());
    tap.record(TapDirection::Outbound, DataLink::Mstp, peer, frame);
}

#[cfg(test)]
mod tests {
    use bacnet_mstp_codec::{Frame, FrameType, MAX_EXTENDED_FRAME_LENGTH};

    use super::*;

    /// T_frame_abort as the host transport sets it.
    const FRAME_ABORT: Duration = Duration::from_millis(crate::mstp::HOST_FRAME_ABORT_MS);

    /// A decoder over `storage` with the host's T_frame_abort.
    fn decoder(storage: &mut [u8]) -> TapDecoder<'_> {
        TapDecoder::new(storage, FRAME_ABORT)
    }

    fn wire(frame: Frame<'_>) -> Vec<u8> {
        let mut out = vec![0; MAX_EXTENDED_FRAME_LENGTH];
        let length = frame.encode_into(&mut out).unwrap();
        out.truncate(length);
        out
    }

    /// Frames split by noise, a broken candidate and an extended frame each
    /// come out once, with the octets they arrived as.
    #[test]
    fn received_frames_are_reported_as_they_arrived() {
        let token = wire(Frame::new(FrameType::TOKEN, 2, 1, &[]));
        let data = wire(Frame::new(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            255,
            3,
            &[1, 0],
        ));
        let npdu = [0x5A; 600];
        let extended = wire(Frame::new(
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            2,
            4,
            &npdu,
        ));
        let mut broken = token.clone();
        broken[7] ^= 0xFF;
        let mut line = vec![0x00, 0x13];
        for part in [&token, &broken, &data, &[0x55][..], &extended] {
            line.extend_from_slice(part);
        }

        let tap = FrameTap::new();
        let mut frames = tap.subscribe();
        let mut storage = vec![0; MAX_EXTENDED_FRAME_LENGTH];
        let mut decoder = decoder(&mut storage);
        for octet in line {
            decoder.push(octet, Instant::ZERO, &tap);
        }
        for (expected, source) in [(&token, 1), (&data, 3), (&extended, 4)] {
            let frame = frames.try_recv().unwrap();
            assert_eq!(frame.direction, TapDirection::Inbound);
            assert_eq!(frame.peer, TapPeer::Mstp(source));
            assert_eq!(&frame.bytes[..], &expected[..]);
        }
        assert!(frames.try_recv().is_none());
        assert!(decoder.raw.len() <= decoder.decoder.buffered_len());
    }

    /// Without a subscriber nothing is buffered, and a frame cut by an
    /// unsubscribed gap is not reported half.
    #[test]
    fn nothing_is_held_without_a_subscriber() {
        let token = wire(Frame::new(FrameType::TOKEN, 2, 1, &[]));
        let tap = FrameTap::new();
        let mut storage = vec![0; MAX_EXTENDED_FRAME_LENGTH];
        let mut decoder = decoder(&mut storage);
        let mut frames = tap.subscribe();
        for &octet in &token[..4] {
            decoder.push(octet, Instant::ZERO, &tap);
        }
        drop(frames);
        decoder.push(token[4], Instant::ZERO, &tap);
        assert!(decoder.raw.is_empty());
        frames = tap.subscribe();
        for &octet in &token[5..] {
            decoder.push(octet, Instant::ZERO, &tap);
        }
        assert!(frames.try_recv().is_none());
        for &octet in &token {
            decoder.push(octet, Instant::ZERO, &tap);
        }
        assert_eq!(&frames.try_recv().unwrap().bytes[..], &token[..]);
    }

    /// A silence longer than T_frame_abort in the middle of a frame gives it
    /// up, as the core gives it up (#1856): its tail is not reported with
    /// its head. A silence of exactly T_frame_abort keeps the frame, as the
    /// core keeps it, and the next frame is reported once.
    #[test]
    fn a_silence_past_frame_abort_gives_the_partial_frame_up() {
        let data = wire(Frame::new(
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            255,
            3,
            &[1, 0],
        ));
        let tap = FrameTap::new();
        let mut frames = tap.subscribe();
        let mut storage = vec![0; MAX_EXTENDED_FRAME_LENGTH];
        let mut decoder = decoder(&mut storage);
        let split = data.len() - 1;
        let mut at = Instant::from_millis(5);
        for (silence, reported) in [
            (FRAME_ABORT, true),
            (FRAME_ABORT + Duration::from_nanos(1), false),
        ] {
            for &octet in &data[..split] {
                decoder.push(octet, at, &tap);
            }
            at += silence;
            decoder.push(data[split], at, &tap);
            assert_eq!(frames.try_recv().is_some(), reported, "{silence:?}");
        }
        for &octet in &data {
            decoder.push(octet, at, &tap);
        }
        assert_eq!(&frames.try_recv().unwrap().bytes[..], &data[..]);
        assert!(frames.try_recv().is_none());
    }

    /// The core gives up a partial frame when it starts to send, and so
    /// does the tap: the octets after the start are not that frame's.
    #[test]
    fn a_transmit_start_gives_the_partial_frame_up() {
        let token = wire(Frame::new(FrameType::TOKEN, 2, 1, &[]));
        let tap = FrameTap::new();
        let mut frames = tap.subscribe();
        let mut storage = vec![0; MAX_EXTENDED_FRAME_LENGTH];
        let mut decoder = decoder(&mut storage);
        for &octet in &token[..5] {
            decoder.push(octet, Instant::ZERO, &tap);
        }
        decoder.abandon();
        for &octet in token[5..].iter().chain(&token) {
            decoder.push(octet, Instant::ZERO, &tap);
        }
        assert_eq!(&frames.try_recv().unwrap().bytes[..], &token[..]);
        assert!(frames.try_recv().is_none());
    }

    #[test]
    fn a_sent_frame_names_its_destination() {
        let tap = FrameTap::new();
        let mut frames = tap.subscribe();
        let pfm = wire(Frame::new(FrameType::POLL_FOR_MASTER, 9, 1, &[]));
        record_sent(&tap, &pfm);
        let frame = frames.try_recv().unwrap();
        assert_eq!(
            (frame.direction, frame.peer),
            (TapDirection::Outbound, TapPeer::Mstp(9))
        );
        assert_eq!(&frame.bytes[..], &pfm[..]);
    }
}

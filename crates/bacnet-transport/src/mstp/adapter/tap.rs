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
//! The decoder sees octets only, not when they arrived: the core's
//! inter-octet timeout (T_frame_abort) is not applied, so a frame the core
//! aborted for a gap in the middle can still be reported whole.

use bacnet_mstp_codec::{DecodeEvent, StreamDecoder};
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
}

impl<'a> TapDecoder<'a> {
    /// A decoder over `storage`, which must hold the longest frame the port
    /// can receive.
    pub(super) fn new(storage: &'a mut [u8]) -> Self {
        Self {
            decoder: StreamDecoder::new(storage).with_reserved_cobs(),
            raw: Vec::new(),
        }
    }

    /// Take one octet the core was fed, reporting a frame it completes.
    pub(super) fn push(&mut self, octet: u8, tap: &FrameTap) {
        if !tap.is_active() {
            if !self.raw.is_empty() {
                self.decoder.reset();
                self.raw.clear();
            }
            return;
        }
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
                    self.decoder.reset();
                    self.raw.clear();
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
        let mut decoder = TapDecoder::new(&mut storage);
        for octet in line {
            decoder.push(octet, &tap);
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
        let mut decoder = TapDecoder::new(&mut storage);
        let mut frames = tap.subscribe();
        for &octet in &token[..4] {
            decoder.push(octet, &tap);
        }
        drop(frames);
        decoder.push(token[4], &tap);
        assert!(decoder.raw.is_empty());
        frames = tap.subscribe();
        for &octet in &token[5..] {
            decoder.push(octet, &tap);
        }
        assert!(frames.try_recv().is_none());
        for &octet in &token {
            decoder.push(octet, &tap);
        }
        assert_eq!(&frames.try_recv().unwrap().bytes[..], &token[..]);
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

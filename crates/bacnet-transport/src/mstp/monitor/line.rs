//! From timestamped reads to timestamped line events: the stream decoder,
//! plus working back from each read's arrival time to each frame's, and
//! giving up a partial frame after a silence.

use std::time::Duration;

use bacnet_mstp_codec::{DecodeEvent, Frame, StreamDecoder, StreamPhase};

use super::stats::{LineErrorKind, LineEvent, octet_time};

/// The stream position and timing state that goes with a [`StreamDecoder`].
#[derive(Debug, Clone)]
pub(crate) struct LineClock {
    octet_time: Duration,
    frame_abort: Duration,
    /// Octets pushed into the decoder so far.
    fed: u64,
    /// Octets the decoder's events have accounted for so far. Ahead of
    /// `fed` while the decoder skips a dropped frame by count.
    position: u64,
    /// When the last octet arrived.
    last_octet_at: Option<Duration>,
}

impl LineClock {
    pub(crate) fn new(baud_rate: u32, frame_abort: Duration) -> Self {
        Self {
            octet_time: octet_time(baud_rate),
            frame_abort,
            fed: 0,
            position: 0,
            last_octet_at: None,
        }
    }

    fn wire_time(&self, octets: u64) -> Duration {
        self.octet_time
            .saturating_mul(u32::try_from(octets).unwrap_or(u32::MAX))
    }

    /// When the octet that ends at stream position `end` arrived, given that
    /// the read ending at position `fed` returned at `at`: the octets after
    /// it took their wire time to come in.
    fn stamp(&self, at: Duration, end: u64) -> Duration {
        at.saturating_sub(self.wire_time(self.fed.saturating_sub(end)))
    }

    /// Account for an event that covered `octets` octets, and return when its
    /// last one arrived.
    fn advance(&mut self, at: Duration, octets: usize) -> Duration {
        let end = self
            .position
            .saturating_add(u64::try_from(octets).unwrap_or(u64::MAX));
        self.position = end;
        self.stamp(at, end.min(self.fed))
    }
}

/// Decode `chunk`, which a read returned at `at`, and hand each resulting
/// event to `sink` with the time its last octet arrived, and the frame
/// itself for a valid one.
///
/// Before decoding, a partial frame is given up when the gap between the
/// previous read's last octet and this chunk's first is longer than the
/// clock's frame-abort time.
pub(crate) fn feed_chunk(
    decoder: &mut StreamDecoder<'_>,
    clock: &mut LineClock,
    chunk: &[u8],
    at: Duration,
    mut sink: impl FnMut(Duration, LineEvent, Option<&Frame<'_>>),
) {
    if chunk.is_empty() {
        return;
    }
    let length = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
    if let Some(last) = clock.last_octet_at {
        let first_began = at.saturating_sub(clock.wire_time(length));
        if decoder.phase() != StreamPhase::Sync
            && first_began.saturating_sub(last) > clock.frame_abort
        {
            decoder.reset();
            clock.position = clock.fed;
            sink(last, LineEvent::FrameAbort, None);
        }
    }
    let mut rest = chunk;
    while !rest.is_empty() {
        let free = decoder.capacity() - decoder.buffered_len();
        if free == 0 {
            // Can't happen with storage for the longest frame plus a read,
            // since every complete frame is drained below; recover anyway.
            decoder.reset();
            clock.position = clock.fed;
            sink(
                clock.stamp(at, clock.fed),
                LineEvent::Error {
                    kind: LineErrorKind::Malformed,
                    destination: None,
                    octets: 0,
                },
                None,
            );
            continue;
        }
        let (piece, after) = rest.split_at(rest.len().min(free));
        rest = after;
        clock.fed = clock
            .fed
            .saturating_add(u64::try_from(piece.len()).unwrap_or(u64::MAX));
        // The stamps of this piece's events assume the rest of the chunk
        // was still to come.
        let piece_at =
            at.saturating_sub(clock.wire_time(u64::try_from(rest.len()).unwrap_or(u64::MAX)));
        let mut event = decoder.push_slice(piece);
        loop {
            match event {
                DecodeEvent::NeedMore => break,
                DecodeEvent::Frame { frame, consumed } => {
                    let stamp = clock.advance(piece_at, consumed);
                    sink(
                        stamp,
                        LineEvent::Frame {
                            header: frame.header,
                            wire_length: consumed,
                        },
                        Some(&frame),
                    );
                }
                DecodeEvent::Invalid { discard } => {
                    let stamp = clock.advance(piece_at, discard);
                    sink(stamp, LineEvent::Noise { octets: discard }, None);
                }
                DecodeEvent::Malformed {
                    discard,
                    error,
                    destination,
                } => {
                    let stamp = clock.advance(piece_at, discard);
                    sink(
                        stamp,
                        LineEvent::Error {
                            kind: LineErrorKind::from_frame_error(error),
                            destination,
                            octets: discard,
                        },
                        None,
                    );
                }
                // The piece fits the free storage, so this is not expected.
                DecodeEvent::Error(_) => break,
            }
            event = decoder.push_slice(&[]);
        }
    }
    clock.last_octet_at = Some(at);
}

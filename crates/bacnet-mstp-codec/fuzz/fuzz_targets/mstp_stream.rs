//! `StreamDecoder` on structure-aware input.
//!
//! # Input
//!
//! A stream of chunks, each written by `mstp_codec_fuzz::encode`:
//!
//! - frames of every type: standard ones with 0 to 501 octets of data, and
//!   COBS-encoded ones with 1 to 2032. Those of types 32 and 33 are valid
//!   extended frames; those of the reserved types 34-127 have a header that
//!   passes, and a type the codec decodes only in the reserved-COBS mode
//!   below;
//! - damaged frames: any of those with one bit flipped past the header, so
//!   the header passes and the data fails its check;
//! - raw noise, any octets, and inert noise, which has no 0x55 and so can't
//!   start a candidate.
//!
//! A decoder with 1 to 2117 octets of storage (the longest frame is 2053)
//! takes the stream in pushes of arbitrary size, single octets through
//! `push`, with empty pushes, drains (empty pushes until `NeedMore`) and
//! resets (a drain, then `reset`) between them. A refused push is pushed
//! again in pieces that fit. What the steps leave is pushed the same way,
//! then drained.
//!
//! The input also picks the decoder's mode: by default, or built
//! `with_reserved_cobs`, which checks the reserved COBS types as a receiver
//! with COBS support does. In that mode a reserved type is decodable like
//! Types 32 and 33: the oracle's `decode_with` and `decodable_with` take the
//! mode, a generated reserved frame is a valid one, and `UnsupportedCobsFrame`
//! never comes out. Everything below holds in both modes, with "a type the
//! codec decodes" read for the mode.
//!
//! # Oracle
//!
//! The target keeps a shadow of the decoder: the stream offset of the first
//! octet it still holds, which runs ahead of the octets pushed while it
//! skips a dropped frame by count, and the frame it delivered last. Each
//! push is checked against it:
//!
//! - Refusal: a push is refused exactly when the octets it has to buffer,
//!   those past the end of a skip, are more than the free room, counting the
//!   frame the push commits, with those two sizes, and nothing is taken.
//! - Never wedged: before every push there is room for one more octet,
//!   `NeedMore` leaves the storage short of full, and a drain that starts
//!   with `n` octets buffered ends within `n + 1` empty pushes (see
//!   `Run::drain`).
//! - `NeedMore` only while nothing is buffered, or the front of a candidate
//!   that still lacks octets and will fit: its header, once complete, passes
//!   its CRC and the checks of Clause 9.5.8, and has a type the codec
//!   decodes.
//! - `Invalid` is one run of noise: it starts with an octet that can't begin
//!   a preamble, and runs to the next 0x55 or to the end of what is
//!   buffered. So it throws away no possible preamble.
//! - `Malformed` drops a candidate at the front, with the error, discard and
//!   destination the decoder's docs give for it: after a header that passed
//!   (a frame longer than the storage, a reserved COBS type, data that fails
//!   its check) the whole frame the header declares, skipped by count, and
//!   after a header that failed or never completed, one octet.
//! - `Frame` is the whole valid frame at the front, by
//!   `mstp_codec_fuzz::decode`, with the same fields and decoded data.
//! - The buffered length matches the shadow after every push, and so does
//!   the phase after `NeedMore` or a frame: the data phase while skipping.
//!
//! At the end, the delivered frames are in stream order and don't overlap,
//! so none came out twice, and every guaranteed generated frame came out at
//! its offset. A generated frame is guaranteed when it fits the storage, no
//! reset fell inside it, and no 0x55 lies between the anchor and its first
//! octet. The anchor is the latest of the stream's start, the last reset at
//! or before the frame, and the end of the last frame delivered, or dropped
//! and skipped by count, before it; one that runs past the frame's first
//! octet hid it. From the anchor the decoder starts fresh, and octets other
//! than 0x55 can't start a candidate, so nothing can hide the frame.
//!
//! That end-of-run check is a backstop. A decoder can only lose a guaranteed
//! frame by discarding its preamble as noise, dropping it as malformed,
//! skipping over it, or holding it whole behind `NeedMore`, and the
//! per-push checks fail at the push where it does any of those.
//!
//! Noise with a 0x55 can hide what follows it: it may hold a header that
//! passes its checks and declares a length running into the next frame, and
//! the decoder then waits for, delivers, or drops and skips that candidate
//! whole. A frame dropped after a good header hides nothing, since the
//! decoder skips exactly the octets its header declares (#1609): its end is
//! an anchor.

#![no_main]

use arbitrary::Arbitrary;
use bacnet_mstp_codec::{DecodeEvent, FrameError, StreamDecoder, StreamPhase};
use libfuzzer_sys::fuzz_target;
use mstp_codec_fuzz::{self as wire, Header, HEADER_END, MAX_COBS_WIRE, MAX_DATA};

/// A stream stops taking chunks past this many octets, to keep runs quick.
const MAX_STREAM: usize = 16 * 1024;
/// Storage sizes run from 1 octet to this many.
const MAX_CAPACITY: usize = MAX_COBS_WIRE + 64;
/// Data lengths of COBS-encoded frames run from 1 to this many octets: the
/// most whose encoding fits the largest Length whatever they hold.
const MAX_COBS_DATA: usize = 2032;

#[derive(Debug, Arbitrary)]
struct Input {
    /// The storage size, 1 to `MAX_CAPACITY` (modulo).
    capacity: u16,
    /// Build the decoder `with_reserved_cobs`.
    reserved_cobs: bool,
    chunks: Vec<Chunk>,
    steps: Vec<Step>,
}

/// One frame, before the target fixes up its fields.
#[derive(Debug, Arbitrary)]
struct FrameSpec {
    frame_type: u8,
    destination: u8,
    source: u8,
    length: u16,
    pattern: Vec<u8>,
}

#[derive(Debug, Arbitrary)]
enum Chunk {
    /// A frame whose header passes: source 255 becomes 254. A COBS type gets
    /// 1 to 2032 octets of data (`length` modulo 2032, plus one); any other
    /// type gets `length` modulo 502, none on a type defined without data,
    /// and at least the two octets of a vendor identifier on a proprietary
    /// type. `pattern` is repeated to fill the data.
    Frame(FrameSpec),
    /// The same frame with at least one octet of data, and the bit
    /// `bit` (modulo 8) of the octet `position` (modulo the octets after the
    /// header) flipped.
    Damaged {
        frame: FrameSpec,
        position: u16,
        bit: u8,
    },
    /// Any octets.
    Noise(Vec<u8>),
    /// Octets with each 0x55 turned into 0x54.
    Inert(Vec<u8>),
}

#[derive(Debug, Arbitrary)]
enum Step {
    /// Push the next 1 to `MAX_CAPACITY` octets (modulo) as one slice.
    Push(u16),
    /// One empty push.
    Empty,
    /// Empty pushes until `NeedMore`.
    Drain,
    /// A drain, then `reset`.
    Reset,
}

/// The generated octets, and the offset and length of each valid frame in
/// them.
struct Stream {
    octets: Vec<u8>,
    frames: Vec<(usize, usize)>,
}

impl FrameSpec {
    /// The frame's wire octets. `with_data` asks for at least one data
    /// octet whatever the type, so there is something past the header.
    fn octets(&self, with_data: bool) -> Vec<u8> {
        let raw = self.frame_type;
        let length = usize::from(self.length);
        let length = match raw {
            _ if wire::is_cobs(raw) => 1 + length % MAX_COBS_DATA,
            _ if with_data => 1 + length % MAX_DATA,
            _ if wire::never_carries_data(raw) => 0,
            _ if wire::is_proprietary(raw) => (length % (MAX_DATA + 1)).max(2),
            _ => length % (MAX_DATA + 1),
        };
        let data = wire::fill(length, &self.pattern);
        wire::encode(raw, self.destination, self.source.min(0xFE), &data)
    }
}

impl Stream {
    /// The stream `chunks` make. A frame of a reserved COBS type is a valid
    /// one only to a decoder that checks them, `reserved_cobs`.
    fn build(chunks: &[Chunk], reserved_cobs: bool) -> Self {
        let mut stream = Self {
            octets: Vec::new(),
            frames: Vec::new(),
        };
        for chunk in chunks {
            if stream.octets.len() >= MAX_STREAM {
                break;
            }
            match chunk {
                Chunk::Frame(spec) => {
                    let frame = spec.octets(false);
                    let raw = spec.frame_type;
                    if !wire::is_cobs(raw) || wire::is_extended(raw) || reserved_cobs {
                        stream.frames.push((stream.octets.len(), frame.len()));
                    }
                    stream.octets.extend_from_slice(&frame);
                }
                Chunk::Damaged {
                    frame,
                    position,
                    bit,
                } => {
                    let mut octets = frame.octets(true);
                    let index = HEADER_END + usize::from(*position) % (octets.len() - HEADER_END);
                    octets[index] ^= 1 << (bit % 8);
                    stream.octets.extend_from_slice(&octets);
                }
                Chunk::Noise(octets) => stream.octets.extend_from_slice(octets),
                Chunk::Inert(octets) => stream.octets.extend(octets.iter().map(|&octet| {
                    if octet == 0x55 {
                        0x54
                    } else {
                        octet
                    }
                })),
            }
        }
        stream
    }
}

/// A `DecodeEvent`, copied out of the decoder's storage.
#[derive(Debug, PartialEq)]
enum Seen {
    NeedMore,
    Frame {
        consumed: usize,
        fields: [u8; 3],
        data: Vec<u8>,
    },
    Invalid {
        discard: usize,
    },
    Malformed {
        discard: usize,
        error: FrameError,
        destination: Option<u8>,
    },
    Error(FrameError),
}

impl From<DecodeEvent<'_>> for Seen {
    fn from(event: DecodeEvent<'_>) -> Self {
        match event {
            DecodeEvent::NeedMore => Self::NeedMore,
            DecodeEvent::Frame { frame, consumed } => Self::Frame {
                consumed,
                fields: [
                    frame.header.frame_type.to_raw(),
                    frame.header.destination,
                    frame.header.source,
                ],
                data: frame.data.to_vec(),
            },
            DecodeEvent::Invalid { discard } => Self::Invalid { discard },
            DecodeEvent::Malformed {
                discard,
                error,
                destination,
            } => Self::Malformed {
                discard,
                error,
                destination,
            },
            DecodeEvent::Error(error) => Self::Error(error),
        }
    }
}

/// A decoder fed from a stream, with the shadow of its buffer.
struct Run<'a> {
    decoder: StreamDecoder<'a>,
    capacity: usize,
    /// The decoder checks reserved COBS types.
    reserved_cobs: bool,
    stream: &'a [u8],
    /// Stream octets pushed and taken so far.
    pushed: usize,
    /// Stream offset of the first octet the decoder holds. Past `pushed`
    /// while it skips a dropped frame: the octets up to it go unbuffered.
    start: usize,
    /// Wire length of the frame the last push delivered, which the next
    /// push commits.
    pending: usize,
    /// Offset and length of each delivered frame.
    delivered: Vec<(usize, usize)>,
    /// Offset and length of each frame dropped and skipped by count.
    skipped: Vec<(usize, usize)>,
    /// Stream offsets at which the decoder was reset.
    resets: Vec<usize>,
}

impl Run<'_> {
    /// Octets the decoder buffers.
    fn buffered(&self) -> usize {
        self.pushed.saturating_sub(self.start)
    }

    /// Octets of a dropped frame still to come, which it drops unbuffered.
    fn skip(&self) -> usize {
        self.start.saturating_sub(self.pushed)
    }

    /// Room for the next push, which first commits a delivered frame.
    fn free(&self) -> usize {
        self.capacity - (self.buffered() - self.pending)
    }

    /// Push the next `length` stream octets, check what the decoder
    /// reports, and return it.
    fn push(&mut self, length: usize) -> Seen {
        let free = self.free();
        assert!(free >= 1, "full storage and nothing to report: wedged");
        let stream = self.stream;
        let input = &stream[self.pushed..self.pushed + length];
        let seen = Seen::from(match input {
            [octet] => self.decoder.push(*octet),
            _ => self.decoder.push_slice(input),
        });
        self.start += self.pending;
        self.pending = 0;
        let to_store = length - self.skip().min(length);
        if to_store > free {
            let refused = FrameError::BufferExhausted {
                needed: to_store,
                available: free,
            };
            assert_eq!(seen, Seen::Error(refused));
        } else {
            self.pushed += length;
            let shadow = &stream[self.start.min(self.pushed)..self.pushed];
            let reserved = self.reserved_cobs;
            match &seen {
                Seen::NeedMore => need_more(shadow, self.capacity, reserved),
                Seen::Invalid { discard } => {
                    invalid(shadow, *discard);
                    self.start += discard;
                }
                Seen::Malformed {
                    discard,
                    error,
                    destination,
                } => {
                    let skipped = malformed(
                        shadow,
                        *discard,
                        *error,
                        *destination,
                        self.capacity,
                        reserved,
                    );
                    if skipped {
                        self.skipped.push((self.start, *discard));
                    }
                    self.start += discard;
                }
                Seen::Frame {
                    consumed,
                    fields,
                    data,
                } => {
                    frame(shadow, *consumed, *fields, data, reserved);
                    self.delivered.push((self.start, *consumed));
                    self.pending = *consumed;
                }
                Seen::Error(error) => panic!("push of {length} with {free} free: {error:?}"),
            }
        }
        assert_eq!(self.decoder.buffered_len(), self.buffered());
        self.check_phase(&seen);
        seen
    }

    /// After `NeedMore` or a frame, the phase the shadow is in.
    fn check_phase(&self, seen: &Seen) {
        let expected = match seen {
            Seen::Frame { .. } => StreamPhase::Sync,
            Seen::NeedMore if self.skip() > 0 => StreamPhase::Data,
            Seen::NeedMore => match self.buffered() {
                0 => StreamPhase::Sync,
                1 => StreamPhase::Preamble,
                buffered if buffered < HEADER_END => StreamPhase::Header,
                _ => StreamPhase::Data,
            },
            _ => return,
        };
        assert_eq!(self.decoder.phase(), expected);
    }

    /// Push the next `length` octets; if that is refused, push them again
    /// in pieces that fit.
    fn feed(&mut self, length: usize) {
        let end = self.pushed + length;
        if !matches!(self.push(length), Seen::Error(_)) {
            return;
        }
        while self.pushed < end {
            let piece = (end - self.pushed).min(self.free());
            self.push(piece);
        }
    }

    /// Empty pushes until `NeedMore`. A drain adds no octets, and each event
    /// before `NeedMore` uses up buffered octets of its own: `Invalid` and
    /// `Malformed` discard at least one, and a frame's are committed by the
    /// next push. So a drain that starts with `n` octets buffered reports at
    /// most `n` such events, and ends within `n + 1` pushes.
    fn drain(&mut self) {
        let limit = self.buffered() + 1;
        for _ in 0..limit {
            if self.push(0) == Seen::NeedMore {
                return;
            }
        }
        panic!("a drain did not end within {limit} empty pushes");
    }

    fn reset(&mut self) {
        self.drain();
        self.decoder.reset();
        self.start = self.pushed;
        self.resets.push(self.pushed);
        assert_eq!(self.decoder.buffered_len(), 0);
        assert_eq!(self.decoder.phase(), StreamPhase::Sync);
    }
}

/// Whether `shadow` holds the start of a preamble, as much of one as it has.
fn starts_preamble(shadow: &[u8]) -> bool {
    shadow.starts_with(&wire::PREAMBLE[..shadow.len().min(2)])
}

/// `NeedMore`: nothing buffered, or the front of a candidate that still
/// lacks octets and will fit. `reserved` is the decoder's mode.
fn need_more(shadow: &[u8], capacity: usize, reserved: bool) {
    assert!(
        shadow.len() < capacity,
        "NeedMore with full storage: wedged"
    );
    if shadow.is_empty() {
        return;
    }
    assert!(
        starts_preamble(shadow),
        "NeedMore holding noise: {shadow:02X?}"
    );
    assert!(
        capacity >= HEADER_END,
        "NeedMore for a header that can't fit"
    );
    let header = Header::read(shadow);
    match header {
        Header::Incomplete => {}
        Header::Good { length, .. } if header.decodable_with(reserved) => {
            let total = wire::wire_length(length);
            assert!(
                total <= capacity,
                "NeedMore for {total} octets in {capacity}"
            );
            assert!(shadow.len() < total, "NeedMore holding a whole candidate");
        }
        _ => panic!("NeedMore after a header to drop: {header:?}"),
    }
}

/// `Invalid`: one run of noise, from an octet that can't start a preamble
/// to the next 0x55 or the end of the buffer.
fn invalid(shadow: &[u8], discard: usize) {
    assert!(!shadow.is_empty(), "Invalid with nothing buffered");
    assert!(
        !starts_preamble(&shadow[..shadow.len().min(2)]),
        "Invalid for a possible preamble: {shadow:02X?}"
    );
    let run = shadow[1..]
        .iter()
        .position(|&octet| octet == 0x55)
        .map_or(shadow.len(), |index| index + 1);
    assert_eq!(discard, run, "Invalid split a run of noise: {shadow:02X?}");
}

/// `Malformed`: the candidate at the front was dropped. Its destination is
/// known once the header CRC has passed. Returns whether the decoder drops
/// it whole by count, because its header passed; otherwise only its first
/// octet goes.
fn malformed(
    shadow: &[u8],
    discard: usize,
    error: FrameError,
    destination: Option<u8>,
    capacity: usize,
    reserved: bool,
) -> bool {
    assert!(!shadow.is_empty(), "Malformed with nothing buffered");
    assert!(
        starts_preamble(shadow),
        "Malformed for noise: {shadow:02X?}"
    );
    let header = Header::read(shadow);
    let (known, frame_type, source, declared) = match header {
        Header::Good {
            destination,
            frame_type,
            source,
            length,
        } => (Some(destination), frame_type, source, length),
        _ => (None, 0, 0, 0),
    };
    assert_eq!(destination, known, "destination for {error:?}");
    let good = matches!(header, Header::Good { .. });
    let cobs = good && wire::is_cobs(frame_type);
    let decodable = header.decodable_with(reserved);
    // The frame's wire length, when its header passed.
    let total = header.wire_length();
    let skipped = match error {
        FrameError::HeaderCrcMismatch if header == Header::BadCrc => false,
        FrameError::BroadcastSource if good && source == 0xFF => false,
        FrameError::InvalidCobsLength { length }
            if cobs && source != 0xFF && length == declared && !header.passes() =>
        {
            false
        }
        FrameError::Oversize { length, maximum }
            if good && !cobs && source != 0xFF && length == declared && length > MAX_DATA =>
        {
            assert_eq!(maximum, MAX_DATA);
            false
        }
        // Never in the mode, where every header that passes is decodable.
        FrameError::UnsupportedCobsFrame { raw_type }
            if header.passes() && !decodable && raw_type == frame_type =>
        {
            true
        }
        FrameError::FrameTooLong {
            needed,
            capacity: reported,
        } if reported == capacity && needed > capacity => match header {
            Header::Incomplete => {
                assert_eq!(needed, HEADER_END);
                false
            }
            _ if decodable => {
                assert_eq!(Some(needed), total);
                true
            }
            _ => panic!("FrameTooLong for header {header:?}"),
        },
        FrameError::DataCrcMismatch { length } if decodable && length == declared => {
            let total = wire::wire_length(length);
            assert!(
                total <= shadow.len(),
                "DataCrcMismatch before the frame is whole"
            );
            assert_eq!(
                wire::decode_with(shadow, reserved),
                None,
                "DataCrcMismatch for a good frame"
            );
            true
        }
        _ => panic!("Malformed {error:?} for {header:?}: {shadow:02X?}"),
    };
    let expected = if skipped {
        total.expect("a skipped frame's header passed")
    } else {
        1
    };
    assert_eq!(discard, expected, "discard for {error:?}");
    skipped
}

/// `Frame`: the whole valid frame at the front, with its fields and its
/// decoded data, a reserved COBS type's included in the mode.
fn frame(shadow: &[u8], consumed: usize, fields: [u8; 3], data: &[u8], reserved: bool) {
    assert_eq!(
        wire::decode_with(shadow, reserved),
        Some((consumed, data.to_vec())),
        "delivered something other than the frame at the front: {shadow:02X?}"
    );
    assert_eq!(fields, [shadow[2], shadow[3], shadow[4]]);
}

/// Frames come out in order, once each, and every guaranteed generated
/// frame came out at its offset (see the module docs).
fn check_delivered(stream: &Stream, capacity: usize, run: &Run<'_>) {
    for pair in run.delivered.windows(2) {
        assert!(pair[0].0 + pair[0].1 <= pair[1].0, "overlap: {pair:?}");
    }
    // Both kinds of span end where the decoder starts fresh.
    let mut spans: Vec<(usize, usize)> =
        run.delivered.iter().chain(&run.skipped).copied().collect();
    spans.sort_unstable();
    let (mut next_span, mut next_reset, mut anchor) = (0, 0, 0);
    for &(offset, length) in &stream.frames {
        while let Some(&(start, size)) = spans.get(next_span) {
            if start >= offset {
                break;
            }
            anchor = anchor.max(start + size);
            next_span += 1;
        }
        while let Some(&reset) = run.resets.get(next_reset) {
            if reset > offset {
                break;
            }
            anchor = anchor.max(reset);
            next_reset += 1;
        }
        let hidden = anchor > offset;
        let straddled = run
            .resets
            .get(next_reset)
            .is_some_and(|&reset| reset < offset + length);
        if length <= capacity
            && !hidden
            && !straddled
            && !stream.octets[anchor..offset].contains(&0x55)
        {
            assert!(
                run.delivered.binary_search(&(offset, length)).is_ok(),
                "the frame at {offset} ({length} octets) didn't come out"
            );
        }
    }
}

fuzz_target!(
    init: wire::self_test(),
    |input: Input| {
        let reserved_cobs = input.reserved_cobs;
        let stream = Stream::build(&input.chunks, reserved_cobs);
        let capacity = 1 + usize::from(input.capacity) % MAX_CAPACITY;
        let mut storage = vec![0u8; capacity];
        let mut decoder = StreamDecoder::new(&mut storage);
        if reserved_cobs {
            decoder = decoder.with_reserved_cobs();
        }
        assert_eq!(decoder.checks_reserved_cobs(), reserved_cobs);
        let mut run = Run {
            decoder,
            capacity,
            reserved_cobs,
            stream: &stream.octets,
            pushed: 0,
            start: 0,
            pending: 0,
            delivered: Vec::new(),
            skipped: Vec::new(),
            resets: Vec::new(),
        };
        assert_eq!(run.decoder.capacity(), capacity);
        for step in &input.steps {
            match *step {
                Step::Push(length) => {
                    let left = stream.octets.len() - run.pushed;
                    run.feed((1 + usize::from(length) % MAX_CAPACITY).min(left));
                }
                Step::Empty => {
                    run.push(0);
                }
                Step::Drain => run.drain(),
                Step::Reset => run.reset(),
            }
        }
        run.feed(stream.octets.len() - run.pushed);
        run.drain();
        check_delivered(&stream, capacity, &run);
    }
);

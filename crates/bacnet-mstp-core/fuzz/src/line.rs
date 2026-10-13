//! The line as the station hears it, rebuilt by the checker from what the
//! target feeds the core: the whole valid frames, the invalid ones, the
//! events EventCount counts, and when the line was last active.
//!
//! The checker runs a decoder of its own over the octets the core takes as
//! line activity, so it knows which frames the core heard without asking
//! the core. The rules for that are the receive state machine's (Clause
//! 9.5.4) and the core's documented adapter contract:
//!
//! - a partial frame left longer than T_frame_abort is dropped, and counts
//!   as an invalid frame once past its preamble;
//! - a receive error drops a partial frame;
//! - the confirmed start of the station's own frame drops a partial frame,
//!   since the receiver is off while the driver is on;
//! - from that start to the frame's completion, what the receiver hears is
//!   the station's own echo, which the target never shows the line;
//! - an extended frame heard by a station without extended frames is an
//!   invalid frame, not a frame.
//!
//! Which dropped candidates the core counts as invalid frames depends on
//! their destination; the checker counts every one, so it may see an
//! invalid frame the core doesn't, but never misses one.

use bacnet_mstp_codec::{DecodeEvent, StreamDecoder, StreamPhase};

use crate::model::Seen;

/// What one input told the checker, beyond activity.
#[derive(Debug)]
pub enum Heard {
    /// A frame that may count as invalid (Clause 9.5.4).
    Invalid,
    /// A whole valid frame.
    Frame(Seen),
}

/// The line, as the station hears it.
pub struct Line<'s> {
    decoder: StreamDecoder<'s>,
    extended: bool,
    frame_abort: u64,
    last_octet: Option<u64>,
    /// When the line was last active: an octet, a receive error, a frame or
    /// octets a host assembler reported, or the station's own transmission.
    pub last_activity: u64,
    /// EventCount's events so far: octets and receive errors, plus one for
    /// each frame or invalid frame a host assembler reports.
    pub events: u64,
    /// Whole valid frames heard so far.
    pub frames: u64,
}

impl<'s> Line<'s> {
    /// A quiet line, heard through `storage`, which is as long as the core's
    /// receive storage.
    pub fn new(storage: &'s mut [u8], extended: bool, frame_abort: u64) -> Self {
        let decoder = StreamDecoder::new(storage);
        Self {
            decoder: if extended {
                decoder.with_reserved_cobs()
            } else {
                decoder
            },
            extended,
            frame_abort,
            last_octet: None,
            last_activity: 0,
            events: 0,
            frames: 0,
        }
    }

    /// Drop a partial frame that has waited longer than T_frame_abort.
    pub fn expire_partial(&mut self, now: u64, heard: &mut Vec<Heard>) {
        let phase = self.decoder.phase();
        if phase != StreamPhase::Sync
            && self
                .last_octet
                .is_some_and(|last| now - last > self.frame_abort)
        {
            if matches!(phase, StreamPhase::Header | StreamPhase::Data) {
                heard.push(Heard::Invalid);
            }
            self.decoder.reset();
        }
    }

    fn activity(&mut self, now: u64, events: u64) {
        self.last_activity = now;
        self.events += events;
    }

    /// One octet off the line.
    pub fn octet(&mut self, octet: u8, now: u64, heard: &mut Vec<Heard>) {
        self.expire_partial(now, heard);
        self.activity(now, 1);
        self.last_octet = Some(now);
        let octet = [octet];
        let mut input: &[u8] = &octet;
        loop {
            let event = match self.decoder.push_slice(input) {
                DecodeEvent::NeedMore => return,
                DecodeEvent::Invalid { .. } => None,
                DecodeEvent::Frame { frame, .. }
                    if frame.header.frame_type.is_cobs() && !self.extended =>
                {
                    Some(Heard::Invalid)
                }
                DecodeEvent::Frame { frame, .. } => Some(Heard::Frame(Seen::of(frame))),
                DecodeEvent::Malformed { .. } | DecodeEvent::Error(_) => Some(Heard::Invalid),
            };
            input = &[];
            if let Some(event) = event {
                if matches!(event, Heard::Frame(_)) {
                    self.frames += 1;
                }
                heard.push(event);
            }
        }
    }

    /// A receive error: an invalid frame when it breaks into one past its
    /// preamble.
    pub fn receive_error(&mut self, now: u64, heard: &mut Vec<Heard>) {
        self.activity(now, 1);
        if matches!(
            self.decoder.phase(),
            StreamPhase::Header | StreamPhase::Data
        ) {
            heard.push(Heard::Invalid);
        }
        self.decoder.reset();
    }

    /// A frame a host assembler validated, which the core took.
    pub fn validated(&mut self, frame: Seen, now: u64, heard: &mut Vec<Heard>) {
        self.activity(now, 1);
        self.frames += 1;
        heard.push(Heard::Frame(frame));
    }

    /// An invalid frame a host assembler reported.
    pub fn invalid(&mut self, now: u64, heard: &mut Vec<Heard>) {
        self.activity(now, 1);
        heard.push(Heard::Invalid);
    }

    /// Octets a host assembler reported.
    pub fn host_activity(&mut self, octets: u64, now: u64) {
        if octets > 0 {
            self.activity(now, octets);
        }
    }

    /// The station confirmed a start: its receiver goes off.
    pub fn started(&mut self) {
        self.decoder.reset();
    }

    /// The station's driver went off at `now`, its frame complete or cut
    /// short.
    pub fn driven_until(&mut self, now: u64) {
        self.last_activity = now;
    }

    /// Whether a frame is part way through.
    pub fn partial(&self) -> bool {
        self.decoder.phase() != StreamPhase::Sync
    }
}

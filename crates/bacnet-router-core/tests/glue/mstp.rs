//! An MS/TP master core over boxed storage, and the frames it sends.

use bacnet_mstp_codec::{
    decode_frame_in_place, Frame, FrameType, MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA,
};
use bacnet_mstp_core::{
    Action, ExtendedActionBuffers, HeaplessQueues, Instant, MasterCore, MstpCoreConfig,
};

/// The router's station on the MS/TP port.
pub const STATION: u8 = 1;
/// The next master, which passes it the token.
pub const NEXT: u8 = 2;

/// Queues of `U` unicast slots and one broadcast slot, each for an
/// extended NPDU.
pub type Queues<const U: usize> = HeaplessQueues<U, 1, MAX_EXTENDED_MPDU_DATA>;

/// A master core over a [`Bus`]'s storage.
pub type Core<'a, const U: usize> = MasterCore<'a, &'a mut Queues<U>, MAX_EXTENDED_MPDU_DATA>;

/// The router's station, masters 0 to 2, with extended frames on, or
/// without them when `extended` is false.
pub fn config(extended: bool) -> MstpCoreConfig {
    config_at(STATION, extended)
}

/// The same for `station`.
pub fn config_at(station: u8, extended: bool) -> MstpCoreConfig {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = station;
    config.max_master = 2;
    if extended {
        config.with_extended_frames()
    } else {
        config
    }
}

/// Storage for a master core, boxed to keep it off the stack.
pub struct Bus<const U: usize> {
    rx: Vec<u8>,
    actions: Box<ExtendedActionBuffers<1>>,
    /// The core's queues, which a test reads once the core is gone.
    pub queues: Box<Queues<U>>,
}

impl<const U: usize> Bus<U> {
    pub fn new() -> Self {
        Self {
            rx: vec![0; MAX_EXTENDED_FRAME_LENGTH],
            actions: Box::default(),
            queues: Box::new(Queues::new()),
        }
    }

    /// The router's core, with extended frames on, or off, built at t = 0.
    pub fn core(&mut self, extended: bool) -> Core<'_, U> {
        self.core_at(STATION, extended)
    }

    /// The core of `station`, with extended frames on, or off.
    pub fn core_at(&mut self, station: u8, extended: bool) -> Core<'_, U> {
        MasterCore::new(
            config_at(station, extended),
            &mut *self.queues,
            &mut self.rx,
            &mut self.actions,
            Instant::ZERO,
        )
        .unwrap()
    }
}

/// A frame the core sent, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wired {
    pub frame_type: FrameType,
    pub destination: u8,
    pub data: Vec<u8>,
    /// The frame as it went on the wire.
    pub octets: Vec<u8>,
}

/// The next frame the core sends from `now` on, serving its deadlines
/// while it waits, and sent whole; `None` if it sends nothing.
pub fn next_frame<const U: usize>(core: &mut Core<'_, U>, now: Instant) -> Option<Wired> {
    let mut now = now;
    for _ in 0..32 {
        match core.poll_action() {
            Action::Transmit { id, frame } => {
                let octets = frame.to_vec();
                core.on_transmit_started(id, now).unwrap();
                core.on_transmit_complete(id, now).unwrap();
                let mut decoded = octets.clone();
                let (frame, _) = decode_frame_in_place(&mut decoded).unwrap();
                return Some(Wired {
                    frame_type: frame.header.frame_type,
                    destination: frame.header.destination,
                    data: frame.data.to_vec(),
                    octets,
                });
            }
            Action::Idle => {
                now = core.next_deadline()?.max(now);
                core.on_deadline(now).unwrap();
            }
            _ => {}
        }
    }
    None
}

/// The core takes the token from [`NEXT`] at `now`, and sends the first
/// frame it has queued.
pub fn token_then_frame<const U: usize>(core: &mut Core<'_, U>, now: Instant) -> Option<Wired> {
    let token = Frame::new(FrameType::TOKEN, STATION, NEXT, &[]);
    core.on_validated_frame(token, now).unwrap();
    next_frame(core, now)
}

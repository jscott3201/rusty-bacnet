//! Receiver observations and bounded, terminal-character receipt correlation.

use std::collections::VecDeque;

use bacnet_mstp_codec::{DecodeEvent, FrameHeader, MAX_EXTENDED_FRAME_LENGTH, StreamDecoder};
use bacnet_mstp_core::{Duration, Instant};

use super::master::Input;
use super::{NodeKind, Sim};
use crate::line::{Character, Contributor, TxId, WindowOutcome};
use crate::{ContentionPolicy, ContentionStats, SimRng, TraceKind, ViolationKind, Who};

const RECEIPT_BOUND: usize = 513;

struct Progress {
    tx: TxId,
    next: usize,
    incarnation: u64,
    start: Instant,
    contended: bool,
}

struct Receipt {
    tx: TxId,
    start: Instant,
    terminal: Instant,
    incarnation: u64,
    header: FrameHeader,
    data: Vec<u8>,
}

pub(super) struct Receiver {
    pub policy: ContentionPolicy,
    pub rng: SimRng,
    stats: ContentionStats,
    progress: Option<Progress>,
    pending: VecDeque<Receipt>,
    overflowed: bool,
    decoder: Option<StreamDecoder<'static>>,
    last_input: Option<Instant>,
    /// A newly enabled audit cannot assume an already running parser is in
    /// sync. A known receive reset is required before its first obligation.
    synchronized: bool,
}

impl Receiver {
    pub fn new(seed: u64, station: u8) -> Self {
        Self {
            policy: ContentionPolicy::default(),
            rng: SimRng::new(
                seed ^ 0x3C6E_F372_FE94_F82B
                    ^ u64::from(station).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            ),
            stats: ContentionStats::default(),
            progress: None,
            pending: VecDeque::new(),
            overflowed: false,
            decoder: None,
            last_input: None,
            synchronized: true,
        }
    }

    fn reset_decoder(&mut self) {
        if let Some(decoder) = &mut self.decoder {
            decoder.reset();
        }
        self.last_input = None;
        self.synchronized = true;
    }

    fn decode(
        &mut self,
        input: &Input,
        now: Instant,
        abort: Duration,
    ) -> Vec<(FrameHeader, Vec<u8>)> {
        if self.last_input.is_some_and(|last| now - last > abort) {
            self.reset_decoder();
        }
        self.last_input = Some(now);
        let Input::Octet(byte) = input else {
            self.reset_decoder();
            return Vec::new();
        };
        let decoder = self.decoder.get_or_insert_with(|| {
            StreamDecoder::new(super::leak_storage(2 * MAX_EXTENDED_FRAME_LENGTH))
        });
        let octet = [*byte];
        let mut bytes: &[u8] = &octet;
        let mut frames = Vec::new();
        loop {
            match decoder.push_slice(bytes) {
                DecodeEvent::NeedMore => break,
                DecodeEvent::Frame { frame, .. } => {
                    self.stats.decoded += 1;
                    frames.push((frame.header, frame.data.to_vec()));
                }
                DecodeEvent::Malformed { .. } | DecodeEvent::Error(_) => self.stats.malformed += 1,
                DecodeEvent::Invalid { .. } => {}
            }
            bytes = &[];
        }
        frames
    }
}

impl Sim {
    /// Receiver-specific digital contention and bounded audit accounting.
    #[must_use]
    pub fn contention_stats(&self, station: u8) -> &ContentionStats {
        &self.nodes[self.index_of(station)].receiver.stats
    }

    /// Physical eligibility/provenance creates a candidate, not permission to
    /// deliver anything. Actual forwarding and independent decoding qualify it.
    pub(super) fn observe_contention(
        &mut self,
        node: usize,
        outcome: &WindowOutcome,
        character: Character,
        contributor: Option<Contributor>,
    ) {
        if self.nodes[node].receiver.policy == ContentionPolicy::FramingError {
            return;
        }
        if outcome.contended {
            let stats = &mut self.nodes[node].receiver.stats;
            stats.windows += 1;
            match character {
                Character::Garbled => stats.framing_errors += 1,
                _ if contributor.is_some_and(|c| c.character == character) => stats.captured += 1,
                _ => stats.altered += 1,
            }
            self.note(
                Who::Station(self.nodes[node].station),
                TraceKind::Contention {
                    start: outcome.start,
                    transmission: contributor.map(|c| c.tx),
                    index: contributor.map(|c| c.index),
                    byte: match character {
                        Character::Clean(byte) => Some(byte),
                        Character::Garbled => None,
                    },
                },
            );
        }
        let incarnation = match &self.nodes[node].kind {
            NodeKind::Master(master) if self.nodes[node].listening_since.is_some() => {
                master.incarnation
            }
            _ => {
                self.nodes[node].receiver.progress = None;
                return;
            }
        };
        let contributor = contributor.filter(|c| {
            c.aligned
                && self
                    .line
                    .get(c.tx)
                    .is_some_and(|tx| Character::Clean(tx.octets[c.index]) == character)
        });
        let Some(c) = contributor else {
            self.nodes[node].receiver.progress = None;
            return;
        };
        let receiver = &mut self.nodes[node].receiver;
        if c.index == 0 {
            receiver.progress = Some(Progress {
                tx: c.tx,
                next: 0,
                incarnation,
                start: outcome.start,
                contended: false,
            });
        }
        let Some(progress) = receiver.progress.as_mut() else {
            return;
        };
        if progress.tx != c.tx || progress.next != c.index || progress.incarnation != incarnation {
            receiver.progress = None;
            return;
        }
        progress.next += 1;
        progress.contended |= outcome.contended;
        let tx = self.line.get(c.tx).expect("a current contributor");
        if progress.next != tx.octets.len() {
            return;
        }
        let progress = receiver.progress.take().unwrap();
        if !progress.contended {
            return;
        }
        let Some((header, data)) = super::decode_whole(&tx.octets) else {
            return;
        };
        receiver.stats.intact += 1;
        if receiver.pending.len() == RECEIPT_BOUND {
            receiver.stats.overflows += 1;
            if !std::mem::replace(&mut receiver.overflowed, true) {
                self.violation(
                    ViolationKind::EventGrowth,
                    "receiver receipts exceeded 513 candidates".into(),
                );
            }
            return;
        }
        receiver.pending.push_back(Receipt {
            tx: c.tx,
            start: progress.start,
            terminal: self.now,
            incarnation,
            header,
            data,
        });
        receiver.stats.pending = receiver.pending.len();
        receiver.stats.max_pending = receiver.stats.max_pending.max(receiver.pending.len());
    }

    /// Compare only the actual input at the candidate's terminal wire time.
    /// Later counters, including frames in the same read, cannot discharge it.
    pub(super) fn receiver_input(
        &mut self,
        node: usize,
        input: Input,
        wire_end: Instant,
    ) -> Result<(), super::master::Refused> {
        let NodeKind::Master(master) = &self.nodes[node].kind else {
            return Ok(());
        };
        let Some(core) = master.core.as_ref() else {
            return Ok(());
        };
        let before = core.counters().frames_received;
        let transmitting = master.transmitting.is_some();
        let incarnation = master.incarnation;
        let extended = master.spec.config.extended_frames;
        let abort = master.spec.config.t_frame_abort;
        let receiver = &mut self.nodes[node].receiver;
        if receiver.policy == ContentionPolicy::FramingError && receiver.decoder.is_none() {
            if transmitting || matches!(input, Input::Garbled) {
                receiver.reset_decoder();
            } else {
                receiver.last_input = Some(self.now);
                receiver.synchronized = false;
            }
            return self.master_input(node, input);
        }
        let decoded = if transmitting {
            receiver.reset_decoder();
            Vec::new()
        } else {
            receiver.decode(&input, self.now, abort)
        };
        let receipt = receiver
            .pending
            .iter()
            .position(|r| r.terminal == wire_end)
            .and_then(|at| receiver.pending.remove(at));
        receiver.stats.pending = receiver.pending.len();
        let synchronized = receiver.synchronized;
        let result = self.master_input(node, input);
        if let Some(receipt) = receipt {
            self.nodes[node].receiver.overflowed = false;
            let qualified = synchronized
                && receipt.incarnation == incarnation
                && decoded.len() == 1
                && decoded[0] == (receipt.header, receipt.data);
            if qualified {
                let expected = u32::from(extended || !receipt.header.frame_type.is_cobs());
                let after = self
                    .master_mut(node)
                    .unwrap()
                    .core
                    .as_ref()
                    .unwrap()
                    .counters()
                    .frames_received;
                let got = after.saturating_sub(before);
                self.nodes[node].receiver.stats.checked += 1;
                if got == expected {
                    self.nodes[node].receiver.stats.matched += 1;
                } else {
                    self.violation(ViolationKind::ListenerMissedFrame, format!(
                        "station {} captured tx{} ending at {} us: terminal input counted {got}, expected {expected}",
                        self.nodes[node].station, receipt.tx, wire_end.as_micros()));
                }
            } else {
                self.nodes[node].receiver.stats.unqualified += 1;
            }
        }
        result
    }

    /// An explicit host filter disposition invalidates the physical candidate
    /// containing that byte. It is not an unexplained missing core receipt.
    pub(super) fn receiver_filter_drop(&mut self, node: usize, wire_end: Instant) {
        let receiver = &mut self.nodes[node].receiver;
        if receiver
            .progress
            .as_ref()
            .is_some_and(|p| p.start < wire_end)
        {
            receiver.progress = None;
        }
        let before = receiver.pending.len();
        receiver
            .pending
            .retain(|r| !(r.start < wire_end && wire_end <= r.terminal));
        receiver.stats.cancelled += (before - receiver.pending.len()) as u64;
        receiver.stats.pending = receiver.pending.len();
        if receiver.pending.len() < RECEIPT_BOUND {
            receiver.overflowed = false;
        }
    }

    pub(super) fn receiver_check_dropped(&mut self, node: usize, retained: &[Instant]) {
        let receiver = &mut self.nodes[node].receiver;
        let mut lost = Vec::new();
        receiver.pending.retain(|receipt| {
            if retained.contains(&receipt.terminal) {
                true
            } else {
                lost.push((receipt.tx, receipt.terminal));
                false
            }
        });
        receiver.stats.pending = receiver.pending.len();
        if receiver.pending.len() < RECEIPT_BOUND {
            receiver.overflowed = false;
        }
        for (tx, terminal) in lost {
            self.violation(
                ViolationKind::ListenerMissedFrame,
                format!(
                "station {} lost captured tx{tx}'s terminal character at {} us before core input",
                self.nodes[node].station, terminal.as_micros()),
            );
        }
    }

    pub(super) fn receiver_has_pending(&self, node: usize) -> bool {
        !self.nodes[node].receiver.pending.is_empty()
    }

    /// The reference parser follows actual core lifecycle boundaries.
    pub(super) fn receiver_started(&mut self, node: usize) {
        self.nodes[node].receiver.progress = None;
        self.nodes[node].receiver.reset_decoder();
    }

    pub(super) fn receiver_power_off(&mut self, node: usize) {
        let receiver = &mut self.nodes[node].receiver;
        receiver.progress = None;
        receiver.reset_decoder();
        receiver.stats.cancelled += receiver.pending.len() as u64;
        receiver.pending.clear();
        receiver.overflowed = false;
        receiver.stats.pending = 0;
    }
}

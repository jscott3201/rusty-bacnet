//! Chunked serial reads around the unchanged portable core.
mod application;
#[cfg(test)]
mod contention_lifecycle_tests;
#[cfg(test)]
mod contention_matrix_tests;
#[cfg(test)]
mod contention_tests;
#[cfg(test)]
mod delayed_echo_tests;
#[cfg(test)]
mod differential;
mod echo;
#[cfg(test)]
mod gap_tests;
mod local_echo;
#[cfg(test)]
mod local_echo_tests;
#[cfg(test)]
mod repair_tests;
mod state;
#[cfg(test)]
mod tests;

use super::Sim;
use super::master::Input;
use crate::line::{Character, TxId};
use crate::report::{HostStats, HostTransmitTiming, ViolationKind};
use crate::trace::{TraceKind, Who};
pub(crate) use local_echo::Profile as EchoProfile;
pub(crate) use state::HostState;
pub(crate) use state::WireRequest;
use state::{Received, Write};

/// Fixed storage bound, independent of run length. An overloaded adapter
/// reports a violation rather than silently losing input or growing forever.
const INPUT_BOUND: usize = 4096;

impl Sim {
    /// Host diagnostics for a powered node, or `None` for octet mode.
    #[must_use]
    pub fn host_stats(&self, station: u8) -> Option<&HostStats> {
        self.master(station).host.as_ref().map(|host| &host.stats)
    }

    pub(super) fn host_character(&mut self, node: usize, character: Character) {
        self.host_character_at(node, character, self.now);
    }

    fn host_character_at(
        &mut self,
        node: usize,
        character: Character,
        wire_end: bacnet_mstp_core::Instant,
    ) {
        let now = self.now;
        let master = self.master_mut(node).expect("master");
        let spec = master.spec.adapter.host.expect("host mode");
        let host = master.host.as_mut().expect("powered host");
        if host.input.len() == INPUT_BOUND {
            self.violation(
                ViolationKind::EventGrowth,
                "host input exceeded 4096 characters".into(),
            );
            return;
        }
        host.input.push_back(Received {
            character,
            wire_end,
        });
        host.ready.get_or_insert(now + spec.receive_latency);
        host.stats.max_buffered = host.stats.max_buffered.max(host.input.len());
    }

    pub(super) fn host_started(&mut self, node: usize, tx: TxId, id: bacnet_mstp_core::TransmitId) {
        let now = self.now;
        if self.master_mut(node).expect("master").host.is_none() {
            return;
        }
        // Reuse the line's captured schedule; do not predict a second,
        // potentially contiguous, end for the host's software completion.
        let wire_end = self.line.owner(tx).expect("physical owner").data_end;
        let frame = self.line.get(tx).expect("live frame").octets.clone();
        let held: Vec<_> = self
            .master_mut(node)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .held_at
            .iter()
            .copied()
            .collect();
        for wire_end in held {
            self.receiver_filter_drop(node, wire_end);
        }
        let master = self.master_mut(node).expect("master");
        let Some(host) = master.host.as_mut() else {
            return;
        };
        let spec = master.spec.adapter.host.expect("host mode");
        if host.filter.confirmed_start() > 0 {
            host.stats.echo_fragments_dropped += 1;
        }
        host.held_at.clear();
        host.write = Some(Write {
            id,
            incarnation: master.incarnation,
            start: now,
            wire_end,
            watchdog: now + master.spec.config.transmit_watchdog,
            ended: wire_end.max(now + spec.drain_duration),
            frame,
            hunting: false,
        });
        let profile = EchoProfile {
            incarnation: master.incarnation,
            delay: spec.echo_return_delay,
        };
        self.meta
            .get_mut(&tx)
            .expect("physical operation")
            .echo_profile = Some(profile);
    }

    fn host_complete(&mut self, node: usize) {
        let now = self.now;
        let station = self.nodes[node].station;
        let master = self.master_mut(node).expect("master");
        let host = master.host.as_mut().expect("host");
        let Some(write) = host.write.take() else {
            return;
        };
        host.stats.last_completion = Some(write.ended);
        host.completion_reported = Some(now);
        let id = master.transmitting.take();
        self.report.host_transmits.push(HostTransmitTiming {
            station,
            incarnation: write.incarnation,
            transmit_id: write.id,
            start: write.start,
            watchdog: write.watchdog,
            data_end: write.wire_end,
            completion: write.ended,
            notification: now,
        });
        if let Some(id) = id {
            let _ = self.master_input(node, Input::Complete(id, write.ended));
        }
    }

    /// Receive readiness sorts ahead of deadlines and transmit admission.
    /// During a write, reads wait for the later of drain and wire completion.
    pub(super) fn host_service(&mut self, node: usize) {
        let now = self.now;
        let master = self.master_mut(node).expect("master");
        let spec = master.spec.adapter.host.expect("host mode");
        let host = master.host.as_mut().expect("host");
        if let Some(write) = &mut host.write
            && !write.hunting
        {
            if now < write.ended {
                return;
            }
            if let Some(grace) = spec.echo_grace {
                host.filter.start(&write.frame, write.ended, grace);
                write.hunting = true;
            } else {
                self.host_complete(node);
            }
        }
        // Once a write has returned, the main loop's biased select polls
        // ready application replies before serial input or timers. While
        // strip_echo still owns the write, this leaves decisions deferred:
        // that await must consume its read (and the whole chunk) first.
        self.host_decisions(node);
        self.host_read(node);
        self.host_timer(node);
        self.host_decisions(node);
        // The production main loop services overdue timers after each read,
        // even when another chunk is already available. Input wins the tie,
        // but a continuously readable port cannot starve core deadlines.
        let master = self.master_mut(node).expect("master");
        if master.transmitting.is_none() && master.fire_at.is_some_and(|at| at <= now) {
            self.service_deadline(node);
        }
        self.host_check_receipts(node);
    }

    fn host_read(&mut self, node: usize) -> bool {
        let now = self.now;
        let master = self.master_mut(node).expect("master");
        let spec = master.spec.adapter.host.expect("host mode");
        let host = master.host.as_ref().expect("host");
        if !host.ready.is_some_and(|ready| ready <= now) {
            return false;
        }
        let hunting = host.write.is_some();
        if !hunting {
            self.host_timer(node);
        }
        let size = if spec.random_chunks {
            self.chunks.range(1, spec.max_chunk as u64) as usize
        } else {
            spec.max_chunk
        };
        let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
        let count = size.min(host.input.len());
        let characters: Vec<_> = host.input.drain(..count).collect();
        #[cfg(test)]
        if let Some(reads) = &mut host.read_capture {
            assert!(
                reads.len() < 64,
                "differential read capture exceeded its bound"
            );
            reads.push((now, characters.clone()));
        }
        host.ready = (!host.input.is_empty()).then_some(now);
        host.stats.reads += 1;
        host.stats.characters += count as u64;
        let station = self.nodes[node].station;
        self.note(
            Who::Station(station),
            TraceKind::Note(format!("host read {count} characters")),
        );
        for received in characters {
            match received.character {
                Character::Garbled => {
                    self.host_complete(node);
                    self.host_feed(node, Input::Garbled, received.wire_end);
                }
                Character::Clean(octet) => {
                    let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
                    let pushed = host.filter.push(octet, now);
                    let mut dropped: Vec<_> = host.held_at.drain(..pushed.stale).collect();
                    let mut passed: Vec<_> = host
                        .filter
                        .held(pushed.release)
                        .iter()
                        .copied()
                        .zip(host.held_at.drain(..pushed.release))
                        .collect();
                    if pushed.pass {
                        passed.push((octet, received.wire_end));
                    } else if pushed.echo {
                        dropped.extend(host.held_at.drain(..));
                        dropped.push(received.wire_end);
                    } else {
                        host.held_at.push_back(received.wire_end);
                    }
                    host.stats.echo_fragments_dropped += u32::from(pushed.stale > 0);
                    host.stats.echo_mismatches += u32::from(pushed.first_mismatch);
                    for wire_end in dropped {
                        self.receiver_filter_drop(node, wire_end);
                    }
                    if pushed.echo || pushed.passes() {
                        self.host_complete(node);
                    }
                    for (octet, wire_end) in passed {
                        self.host_feed(node, Input::Octet(octet), wire_end);
                    }
                }
            }
        }
        self.host_check_contention(node);
        true
    }

    fn host_feed(&mut self, node: usize, input: Input, wire_end: bacnet_mstp_core::Instant) {
        let now = self.now;
        let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
        host.activity = Some(now);
        host.receiving = Some(wire_end);
        let _ = self.receiver_input(node, input, wire_end);
        self.master_mut(node)
            .unwrap()
            .host
            .as_mut()
            .unwrap()
            .receiving = None;
    }

    fn host_timer(&mut self, node: usize) {
        let now = self.now;
        let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
        let due = host.filter.deadline().is_some_and(|end| end <= now);
        let ended = host.filter.timer(now);
        let dropped: Vec<_> = host.held_at.drain(..ended.stale).collect();
        let passed: Vec<_> = host
            .filter
            .held(ended.release)
            .iter()
            .copied()
            .zip(host.held_at.drain(..ended.release))
            .collect();
        host.stats.echo_missing += u32::from(ended.missing);
        host.stats.echo_fragments_dropped += u32::from(ended.stale > 0);
        for wire_end in dropped {
            self.receiver_filter_drop(node, wire_end);
        }
        if due {
            self.host_complete(node);
        }
        for (octet, wire_end) in passed {
            self.host_feed(node, Input::Octet(octet), wire_end);
        }
        self.host_check_contention(node);
    }

    /// Like production's bounded nonblocking drain, only reads already ready
    /// are admitted. Bytes not yet available cannot be known by this adapter.
    pub(super) fn host_before_start(&mut self, node: usize) {
        if self.master_mut(node).unwrap().host.is_none() {
            return;
        }
        for _ in 0..8 {
            if !self.host_read(node) || self.master_mut(node).unwrap().pending.is_none() {
                break;
            }
        }
        let now = self.now;
        let host = self.master_mut(node).unwrap().host.as_mut().unwrap();
        let ended = host.filter.before_start(now);
        let dropped: Vec<_> = host.held_at.drain(..ended.stale).collect();
        let passed: Vec<_> = host
            .filter
            .held(ended.release)
            .iter()
            .copied()
            .zip(host.held_at.drain(..ended.release))
            .collect();
        host.stats.echo_fragments_dropped += u32::from(ended.stale > 0);
        for wire_end in dropped {
            self.receiver_filter_drop(node, wire_end);
        }
        for (octet, wire_end) in passed {
            self.host_feed(node, Input::Octet(octet), wire_end);
        }
        self.host_check_contention(node);
    }

    fn host_check_contention(&mut self, node: usize) {
        if !self.receiver_has_pending(node) {
            return;
        }
        let host = self.master_mut(node).unwrap().host.as_ref().unwrap();
        let retained: Vec<_> = host
            .input
            .iter()
            .map(|r| r.wire_end)
            .chain(host.held_at.iter().copied())
            .collect();
        self.receiver_check_dropped(node, &retained);
    }

    pub(super) fn host_expect_frame(&mut self, node: usize, before: u32, takes: bool) {
        let now = self.now;
        let master = self.master_mut(node).unwrap();
        let spec = master.spec.adapter.host.unwrap();
        let host = master.host.as_mut().unwrap();
        let previous = host
            .expected
            .back()
            .map_or(before, |(_, expected)| (*expected).max(before));
        let mut at = now + spec.receive_latency;
        if let Some(write) = &host.write {
            at = at.max(write.ended + spec.echo_grace.unwrap_or_default());
        }
        // At least 8 wire octets per frame: a full serial buffer cannot hold
        // more than this many complete frames waiting for delivery.
        if host.expected.len() == INPUT_BOUND / 8 + 1 {
            self.violation(
                ViolationKind::EventGrowth,
                "host frame receipts exceeded bound".into(),
            );
            return;
        }
        host.expected.push_back((at, previous + u32::from(takes)));
    }

    fn host_check_receipts(&mut self, node: usize) {
        let now = self.now;
        let station = self.nodes[node].station;
        let master = self.master_mut(node).unwrap();
        let count = master.core.as_ref().unwrap().counters().frames_received;
        let host = master.host.as_mut().unwrap();
        // Multiple ready chunks are processed at the same instant before
        // deciding a receipt is missing.
        if host.ready.is_some_and(|at| at <= now) {
            return;
        }
        let mut missed = Vec::new();
        while let Some(&(at, expected)) = host.expected.front() {
            if at > now {
                break;
            }
            host.expected.pop_front();
            if count < expected {
                missed.push(expected);
            }
        }
        for expected in missed {
            self.violation(ViolationKind::ListenerMissedFrame,
                format!("host station {station} counted {count}, expected at least {expected} after read delivery"));
        }
    }
}

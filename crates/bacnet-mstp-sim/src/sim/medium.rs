//! Line events: octets going out, characters arriving, transmissions ending.

use bacnet_mstp_codec::{DecodeEvent, FrameHeader};
use bacnet_mstp_core::{Duration, Instant};

use super::master::Input;
use super::{Event, NodeKind, Sim, TxMeta, describe, tie_of};
use crate::event::Class;
use crate::line::{Character, Driver, Next, Transmission, TxId};
use crate::report::WireFrame;
use crate::spec::{Corruption, ScriptSend, SeenFrame};
use crate::trace::{TraceKind, Who};

/// Bit times of silence after which a scripted station's receiver drops a
/// partial frame, the shortest Tframe_abort (Clause 9.5.3).
const SCRIPTED_FRAME_ABORT_BITS: u64 = 60;

fn who(driver: Driver) -> Who {
    match driver {
        Driver::Station(station) => Who::Station(station),
        Driver::Noise(_) => Who::Noise,
    }
}

impl Sim {
    /// Register a transmission and the facts the checks need about it. A
    /// master's frame carries when its core handed it over and whether it is
    /// a token-generation poll. The caller starts its first octet.
    pub(super) fn begin_transmission(
        &mut self,
        driver: Driver,
        octets: Vec<u8>,
        corruption: Option<(usize, Corruption)>,
        dispatch: Option<(Instant, bool)>,
        post_drive: Duration,
        inter_octet_gap: Duration,
    ) -> TxId {
        let now = self.now;
        self.note(who(driver), TraceKind::FrameStart(describe(&octets)));
        let listeners = if self.perturbed {
            Vec::new()
        } else {
            self.listening_masters(driver)
        };
        let cut_after = self.take_cut_off(driver);
        let timing = self.timing_of(driver);
        let tx = self.line.begin(
            timing,
            driver,
            octets,
            now,
            corruption,
            post_drive,
            inter_octet_gap,
        );
        self.meta.insert(
            tx,
            TxMeta {
                echo_profile: None,
                transmit_id: None,
                dispatched: dispatch.map(|(at, _)| at),
                generation: dispatch.is_some_and(|(_, generation)| generation),
                corrupted: corruption.is_some(),
                cut_after,
                listeners,
                collided_with: Vec::new(),
            },
        );
        if corruption.is_some() {
            self.perturb("an injected corruption");
        }
        tx
    }

    /// The character grid `driver` sends on: a master's UART at its own
    /// rate error, anything else at the line's nominal rate.
    fn timing_of(&self, driver: Driver) -> crate::WireTiming {
        let Driver::Station(station) = driver else {
            return self.timing;
        };
        self.nodes
            .iter()
            .find(|node| node.station == station)
            .and_then(|node| match &node.kind {
                NodeKind::Master(master) => Some(master.spec.skew.baud_ppm),
                NodeKind::Scripted(_) => None,
            })
            .map_or(self.timing, |ppm| self.timing.with_rate_error(ppm))
    }

    /// Masters other than the driver with their receivers on, and how many
    /// valid frames each has counted.
    fn listening_masters(&self, driver: Driver) -> Vec<(usize, u32)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.listening_since.is_some() && driver != Driver::Station(node.station)
            })
            .filter_map(|(index, node)| match &node.kind {
                NodeKind::Master(master) => master
                    .core
                    .as_ref()
                    .map(|core| (index, core.counters().frames_received)),
                NodeKind::Scripted(_) => None,
            })
            .collect()
    }

    /// Put octet `index` of `tx` on the line now.
    pub(super) fn octet_start(&mut self, tx: TxId, index: usize) {
        let Some(transmission) = self.line.get(tx) else {
            return;
        };
        let driver = transmission.driver;
        let byte = transmission.octets[index];
        if self
            .meta
            .get(&tx)
            .and_then(|meta| meta.cut_after)
            .is_some_and(|cut| index >= cut)
        {
            if let Driver::Station(station) = driver {
                self.power_off(self.index_of(station));
            }
            return;
        }
        self.note(who(driver), TraceKind::Octet { index, byte });
        let outcome = self.line.start_octet(tx, index, self.now);
        for other in outcome.overlaps {
            self.on_overlap(tx, driver, other);
        }
        if let Some(end) = outcome.window_end {
            self.push(end, Class::WindowEnd, 400u16, Event::WindowEnd);
        }
        match self.line.next_after(tx, index) {
            Some(Next::Octet { at, index }) => {
                self.push(
                    at,
                    Class::OctetStart,
                    tie_of(driver),
                    Event::OctetStart { tx, index },
                );
            }
            Some(Next::End { at }) => {
                self.push(at, Class::TxEnd, tie_of(driver), Event::TxEnd { tx });
            }
            None => {}
        }
    }

    /// A receiver character ends: hand it to every node listening.
    pub(super) fn window_end(&mut self) {
        let now = self.now;
        let Some(outcome) = self.line.end_window(now) else {
            return;
        };
        if let Some(end) = outcome.reopened {
            self.push(end, Class::WindowEnd, 400u16, Event::WindowEnd);
        }
        if outcome.character == Character::Garbled {
            self.note(Who::Line, TraceKind::Garbled);
        }
        for node in 0..self.nodes.len() {
            let own = self.nodes[node].driving.filter(|tx| {
                if self.nodes[node].receiver.policy == crate::ContentionPolicy::FramingError {
                    outcome.opener.is_some_and(|(opener, _)| opener == *tx)
                } else {
                    outcome.contributors.iter().any(|c| c.tx == *tx)
                }
            });
            let eligible = match self.nodes[node].listening_since {
                // A receiver switched on part way through hears a fragment.
                Some(since) => since <= outcome.start,
                None => {
                    let echo = matches!(&self.nodes[node].kind, NodeKind::Master(master)
                        if master.spec.adapter.echo && master.core.is_some());
                    if !(echo && own.is_some()) {
                        continue;
                    }
                    outcome
                        .contributors
                        .iter()
                        .any(|c| Some(c.tx) == own && c.aligned)
                }
            };
            let receiver = &mut self.nodes[node].receiver;
            let (character, contributor) = if eligible {
                crate::contention::resolve(receiver.policy, &outcome, &mut receiver.rng)
            } else {
                (Character::Garbled, None)
            };
            self.observe_contention(node, &outcome, character, contributor);
            match &self.nodes[node].kind {
                NodeKind::Master(master) if master.host.is_some() => {
                    if self.nodes[node].listening_since.is_none() {
                        let tx = own.expect("an own echo observation");
                        self.host_local_echo(node, tx, character);
                    } else {
                        self.host_character(node, character);
                    }
                }
                NodeKind::Master(_) => {
                    let input = match character {
                        Character::Clean(byte) => Input::Octet(byte),
                        Character::Garbled => Input::Garbled,
                    };
                    let _ = self.receiver_input(node, input, now);
                }
                NodeKind::Scripted(_) => self.scripted_receive(node, character),
            }
        }
    }

    /// A transmission's final stop bit has gone out.
    pub(super) fn tx_end(&mut self, tx: TxId) {
        let Some(transmission) = self.line.finish(tx) else {
            return;
        };
        let driver = transmission.driver;
        self.record_frame(tx, transmission);
        let release = self.line.owner(tx).expect("a physical driver").release;
        if release == self.now {
            self.release_driver(tx, crate::DriverReleaseCause::Completed);
        } else {
            self.push(
                release,
                Class::DriverRelease,
                tie_of(driver),
                Event::DriverRelease { tx },
            );
        }
        if let Driver::Station(station) = driver {
            let node = self.index_of(station);
            self.master_tx_done(node);
        }
    }

    /// Log a finished or cut-off transmission and check it.
    pub(super) fn record_frame(&mut self, tx: TxId, transmission: Transmission) {
        let now = self.now;
        let meta = self.meta.get(&tx).cloned().unwrap_or_default();
        let decoded = super::decode_whole(&transmission.octets);
        let frame = WireFrame {
            driver: transmission.driver,
            start: transmission.start,
            end: now,
            header: decoded.as_ref().map(|(header, _)| *header),
            data: decoded.map(|(_, data)| data).unwrap_or_default(),
            octets: transmission.octets,
            collided: transmission.collided,
            corrupted: meta.corrupted,
            truncated: transmission.truncated,
            generation: meta.generation,
        };
        let mut flags = String::new();
        for (flag, text) in [
            (frame.collided, " collided"),
            (frame.corrupted, " corrupted"),
            (frame.truncated, " truncated"),
        ] {
            if flag {
                flags.push_str(text);
            }
        }
        self.note(
            who(frame.driver),
            TraceKind::FrameEnd(format!("{}{flags}", describe(&frame.octets))),
        );
        self.check_frame(&frame, &meta.listeners);
        self.frames.push(frame);
    }

    /// A scripted station's receiver: a codec stream decoder with the
    /// shortest frame-abort timeout.
    fn scripted_receive(&mut self, node: usize, character: Character) {
        let now = self.now;
        let abort = self.timing.bits(SCRIPTED_FRAME_ABORT_BITS);
        let NodeKind::Scripted(scripted) = &mut self.nodes[node].kind else {
            return;
        };
        if scripted
            .last_character
            .is_some_and(|last| now - last > abort)
        {
            scripted.decoder.reset();
        }
        scripted.last_character = Some(now);
        let mut seen = Vec::new();
        match character {
            Character::Garbled => scripted.decoder.reset(),
            Character::Clean(byte) => {
                let octet = [byte];
                let mut input: &[u8] = &octet;
                loop {
                    match scripted.decoder.push_slice(input) {
                        DecodeEvent::NeedMore => break,
                        DecodeEvent::Frame { frame, .. } => seen.push(SeenFrame {
                            at: now,
                            frame_type: frame.header.frame_type,
                            destination: frame.header.destination,
                            source: frame.header.source,
                            data: frame.data.to_vec(),
                        }),
                        _ => {}
                    }
                    input = &[];
                }
            }
        }
        let mut sends = Vec::new();
        for frame in &seen {
            scripted.script.on_frame(frame, &mut sends);
        }
        let station = self.nodes[node].station;
        for send in sends {
            self.push(
                now + send.after,
                Class::Scheduled,
                station,
                Event::ScriptSend { node, send },
            );
        }
    }

    /// A scripted station puts its octets on the line.
    pub(super) fn script_send(&mut self, node: usize, send: ScriptSend) {
        self.timing.validate_post_drive(send.post_drive);
        self.timing.validate_inter_octet_gap(send.inter_octet_gap);
        let station = self.nodes[node].station;
        if self.nodes[node].listening_since.is_none() {
            self.note(
                Who::Station(station),
                TraceKind::Note("scripted send dropped: busy or unpowered".into()),
            );
            return;
        }
        if send.malformed {
            self.perturb("a deliberately malformed scripted frame");
        }
        let tx = self.begin_transmission(
            Driver::Station(station),
            send.octets,
            None,
            None,
            send.post_drive,
            send.inter_octet_gap,
        );
        self.nodes[node].driving = Some(tx);
        self.nodes[node].listening_since = None;
        self.octet_start(tx, 0);
    }

    /// Inject a noise burst now.
    pub(super) fn noise(&mut self, octets: Vec<u8>) {
        if octets.is_empty() {
            return;
        }
        self.perturb("injected noise");
        self.noise_bursts += 1;
        let tx = self.begin_transmission(
            Driver::Noise(self.noise_bursts),
            octets,
            None,
            None,
            Duration::ZERO,
            Duration::ZERO,
        );
        self.octet_start(tx, 0);
    }

    /// The header of the last frame on the line, if it was clean and nothing
    /// has happened since it ended.
    pub(super) fn last_clean_frame(&self) -> Option<(FrameHeader, Instant)> {
        let frame = self.frames.last()?;
        let header = frame.header?;
        (frame.is_clean() && self.line.last_activity() == Some(frame.end))
            .then_some((header, frame.end))
    }
}

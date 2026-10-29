//! Each step: what it feeds the core, and what the core must answer.

use bacnet_mstp_core::{
    MasterCore, QueueRefusal, QueueStorage, ReceiveError, ReplyDecision, RequestError, RequestId,
    TransmitError, TransmitFailure, TransmitId,
};

use super::Driver;
use crate::checker::Decided;
use crate::input::{
    Address, Decision, Echo, MAX_OCTETS, Path, Step, TransmitEvent, fill, priority,
};
use crate::model::{Seen, encode};

/// How often a deadline may be serviced at one instant before it counts as
/// stuck.
const SERVICES: usize = 16;

impl Driver<'_> {
    pub(super) fn step<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        step: &Step,
    ) {
        let octet_time = self.params().octet_time;
        let line_input = matches!(
            step,
            Step::Octets { .. }
                | Step::Frame { .. }
                | Step::Damaged { .. }
                | Step::Cut { .. }
                | Step::ReceiveError(_)
                | Step::InvalidFrame { .. }
                | Step::HostActivity { .. }
                | Step::Wait(_)
                | Step::Enqueue { .. }
                | Step::Regress { .. }
        );
        if self.lazy && !line_input {
            // Poll what line inputs left before any other call.
            self.drain(core);
        }
        match step {
            Step::Octets { octets, gap } => {
                for &octet in octets.iter().take(MAX_OCTETS) {
                    self.now += octet_time + 16 * u64::from(*gap);
                    self.octet(core, octet);
                }
            }
            Step::Frame { frame, path } => {
                let validated = matches!(path, Path::Validated { .. });
                let frame = self.builder().frame(frame, validated, false);
                match *path {
                    Path::Octets => self.wire(core, &encode(&frame), None),
                    Path::Gapped { at, micros } => {
                        self.wire(core, &encode(&frame), Some((usize::from(at), micros)));
                    }
                    Path::Validated { with_octets } => {
                        let octets = 10 + frame.data.len() as u64;
                        if with_octets {
                            self.now += octet_time;
                            self.line_input(
                                core,
                                |core, now| core.on_host_activity(octets as usize, now),
                                |line, now, _| line.host_activity(octets, now),
                            );
                        }
                        self.now += octets * octet_time;
                        self.validated(core, frame);
                    }
                }
            }
            Step::Damaged {
                frame,
                position,
                bit,
            } => {
                let frame = self.builder().frame(frame, false, true);
                let mut wire = encode(&frame);
                if wire.len() > 8 {
                    let at = 8 + usize::from(*position) % (wire.len() - 8);
                    wire[at] ^= 1 << (bit % 8);
                }
                self.wire(core, &wire, None);
            }
            Step::Cut { frame, at } => {
                let wire = encode(&self.builder().frame(frame, false, false));
                let cut = 1 + usize::from(*at) % (wire.len() - 1);
                self.wire(core, &wire[..cut], None);
            }
            Step::ReceiveError(fault) => {
                self.now += octet_time;
                let error = ReceiveError::from(*fault);
                self.line_input(
                    core,
                    |core, now| core.on_receive_error(error, now),
                    |line, now, heard| line.receive_error(now, heard),
                );
            }
            Step::InvalidFrame { after } => {
                self.now += u64::from(*after);
                self.line_input(
                    core,
                    |core, now| core.on_invalid_frame(now),
                    |line, now, heard| line.invalid(now, heard),
                );
            }
            Step::HostActivity { octets, after } => {
                self.now += u64::from(*after);
                let octets = *octets;
                self.line_input(
                    core,
                    |core, now| core.on_host_activity(usize::from(octets), now),
                    |line, now, _| line.host_activity(u64::from(octets), now),
                );
            }
            Step::Silence(micros) => {
                self.now += u64::from(*micros);
                self.checker.tick(self.now);
                core.on_silence(self.at());
                self.stamp = self.now;
                self.drain(core);
            }
            Step::Wait(micros) => self.now += u64::from(*micros),
            Step::Deadline {
                late,
                start_before_poll,
            } => {
                let due = core
                    .next_deadline()
                    .map_or(self.now, |due| due.as_micros().max(self.now));
                self.now = due + late.micros();
                self.deadline(core, *start_before_poll);
            }
            Step::Start { after } => self.start(core, u64::from(*after)),
            Step::Complete { after, echo } => self.complete(core, u64::from(*after), echo),
            Step::Abort { after, reason } => {
                let Some(tx) = &self.tx else {
                    return self.bogus(core, TransmitEvent::Abort, u32::from(*after));
                };
                let (id, started) = (tx.id, tx.started.is_some());
                self.now += u64::from(*after);
                self.checker.tick(self.now);
                let failure = TransmitFailure::from(*reason);
                core.on_transmit_aborted(id, failure, self.at());
                self.stamp = self.now;
                self.tx = None;
                if started {
                    self.checker.aborted(self.now);
                } else {
                    self.checker.withdrawn();
                }
                self.drain(core);
            }
            Step::Bogus { event, id } => self.bogus(core, *event, *id),
            Step::Enqueue {
                to,
                length,
                expecting,
                priority: raw,
                pattern,
                copies,
            } => {
                let max_npdu = self.params().max_npdu;
                let to = self.builder().resolve(*to, false, false);
                let length = if length & 0x8000 != 0 {
                    usize::from(length & 0x7FFF) % (max_npdu + 17)
                } else {
                    usize::from(length % 24)
                };
                let mut npdu = fill(length, *pattern);
                if length >= 2 {
                    npdu[0] = 0x01;
                    npdu[1] = if *expecting { 0x04 } else { 0x00 };
                }
                for _ in 0..=copies % 4 {
                    let result = core.enqueue_npdu(to, &npdu, priority(*raw), self.at());
                    let fine = if length > max_npdu {
                        result == Err(QueueRefusal::Oversize)
                    } else {
                        matches!(result, Ok(()) | Err(QueueRefusal::Full))
                    };
                    if !fine {
                        self.fail(&format!(
                            "an NPDU of {length} octets for {to} was queued with {result:?}"
                        ));
                    }
                }
            }
            Step::Decide {
                pick,
                decision,
                after,
            } => self.decide(core, *pick, *decision, u64::from(*after)),
            Step::Drain => self.drain(core),
            Step::Run { rounds } => {
                for _ in 0..=rounds % 32 {
                    match &self.tx {
                        Some(tx) if tx.started.is_none() => self.start(core, 0),
                        Some(_) => self.complete(core, 0, &Echo::Silent),
                        None => {
                            let Some(due) = core.next_deadline() else {
                                return;
                            };
                            self.now = self.now.max(due.as_micros());
                            self.deadline(core, false);
                        }
                    }
                }
            }
            Step::Regress { back, event } => self.regress(core, *back, *event),
        }
    }

    /// Octets back to back, with an optional gap before one of them.
    fn wire<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        octets: &[u8],
        gap: Option<(usize, u32)>,
    ) {
        let octet_time = self.params().octet_time;
        for (index, &octet) in octets.iter().enumerate() {
            self.now += octet_time;
            if let Some((at, micros)) = gap
                && index == at % octets.len()
            {
                self.now += u64::from(micros);
            }
            self.octet(core, octet);
        }
    }

    fn deadline<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        start_before_poll: bool,
    ) {
        self.checker.tick(self.now);
        core.on_deadline(self.at());
        self.stamp = self.now;
        if start_before_poll {
            let held = self.tx.as_ref().map(|tx| tx.id);
            if let Some(id) = core.snapshot().transmit_id.filter(|&id| Some(id) != held) {
                let result = core.on_transmit_started(id, self.at()).map(<[u8]>::len);
                if result != Err(TransmitError::NotPolled) {
                    self.fail(&format!(
                        "a start before the frame was polled got {result:?}"
                    ));
                }
            }
        }
        self.drain(core);
        self.checker.answers_due(self.now, None);
        for _ in 0..SERVICES {
            match core.next_deadline() {
                Some(due) if due.as_micros() <= self.now => {
                    self.checker.tick(self.now);
                    core.on_deadline(self.at());
                    self.stamp = self.now;
                    self.drain(core);
                    self.checker.answers_due(self.now, None);
                }
                _ => return,
            }
        }
        self.fail("a deadline stays due however often it is serviced");
    }

    fn start<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        after: u64,
    ) {
        let Some(tx) = &self.tx else {
            return self.bogus(core, TransmitEvent::Start, after as u32);
        };
        let id = tx.id;
        if tx.started.is_some() {
            let before = core.snapshot();
            let result = core.on_transmit_started(id, self.at()).map(<[u8]>::len);
            self.stamp = self.now;
            if result != Err(TransmitError::AlreadyStarted) || core.snapshot() != before {
                self.fail(&format!("a second start got {result:?}"));
            }
            return;
        }
        self.now += after;
        self.checker.tick(self.now);
        let result = core.on_transmit_started(id, self.at());
        self.stamp = self.now;
        match result {
            Ok(frame) => {
                // Only a confirmed start hands the octets over: exactly the
                // frame the action named, from the action buffers.
                self.check_span(frame, self.frame, "a started transmit");
                if self.tx.as_ref().is_none_or(|tx| tx.octets != frame) {
                    self.fail("the start handed over other octets than the action named");
                }
                self.checker.started(self.now);
                if let Some(tx) = &mut self.tx {
                    tx.started = Some(self.now);
                }
            }
            Err(TransmitError::Deferred) => {
                self.checker.deferred(self.now);
                self.tx = None;
            }
            Err(TransmitError::Expired) => {
                self.checker.expired(self.now);
                self.tx = None;
            }
            Err(error) => self.fail(&format!("the start of a polled frame got {error:?}")),
        }
        self.drain(core);
    }

    fn complete<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        after: u64,
        echo: &Echo,
    ) {
        let Some(tx) = &self.tx else {
            return self.bogus(core, TransmitEvent::Complete, after as u32);
        };
        let id = tx.id;
        let Some(start) = tx.started else {
            let before = (core.snapshot(), core.counters().adapter_errors);
            core.on_transmit_complete(id, self.at());
            self.stamp = self.now;
            if (core.snapshot(), core.counters().adapter_errors) != (before.0, before.1 + 1) {
                self.fail("completing an unstarted frame was not ignored and counted");
            }
            return;
        };
        let octets = tx.octets.clone();
        let octet_time = self.params().octet_time;
        match echo {
            Echo::Silent => {}
            Echo::Octets => {
                for (index, &octet) in octets.iter().enumerate() {
                    self.now = self.now.max(start + (index as u64 + 1) * octet_time);
                    self.octet(core, octet);
                }
            }
            Echo::Frame => {
                self.now = self.now.max(start + octets.len() as u64 * octet_time);
                let frame = Seen::decode(&octets).unwrap_or_else(|error| self.fail(&error));
                self.validated(core, frame);
            }
            Echo::Noise(noise) => {
                for &octet in noise.iter().take(MAX_OCTETS) {
                    self.now += octet_time;
                    self.octet(core, octet);
                }
            }
        }
        self.now = self.now.max(start + octets.len() as u64 * octet_time) + after;
        self.checker.tick(self.now);
        core.on_transmit_complete(id, self.at());
        self.stamp = self.now;
        self.tx = None;
        self.checker.completed(self.now);
        self.drain(core);
    }

    /// A transmit event for an id the adapter doesn't hold, which the core
    /// must ignore, or refuse as stale for a start, without changing
    /// anything.
    fn bogus<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        event: TransmitEvent,
        id: u32,
    ) {
        let taken = [
            self.tx.as_ref().map(|tx| tx.id),
            core.snapshot().transmit_id,
        ];
        let mut id = TransmitId::new(id);
        while taken.contains(&Some(id)) {
            id = TransmitId::new(id.generation().wrapping_add(1));
        }
        let before = (core.snapshot(), core.counters());
        let at = self.at();
        let result = match event {
            TransmitEvent::Start => core.on_transmit_started(id, at).map(drop),
            TransmitEvent::Complete => {
                core.on_transmit_complete(id, at);
                Err(TransmitError::Stale)
            }
            TransmitEvent::Abort => {
                core.on_transmit_aborted(id, TransmitFailure::Io, at);
                Err(TransmitError::Stale)
            }
        };
        self.stamp = self.now;
        if result != Err(TransmitError::Stale) || (core.snapshot(), core.counters()) != before {
            self.fail(&format!(
                "{event:?} for an id nobody holds got {result:?}, or changed the core"
            ));
        }
    }

    fn decide<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        pick: u8,
        decision: Decision,
        after: u64,
    ) {
        let issued = self.requests.len();
        let id = match pick % 4 {
            0 | 1 if issued > 0 => Some(self.requests[issued - 1]),
            2 if issued > 1 => Some(self.requests[issued - 2]),
            _ => None,
        };
        let request = id.unwrap_or(RequestId::new(0xF000_0000 | u32::from(pick)));
        self.now += 8 * after;
        self.checker.tick(self.now);
        let max_npdu = self.params().max_npdu;
        let data = match decision {
            Decision::Data { length, pattern } => fill(
                if length & 0x8000 != 0 {
                    usize::from(length & 0x7FFF) % (max_npdu + 17)
                } else {
                    usize::from(length % 16)
                },
                pattern,
            ),
            _ => Vec::new(),
        };
        let reply = match decision {
            Decision::Data { .. } => ReplyDecision::Data(&data),
            Decision::Postponed | Decision::Cancelled => ReplyDecision::Postponed,
            Decision::Abandoned => ReplyDecision::Abandoned,
        };
        let result = core.complete_request(request, reply, self.at());
        self.stamp = self.now;
        let data_reply = matches!(decision, Decision::Data { .. });
        match result {
            Ok(()) if id.is_none() => self.fail("a decision for a request nobody made was taken"),
            Ok(()) if data_reply && data.len() > max_npdu => {
                self.fail(&format!("a reply of {} octets was taken", data.len()));
            }
            Ok(()) => self.checker.decided(
                request,
                match decision {
                    Decision::Data { .. } => Decided::Data(&data),
                    Decision::Postponed | Decision::Cancelled => Decided::Postponed,
                    Decision::Abandoned => Decided::Abandoned,
                },
            ),
            Err(RequestError::Oversize { .. }) if data.len() <= max_npdu => {
                self.fail(&format!("a reply of {} octets was refused", data.len()));
            }
            Err(RequestError::Stale | RequestError::NoPending) => {}
            Err(error) if id.is_none() => {
                self.fail(&format!(
                    "a decision for a request nobody made got {error:?}"
                ));
            }
            _ => {}
        }
        self.drain(core);
        if result.is_ok() {
            // A decision the core took makes its answer, and sends it at
            // once if the line is quiet.
            self.checker.answers_due(self.now, Some(request));
        }
    }

    /// A call stamped before the last one. The core takes a `now` earlier
    /// than the latest it was given as that latest one, so the call must do
    /// exactly what the same call stamped then would: the driver runs it as
    /// that call, which the checker models at the latest time, and only the
    /// core is told the earlier one. (Until #1684 the core refused such a
    /// call, and this step checked that it changed nothing.)
    ///
    /// A silence first brings the core's latest time up to the driver's, so
    /// the two agree on when that is.
    fn regress<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        back: u32,
        event: u8,
    ) {
        self.step(core, &Step::Silence(0));
        if self.stamp == 0 {
            return;
        }
        self.skew = 1 + u64::from(back) % self.stamp;
        match event % 10 {
            0 => self.octet(core, 0x55),
            1 => self.line_input(
                core,
                |core, now| core.on_receive_error(ReceiveError::Framing, now),
                |line, now, heard| line.receive_error(now, heard),
            ),
            2 => match core.next_deadline() {
                Some(due) if due.as_micros() <= self.now => self.deadline(core, false),
                _ => {
                    // Nothing is due yet, so the call changes nothing.
                    let before = (core.snapshot(), core.counters());
                    core.on_deadline(self.at());
                    if (core.snapshot(), core.counters()) != before {
                        self.fail("a deadline serviced before it fell due changed the core");
                    }
                }
            },
            3 => self.step(core, &Step::Silence(0)),
            4 => self.line_input(
                core,
                |core, now| core.on_host_activity(3, now),
                |line, now, _| line.host_activity(3, now),
            ),
            5 => self.line_input(
                core,
                |core, now| core.on_invalid_frame(now),
                |line, now, heard| line.invalid(now, heard),
            ),
            6 => self.decide(core, 0, Decision::Postponed, 0),
            7 => self.start(core, 0),
            8 => self.validated(core, self.token_frame()),
            _ => self.step(
                core,
                &Step::Enqueue {
                    to: Address::Peer,
                    length: 2,
                    expecting: false,
                    priority: 0,
                    pattern: 0,
                    copies: 0,
                },
            ),
        }
        self.skew = 0;
    }
}

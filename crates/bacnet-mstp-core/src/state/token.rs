//! Token ownership, maintenance polling, and queue scheduling.

use super::*;

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    pub(super) fn enter_no_token(&mut self, now: Instant) {
        self.set_state(MasterState::NoToken);
        self.retry_token_count = 0;
        // Clause 9.5.6.2 (LostToken): SawFrame counts events from here.
        self.event_count = 0;
        self.generate_token_if_in_slot(now)
    }

    /// Generate a token in this station's slot of the silence (Clause
    /// 9.5.6.7), or in a later window if the deadline came after the slot
    /// closed: the local recovery policy described on [`MasterCore`], counted
    /// in [`MstpCounters::missed_generation_slots`].
    pub(super) fn generate_token_if_in_slot(&mut self, now: Instant) {
        let cycle = T_SLOT.saturating_mul(u32::from(self.config.max_master) + 1);
        let first =
            self.last_activity + T_NO_TOKEN + T_SLOT.saturating_mul(u32::from(self.config.station));
        let mut start = first;
        // The window that `now` falls in, or the next one: whole cycles after
        // the first. T_slot is 10 ms and validation caps Max_Master at 127, so
        // a cycle is at most 1.28 s; `checked_rem` takes one step per bit of
        // the cycles passed, with no 64-bit division.
        if now >= first + T_SLOT {
            if let Some(offset) = (now - first).checked_rem(cycle) {
                start = now - offset;
                if offset >= T_SLOT {
                    start += cycle;
                }
            }
        }
        if now < start {
            self.protocol_deadline = Some(start);
            return;
        }
        if start != first {
            self.counters.record_missed_generation_slot();
        }
        self.counters.record_token_regeneration();
        self.next_station = self.config.station;
        self.token_count = 0;
        self.poll_station = next_addr(self.config.station, self.config.max_master);
        self.queue_poll(Origin::NoToken, now)
    }

    pub(super) fn pass_token_timeout(&mut self, now: Instant) {
        if self.event_count > u32::from(N_MIN_OCTETS) {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + T_NO_TOKEN);
        } else if self.retry_token_count < N_RETRY_TOKEN {
            self.retry_token_count = self.retry_token_count.saturating_add(1);
            self.counters.record_token_retry();
            self.queue_token(self.next_station, Origin::PassToken, now);
        } else {
            let failed = self.next_station;
            self.next_station = self.config.station;
            self.poll_station = next_addr(failed, self.config.max_master);
            if self.poll_station == self.config.station {
                self.poll_station = next_addr(self.config.station, self.config.max_master);
            }
            self.retry_token_count = 0;
            self.token_count = 0;
            self.queue_poll(Origin::PassToken, now);
        }
    }

    pub(super) fn poll_timeout(&mut self, now: Instant) {
        if self.sole_master {
            self.frame_count = 0;
            self.set_state(MasterState::UseToken);
            return self.drive_token(now);
        }
        if self.next_station != self.config.station {
            self.retry_token_count = 0;
            return self.queue_token(self.next_station, Origin::Holder, now);
        }
        let next = next_addr(self.poll_station, self.config.max_master);
        if next == self.config.station {
            self.sole_master = true;
            self.frame_count = 0;
            self.set_state(MasterState::UseToken);
            return self.drive_token(now);
        }
        self.poll_station = next;
        self.queue_poll(Origin::NextPoll, now)
    }

    pub(super) fn drive_token(&mut self, now: Instant) {
        if self.active_transmit.is_some() || self.pending_send.is_some() {
            return;
        }
        let frame_limit = u16::from(self.config.max_info_frames)
            .saturating_mul(u16::from(self.config.multiple_frame_ratio))
            .div_ceil(100)
            .clamp(1, u16::from(self.config.max_info_frames)) as u8;
        if self.frame_count < frame_limit && self.dequeue_and_send(now) {
            return;
        }
        self.set_state(MasterState::DoneWithToken);
        self.done_with_token(now)
    }

    /// Send the next queued NPDU, if there is one: in a standard frame, or
    /// an extended one when it is longer than 501 octets (Clauses 9.3.9,
    /// 9.3.10), which only extended frames let into the queue.
    ///
    /// The frame is encoded straight from the queue's slot, which lends the
    /// NPDU, into the action buffers' frame buffer: nothing NPDU-sized goes
    /// on the stack.
    ///
    /// An NPDU that cannot go out (longer than this station sends, or than
    /// an `NPDU`-octet delivery slot holds) is dropped, counted as oversize,
    /// and the next one is tried, so it never holds the token. The attempts
    /// are bounded in case a [`QueueStorage`] fails to drop it.
    fn dequeue_and_send(&mut self, now: Instant) -> bool {
        self.queues.expire(now);
        for _ in 0..=self.queues.total_depth() {
            let Some(kind) = self.next_queue_kind(now) else {
                return false;
            };
            let item = match self.queues.dequeue_kind(kind, now, NPDU) {
                Ok(Some(item)) => item,
                Ok(None) => return false,
                // Longer than NPDU: the queue dropped and counted it.
                Err(_) => continue,
            };
            let data = item.npdu;
            if data.len() > NPDU || data.len() > self.config.max_npdu() {
                // The queue lent more octets than it was asked for, or holds
                // an NPDU this station doesn't send, which a queue fed only
                // through `enqueue_npdu` never does.
                self.record_oversize_npdu(kind);
                continue;
            }
            self.consecutive_broadcast = if kind == QueueKind::Broadcast {
                self.consecutive_broadcast.saturating_add(1)
            } else {
                0
            };
            self.frame_count = self.frame_count.saturating_add(1);
            let expecting = kind == QueueKind::Unicast && data.len() >= 2 && data[1] & 0x04 != 0;
            let extended = data.len() > MAX_STANDARD_MPDU_DATA;
            let (frame_type, post) = match (expecting, extended) {
                (true, false) => (
                    FrameType::BACNET_DATA_EXPECTING_REPLY,
                    PostTransmit::WaitForReply,
                ),
                (true, true) => (
                    FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
                    PostTransmit::WaitForReply,
                ),
                (false, false) => (
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                    PostTransmit::ContinueToken,
                ),
                (false, true) => (
                    FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
                    PostTransmit::ContinueToken,
                ),
            };
            // The lent NPDU borrows the queues, and the frame buffer is the
            // action buffers': the two never overlap.
            let frame = Frame::new(frame_type, item.destination, self.config.station, data);
            if !pending::encode(&mut self.actions, &mut self.counters, frame) {
                continue;
            }
            self.send_encoded(post, Origin::Holder, now);
            return true;
        }
        false
    }

    /// The queue to serve next under the broadcast/unicast ratio.
    fn next_queue_kind(&self, now: Instant) -> Option<QueueKind> {
        let has_u = self.queues.peek_kind(QueueKind::Unicast, now).is_some();
        let has_b = self.queues.peek_kind(QueueKind::Broadcast, now).is_some();
        match (has_u, has_b) {
            (false, false) => None,
            (true, false) => Some(QueueKind::Unicast),
            (false, true) => Some(QueueKind::Broadcast),
            (true, true) if self.consecutive_broadcast < self.config.broadcast_unicast_ratio => {
                Some(QueueKind::Broadcast)
            }
            (true, true) => Some(QueueKind::Unicast),
        }
    }

    fn done_with_token(&mut self, now: Instant) {
        loop {
            let next_ts = next_addr(self.config.station, self.config.max_master);
            let next_ps = next_addr(self.poll_station, self.config.max_master);
            if !self.sole_master && self.next_station == self.config.station {
                self.poll_station = next_ts;
                return self.queue_poll(Origin::Holder, now);
            }
            if self.token_count < N_POLL {
                self.token_count = self.token_count.saturating_add(1);
                if self.sole_master {
                    self.frame_count = 0;
                    self.set_state(MasterState::UseToken);
                    if self.dequeue_and_send(now) {
                        return;
                    }
                    continue;
                }
                self.retry_token_count = 0;
                return self.queue_token(self.next_station, Origin::Holder, now);
            }
            // The maintenance sweep ends (ResetMaintenancePFM, Clause
            // 9.5.6.5) when the address after PS is NS. A successor above
            // Max_Master, a station outside the master range that answered a
            // poll, is never that address, so the sweep also ends before it
            // would reach this station's own: every other address has been
            // polled by then.
            if next_ps == self.next_station || next_ps == self.config.station {
                self.poll_station = self.config.station;
                self.token_count = 1;
                if self.sole_master {
                    self.next_station = self.config.station;
                    self.poll_station = next_ts;
                    return self.queue_poll(Origin::Holder, now);
                }
                self.retry_token_count = 0;
                return self.queue_token(self.next_station, Origin::Holder, now);
            }
            self.poll_station = next_ps;
            return self.queue_poll(Origin::Holder, now);
        }
    }

    /// Pass the token to `destination` and enter PASS_TOKEN; a station that
    /// would pass to itself polls for a successor instead.
    pub(super) fn queue_token(&mut self, destination: u8, origin: Origin, now: Instant) {
        if destination == self.config.station {
            self.poll_station = next_addr(self.config.station, self.config.max_master);
            return self.queue_poll(origin, now);
        }
        self.queue_frame(
            FrameType::TOKEN,
            destination,
            &[],
            PostTransmit::PassToken,
            origin,
            now,
        );
        self.set_state(MasterState::PassToken);
        self.event_count = 0;
    }

    /// Poll `poll_station` for a master and enter POLL_FOR_MASTER.
    fn queue_poll(&mut self, origin: Origin, now: Instant) {
        self.queue_frame(
            FrameType::POLL_FOR_MASTER,
            self.poll_station,
            &[],
            PostTransmit::PollForMaster,
            origin,
            now,
        );
        self.set_state(MasterState::PollForMaster);
    }
}

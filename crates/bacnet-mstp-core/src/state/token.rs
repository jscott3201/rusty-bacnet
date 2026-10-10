//! Token ownership, maintenance polling, and queue scheduling.

use super::*;

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    pub(super) fn enter_no_token(&mut self, now: Instant) -> Result<(), CoreError> {
        self.set_state(MasterState::NoToken);
        self.retry_token_count = 0;
        self.generate_token_if_in_slot(now)
    }

    pub(super) fn generate_token_if_in_slot(&mut self, now: Instant) -> Result<(), CoreError> {
        let cycle = scale(self.config.t_slot, u64::from(self.config.max_master) + 1);
        let first = self.last_activity
            + self.config.t_no_token
            + scale(self.config.t_slot, u64::from(self.config.station));
        let mut start = first;
        if now >= start + self.config.t_slot && cycle != Duration::ZERO {
            let cycles = (now - start).as_micros() / cycle.as_micros();
            start += scale(cycle, cycles);
            if now >= start + self.config.t_slot {
                start += cycle;
            }
        }
        if now < start {
            self.protocol_deadline = Some(start);
            return Ok(());
        }
        self.counters.record_token_regeneration();
        self.next_station = self.config.station;
        self.token_count = 0;
        self.poll_station = next_addr(self.config.station, self.config.max_master);
        self.queue_frame(
            FrameType::PollForMaster,
            self.poll_station,
            &[],
            PostTransmit::PollForMaster,
            now,
        )?;
        self.set_state(MasterState::PollForMaster);
        Ok(())
    }

    pub(super) fn pass_token_timeout(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.event_count > u32::from(self.config.n_min_octets) {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
        } else if self.retry_token_count < self.config.n_retry_token {
            self.retry_token_count = self.retry_token_count.saturating_add(1);
            self.counters.record_token_retry();
            self.queue_token(self.next_station, now)?;
        } else {
            let failed = self.next_station;
            self.next_station = self.config.station;
            self.poll_station = next_addr(failed, self.config.max_master);
            if self.poll_station == self.config.station {
                self.poll_station = next_addr(self.config.station, self.config.max_master);
            }
            self.retry_token_count = 0;
            self.token_count = 0;
            self.queue_frame(
                FrameType::PollForMaster,
                self.poll_station,
                &[],
                PostTransmit::PollForMaster,
                now,
            )?;
            self.set_state(MasterState::PollForMaster);
        }
        Ok(())
    }

    pub(super) fn poll_timeout(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.sole_master {
            self.frame_count = 0;
            self.set_state(MasterState::UseToken);
            return self.drive_token(now);
        }
        if self.next_station != self.config.station {
            self.retry_token_count = 0;
            return self.queue_token(self.next_station, now);
        }
        let next = next_addr(self.poll_station, self.config.max_master);
        if next == self.config.station {
            self.sole_master = true;
            self.frame_count = 0;
            self.set_state(MasterState::UseToken);
            return self.drive_token(now);
        }
        self.poll_station = next;
        self.queue_frame(
            FrameType::PollForMaster,
            self.poll_station,
            &[],
            PostTransmit::PollForMaster,
            now,
        )
    }

    pub(super) fn drive_token(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.active_transmit.is_some() || self.pending_send.is_some() {
            return Ok(());
        }
        let frame_limit = u16::from(self.config.max_info_frames)
            .saturating_mul(u16::from(self.config.multiple_frame_ratio))
            .div_ceil(100)
            .clamp(1, u16::from(self.config.max_info_frames)) as u8;
        if self.frame_count < frame_limit && self.dequeue_and_send(now)? {
            return Ok(());
        }
        self.set_state(MasterState::DoneWithToken);
        self.done_with_token(now)
    }

    fn dequeue_and_send(&mut self, now: Instant) -> Result<bool, CoreError> {
        self.queues.expire(now.as_micros());
        let has_u = self
            .queues
            .peek_kind(QueueKind::Unicast, now.as_micros())
            .is_some();
        let has_b = self
            .queues
            .peek_kind(QueueKind::Broadcast, now.as_micros())
            .is_some();
        let kind = match (has_u, has_b) {
            (false, false) => return Ok(false),
            (true, false) => QueueKind::Unicast,
            (false, true) => QueueKind::Broadcast,
            (true, true) if self.consecutive_broadcast < self.config.broadcast_unicast_ratio => {
                QueueKind::Broadcast
            }
            (true, true) => QueueKind::Unicast,
        };
        let mut npdu = [0; MAX_STANDARD_MPDU_DATA];
        let Some(item) = self
            .queues
            .dequeue_kind_into(kind, now.as_micros(), &mut npdu)
            .map_err(|_| CoreError::ActionCapacity)?
        else {
            return Ok(false);
        };
        self.consecutive_broadcast = if kind == QueueKind::Broadcast {
            self.consecutive_broadcast.saturating_add(1)
        } else {
            0
        };
        self.frame_count = self.frame_count.saturating_add(1);
        let expecting = kind == QueueKind::Unicast && item.len >= 2 && npdu[1] & 0x04 != 0;
        let (frame_type, post) = if expecting {
            (
                FrameType::BACnetDataExpectingReply,
                PostTransmit::WaitForReply {
                    source: item.destination,
                },
            )
        } else {
            (
                FrameType::BACnetDataNotExpectingReply,
                PostTransmit::ContinueToken,
            )
        };
        self.queue_frame(frame_type, item.destination, &npdu[..item.len], post, now)?;
        Ok(true)
    }

    fn done_with_token(&mut self, now: Instant) -> Result<(), CoreError> {
        loop {
            let next_ts = next_addr(self.config.station, self.config.max_master);
            let next_ps = next_addr(self.poll_station, self.config.max_master);
            if !self.sole_master && self.next_station == self.config.station {
                self.poll_station = next_ts;
                self.queue_frame(
                    FrameType::PollForMaster,
                    self.poll_station,
                    &[],
                    PostTransmit::PollForMaster,
                    now,
                )?;
                self.set_state(MasterState::PollForMaster);
                return Ok(());
            }
            if self.token_count < self.config.n_poll {
                self.token_count = self.token_count.saturating_add(1);
                if self.sole_master {
                    self.frame_count = 0;
                    self.set_state(MasterState::UseToken);
                    if self.dequeue_and_send(now)? {
                        return Ok(());
                    }
                    continue;
                }
                self.retry_token_count = 0;
                return self.queue_token(self.next_station, now);
            }
            if next_ps == self.next_station {
                self.poll_station = self.config.station;
                self.token_count = 1;
                if self.sole_master {
                    self.next_station = self.config.station;
                    self.poll_station = next_ts;
                    self.queue_frame(
                        FrameType::PollForMaster,
                        self.poll_station,
                        &[],
                        PostTransmit::PollForMaster,
                        now,
                    )?;
                    self.set_state(MasterState::PollForMaster);
                    return Ok(());
                }
                self.retry_token_count = 0;
                return self.queue_token(self.next_station, now);
            }
            self.poll_station = next_ps;
            self.queue_frame(
                FrameType::PollForMaster,
                self.poll_station,
                &[],
                PostTransmit::PollForMaster,
                now,
            )?;
            self.set_state(MasterState::PollForMaster);
            return Ok(());
        }
    }

    pub(super) fn queue_token(&mut self, destination: u8, now: Instant) -> Result<(), CoreError> {
        if destination == self.config.station {
            self.poll_station = next_addr(self.config.station, self.config.max_master);
            self.queue_frame(
                FrameType::PollForMaster,
                self.poll_station,
                &[],
                PostTransmit::PollForMaster,
                now,
            )?;
            self.set_state(MasterState::PollForMaster);
            return Ok(());
        }
        self.queue_frame(
            FrameType::Token,
            destination,
            &[],
            PostTransmit::PassToken,
            now,
        )?;
        self.set_state(MasterState::PassToken);
        self.event_count = 0;
        Ok(())
    }
}

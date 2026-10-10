//! Encoded-frame deferral and transmit action dispatch.

use super::*;

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    pub(super) fn queue_frame(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        data: &[u8],
        post: PostTransmit,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.queue_frame_until(frame_type, destination, data, post, now, None)
    }

    pub(super) fn queue_frame_until(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        data: &[u8],
        post: PostTransmit,
        now: Instant,
        expires: Option<Instant>,
    ) -> Result<(), CoreError> {
        let mut encoded = [0; MAX_STANDARD_FRAME_LENGTH];
        let length = Frame::new(frame_type, destination, self.config.station, data)
            .encode_into(&mut encoded)?;
        let earliest = self.last_activity + self.config.t_turnaround;
        if now < earliest {
            self.pending_send = Some(PendingSend {
                length,
                frame: encoded,
                post,
                earliest,
                expires,
            });
            self.protocol_deadline = Some(earliest);
            return Ok(());
        }
        if expires.is_some_and(|deadline| now > deadline) {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        }
        self.dispatch_frame(&encoded[..length], post, expires, now)
    }

    pub(super) fn dispatch_pending_send(&mut self, now: Instant) -> Result<(), CoreError> {
        let pending = self.pending_send.ok_or(CoreError::ActionCapacity)?;
        if now < pending.earliest {
            self.protocol_deadline = Some(pending.earliest);
            return Ok(());
        }
        if pending.expires.is_some_and(|deadline| now > deadline) {
            self.pending_send = None;
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        }
        self.dispatch_frame(
            &pending.frame[..pending.length],
            pending.post,
            pending.expires,
            now,
        )?;
        self.pending_send = None;
        Ok(())
    }

    fn dispatch_frame(
        &mut self,
        frame: &[u8],
        post: PostTransmit,
        expires: Option<Instant>,
        now: Instant,
    ) -> Result<(), CoreError> {
        if self.active_transmit.is_some() {
            return Err(CoreError::ActionCapacity);
        }
        let id = TransmitId::new(self.next_transmit_generation);
        self.actions
            .enqueue_transmission(id, frame)
            .map_err(|error| self.action_error(error))?;
        self.next_transmit_generation = self.next_transmit_generation.wrapping_add(1);
        self.active_transmit = Some(ActiveTransmit {
            id,
            asserted_at: now,
            started: false,
            post,
            expires,
        });
        self.protocol_deadline = None;
        Ok(())
    }

    fn action_error(&mut self, _error: ActionEnqueueError) -> CoreError {
        self.counters.record_action_drop();
        CoreError::ActionCapacity
    }
}

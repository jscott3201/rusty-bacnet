//! Receive-frame classification and application delivery.

use super::*;

impl<'rx, Q: QueueStorage> MasterCore<'rx, Q> {
    /// Abort an expired partial receive and report whether it had progressed
    /// beyond preamble search into a candidate HEADER or DATA frame.
    pub(super) fn abort_partial_if_late(&mut self, now: Instant) -> bool {
        if self.parser_partial
            && self
                .last_octet
                .is_some_and(|last| now - last > self.config.t_frame_abort)
        {
            let invalid_candidate = matches!(
                self.decoder.phase(),
                StreamPhase::Header | StreamPhase::Data
            );
            self.decoder.reset();
            self.parser_partial = false;
            self.counters.record_frame_abort();
            if invalid_candidate {
                self.counters.record_parser_error();
            }
            return invalid_candidate;
        }
        false
    }

    pub(super) fn copy_frame(frame: Frame<'_>) -> OwnedFrame {
        let mut data = [0; MAX_STANDARD_MPDU_DATA];
        data[..frame.data.len()].copy_from_slice(frame.data);
        OwnedFrame {
            header: frame.header,
            length: frame.data.len(),
            data,
        }
    }

    pub(super) fn invalid_frame_event(&mut self, now: Instant) -> Result<(), CoreError> {
        if self.state == MasterState::WaitForReply {
            self.expected_reply_source = None;
            self.set_state(MasterState::DoneWithToken);
            self.drive_token(now)?;
        } else if self.state == MasterState::PollForMaster {
            self.poll_timeout(now)?;
        }
        Ok(())
    }

    pub(super) fn process_owned_frame(
        &mut self,
        frame: OwnedFrame,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.counters.record_frame_received();
        if self.pending_request.is_some() {
            return Ok(());
        }
        // A complete valid frame proves that another station began using the
        // token. The octet ingress reaches this transition through
        // Nmin_octets before decoding completes; validated host ingress must
        // apply the same transition explicitly because it arrives as one
        // event rather than as individual octets.
        if self.state == MasterState::PassToken {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
        }
        if self.state == MasterState::WaitForReply {
            return self.process_wait_for_reply(frame, now);
        }
        if self.state == MasterState::PollForMaster {
            if frame.header.frame_type == FrameType::ReplyToPollForMaster
                && frame.header.destination == self.config.station
            {
                self.next_station = frame.header.source;
                self.poll_station = self.config.station;
                self.token_count = 0;
                self.retry_token_count = 0;
                self.sole_master = false;
                return self.queue_token(self.next_station, now);
            }
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        }
        match frame.header.frame_type {
            FrameType::Token if frame.header.destination == self.config.station => {
                self.sole_master = false;
                self.frame_count = 0;
                self.retry_token_count = 0;
                self.set_state(MasterState::UseToken);
                self.drive_token(now)
            }
            FrameType::PollForMaster if frame.header.destination == self.config.station => self
                .queue_frame(
                    FrameType::ReplyToPollForMaster,
                    frame.header.source,
                    &[],
                    PostTransmit::Idle,
                    now,
                ),
            FrameType::BACnetDataNotExpectingReply
                if frame.header.destination == self.config.station
                    || frame.header.destination == BROADCAST_MAC =>
            {
                self.deliver(&frame, None);
                Ok(())
            }
            FrameType::BACnetDataExpectingReply if frame.header.destination == BROADCAST_MAC => {
                self.deliver(&frame, None);
                Ok(())
            }
            FrameType::BACnetDataExpectingReply
                if frame.header.destination == self.config.station =>
            {
                let id = RequestId::new(self.next_request_generation);
                self.next_request_generation = self.next_request_generation.wrapping_add(1);
                let deadline = now + self.config.t_reply_delay;
                self.pending_request = Some(PendingRequest {
                    id,
                    source: frame.header.source,
                    deadline,
                });
                self.set_state(MasterState::AnswerDataRequest);
                self.protocol_deadline = Some(deadline);
                if self.deliver(&frame, Some(id)) {
                    Ok(())
                } else {
                    self.complete_request(id, ReplyDecision::Postponed, now)
                }
            }
            FrameType::TestRequest if frame.header.destination == self.config.station => self
                .queue_frame(
                    FrameType::TestResponse,
                    frame.header.source,
                    &frame.data[..frame.length],
                    PostTransmit::Idle,
                    now,
                ),
            _ => Ok(()),
        }
    }

    fn process_wait_for_reply(&mut self, frame: OwnedFrame, now: Instant) -> Result<(), CoreError> {
        let is_reply = frame.header.destination == self.config.station
            && self.expected_reply_source == Some(frame.header.source)
            && matches!(
                frame.header.frame_type,
                FrameType::BACnetDataNotExpectingReply | FrameType::ReplyPostponed
            );
        self.expected_reply_source = None;
        if !is_reply {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + self.config.t_no_token);
            return Ok(());
        }
        if frame.header.frame_type == FrameType::BACnetDataNotExpectingReply {
            self.deliver(&frame, None);
        }
        self.set_state(MasterState::DoneWithToken);
        self.drive_token(now)
    }

    fn deliver(&mut self, frame: &OwnedFrame, request_id: Option<RequestId>) -> bool {
        match self.actions.enqueue_delivery(
            frame.header.source,
            frame.header.destination,
            &frame.data[..frame.length],
            request_id,
        ) {
            Ok(()) => {
                self.counters.record_npdu_delivered();
                true
            }
            Err(_) => {
                self.counters.record_action_drop();
                false
            }
        }
    }
}

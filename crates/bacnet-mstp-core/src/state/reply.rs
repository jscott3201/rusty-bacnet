//! Answering a received data-expecting-reply frame (Clause 9.5.6.9).
//!
//! The application has until T_reply_delay less the configured transmit
//! margin to decide. After that the core sends Reply Postponed itself, even
//! when its deadline callback or the platform's transmit start comes late.
//! Such a late answer gains the requester little, a few milliseconds at most
//! before its own reply timeout would have ended the wait, but it is still
//! the correct answer, so the core sends it while that is safe. It stops once
//! the requester may have given up: its shortest T_reply_timeout (255 ms,
//! Clause 9.5.3), less one octet, which has to arrive whole before that
//! timeout, and less a small fixed guard. A request dropped then is counted.

use super::*;

/// A small fixed guard kept back from the requester's shortest reply
/// timeout, against adapter and clock jitter: the request time the adapter
/// reports can only be later than the requester's last octet.
const REPLY_GUARD: Duration = Duration::from_millis(1);

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Hand a request to the application and open its reply window.
    pub(super) fn begin_request(&mut self, frame: Frame<'_>, now: Instant) {
        let id = RequestId::new(self.next_request_generation);
        self.next_request_generation = self.next_request_generation.wrapping_add(1);
        let window = self.reply_window(now);
        let request = PendingRequest {
            id,
            source: frame.header.source,
            decide_by: window.send_by - self.config.reply_transmit_margin,
            window,
        };
        self.pending_request = Some(request);
        self.set_state(MasterState::AnswerDataRequest);
        self.protocol_deadline = Some(request.decide_by);
        if !self.deliver(frame, Some(id)) {
            // The request was just made and its window is open, so Reply
            // Postponed is always taken.
            let _ = self.complete_request(id, ReplyDecision::Postponed, now);
        }
    }

    /// Complete a request-bearing delivery using its generation identifier.
    ///
    /// A [`ReplyDecision::Data`] answer has to arrive before the decision
    /// deadline: T_reply_delay less
    /// [`MstpCoreConfig::reply_transmit_margin`] after the request. A later
    /// one is refused with [`RequestError::Expired`], and the core keeps
    /// the request and sends Reply Postponed when [`Self::on_deadline`] runs.
    ///
    /// Once the requester may have stopped waiting (the shortest
    /// T_reply_timeout, 255 ms, less one octet time and a 1 ms guard), every
    /// decision is refused that way, [`ReplyDecision::Abandoned`] included,
    /// and the request is dropped unanswered and counted in
    /// [`MstpCounters::expired_requests`].
    ///
    /// A data answer longer than this station sends
    /// ([`MstpCoreConfig::max_npdu`]) is refused with
    /// [`RequestError::Oversize`]. With extended frames on, one of 502
    /// octets or more goes out as BACnet Extended Data Not Expecting Reply
    /// (Type 33), whatever the type of the request.
    pub fn complete_request(
        &mut self,
        id: RequestId,
        decision: ReplyDecision<'_>,
        now: Instant,
    ) -> Result<(), RequestError> {
        let now = self.observe_time(now);
        let request = self.pending_request.ok_or(RequestError::NoPending)?;
        if request.id != id {
            return Err(RequestError::Stale);
        }
        if now >= request.window.give_up {
            self.release_unanswered_request(now);
            return Err(RequestError::Expired);
        }
        let (frame_type, payload) = match decision {
            ReplyDecision::Data(_) if now >= request.decide_by => {
                return Err(RequestError::Expired);
            }
            ReplyDecision::Data(data) if data.len() > self.config.max_npdu() => {
                return Err(RequestError::Oversize {
                    length: data.len(),
                    maximum: self.config.max_npdu(),
                });
            }
            ReplyDecision::Data(data) if data.len() > MAX_STANDARD_MPDU_DATA => {
                (FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY, data)
            }
            ReplyDecision::Data(data) => (FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, data),
            ReplyDecision::Postponed => (FrameType::REPLY_POSTPONED, &[][..]),
            ReplyDecision::Abandoned => {
                self.pending_request = None;
                self.enter_idle(now);
                return Ok(());
            }
        };
        self.queue_frame(
            frame_type,
            request.source,
            payload,
            PostTransmit::Idle,
            Origin::Answer(request.window),
            now,
        );
        self.pending_request = None;
        Ok(())
    }

    /// The decision deadline has passed without an answer: send Reply
    /// Postponed, however late, unless the requester may have stopped waiting.
    pub(super) fn reply_deadline(&mut self, now: Instant) {
        let Some(request) = self.pending_request else {
            return self.enter_idle(now);
        };
        // Reply Postponed for the waiting request is refused only as too
        // late, when the request was dropped and counted.
        let _ = self.complete_request(request.id, ReplyDecision::Postponed, now);
    }

    /// The window for answering a frame that asked for a reply (a data
    /// request or Test_Request) and ended at `now`: T_reply_delay, then the
    /// requester's shortest T_reply_timeout.
    pub(super) fn reply_window(&self, now: Instant) -> ReplyWindow {
        let send_by = now + self.config.t_reply_delay;
        // Never before the end of T_reply_delay, which a reply may use in
        // full. Defensive: at the lowest baud rate validation allows, the
        // cutoff still falls just after T_reply_delay.
        let give_up =
            (now + MIN_REPLY_TIMEOUT - self.config.octet_time() - REPLY_GUARD).max(send_by);
        ReplyWindow { send_by, give_up }
    }

    /// The window for answering a Poll For Master that ended at `now`:
    /// T_usage_delay, then the poller's shortest T_usage_timeout (Clauses
    /// 9.5.3 and 9.5.6.8), less one octet and the guard as for a reply.
    pub(super) fn poll_reply_window(&self, now: Instant) -> ReplyWindow {
        let send_by = now + self.config.t_usage_delay;
        let give_up =
            (now + MIN_USAGE_TIMEOUT - self.config.octet_time() - REPLY_GUARD).max(send_by);
        ReplyWindow { send_by, give_up }
    }

    /// Drop a request unanswered because its requester may have stopped
    /// waiting, and count it.
    pub(super) fn release_unanswered_request(&mut self, now: Instant) {
        self.pending_request = None;
        self.counters.record_expired_request();
        self.enter_idle(now)
    }
}

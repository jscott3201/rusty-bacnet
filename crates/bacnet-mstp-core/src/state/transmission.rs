//! Transmit actions and the adapter's transmit events.

use super::*;

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Confirm that the platform is about to start the matching UART transfer.
    ///
    /// Call it after polling [`Action::Transmit`] and before enabling the
    /// line driver, and send only if it returns `Ok`. Until it does, keep the
    /// receiver on and feed the core what it hears: those octets are line
    /// activity, not echo. Right before the call, drain the receive FIFO or
    /// DMA buffer and feed the core every octet and receive error in it; an
    /// adapter that cannot tell when they arrived may stamp them with the
    /// drain time, which is safe because a later stamp can only defer the
    /// start. After an error for a transmit, or once it is aborted, send
    /// nothing more for its id; the one exception is
    /// [`CoreError::TransmitNotPolled`], below.
    ///
    /// A host assembler that was part way through a frame drops it now: the
    /// receiver is off while the driver is on, so it can never complete, and
    /// the core drops its own decoder's partial frame here too.
    ///
    /// - A start for a transmit whose [`Action::Transmit`] has not been
    ///   polled is refused with [`CoreError::TransmitNotPolled`] and counted
    ///   in [`MstpCounters::adapter_errors`]. It is an adapter bug: an id
    ///   learned elsewhere, such as from [`Self::snapshot`], is no frame to
    ///   send. Nothing changes and nothing is queued; the id stays live, and
    ///   the fix is to poll the frame, then start it.
    /// - If the line was active within T_turnaround of `now` (activity that
    ///   reached the core after the frame was handed over), the start is
    ///   refused with [`CoreError::TransmitDeferred`] and counted in
    ///   [`MstpCounters::deferred_starts`]. The frame waits for a quiet line
    ///   again (Clause 9.5.5.1) and comes back as a new [`Action::Transmit`],
    ///   unless its waiting rules drop it.
    /// - An answer that starts after its send-by time (T_reply_delay, or
    ///   T_usage_delay for Reply To Poll For Master) still goes out and is
    ///   counted in [`MstpCounters::late_replies`]. One that would start once
    ///   the asker may have stopped waiting is refused with
    ///   [`CoreError::RequestExpired`] and counted as an expired request.
    ///
    /// A deferred or expired start queues
    /// [`Direction::Receive`](crate::Direction::Receive); the other
    /// refusals queue nothing.
    pub fn on_transmit_started(&mut self, id: TransmitId, now: Instant) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        let tx = self
            .active_transmit
            .filter(|tx| tx.id == id)
            .ok_or(CoreError::StaleTransmit)?;
        if tx.started {
            self.counters.record_adapter_error();
            return Err(CoreError::TransmitAlreadyStarted);
        }
        // The control actions stay within a switch to receive and one
        // transmit only because a transmit is polled before it starts.
        if self.actions.transmit_queued(id) {
            self.counters.record_adapter_error();
            return Err(CoreError::TransmitNotPolled);
        }
        // An unconfirmed transmit is still the waiting frame.
        let pending = self.pending_send.ok_or(CoreError::StaleTransmit)?;
        let window = match pending.origin {
            Origin::Answer(window) => Some(window),
            Origin::Holder | Origin::NextPoll | Origin::PassToken | Origin::NoToken => None,
        };
        if window.is_some_and(|window| now >= window.give_up) {
            self.drop_pending(now)?;
            return Err(CoreError::RequestExpired);
        }
        if now < pending.earliest {
            // Activity since the hand-over: the line has not been quiet for
            // T_turnaround, so the frame waits again.
            self.withdraw_transmit(id);
            self.counters.record_deferred_start();
            // Past its limit: drop it now, rather than leave a deadline that
            // is already due.
            if now >= pending.limit {
                self.drop_pending(now)?;
            } else {
                self.protocol_deadline = Some(pending.deadline());
            }
            return Err(CoreError::TransmitDeferred);
        }
        if window.is_some_and(|window| now > window.send_by) {
            self.counters.record_late_reply();
        }
        // A partial frame whose T_frame_abort fell due unserviced is counted
        // as on_deadline would count it (no invalid-frame event while this
        // frame waits).
        let _ = self.abort_partial_if_late(now);
        // The receiver goes off as the driver goes on (Clause 9.5.5.1), so a
        // frame part received now can never complete. Drop it as an abort,
        // not an invalid frame: its T_frame_abort would otherwise fall during
        // or after this transmission and be judged in the state it leads to.
        if self.parser_partial {
            self.decoder.reset();
            self.parser_partial = false;
            self.counters.record_frame_abort();
        }
        self.pending_send = None;
        let tx = self.active_transmit.as_mut().expect("checked above");
        tx.started = true;
        tx.asserted_at = now;
        Ok(())
    }

    /// Report that the final stop bit for the matching generation left UART.
    ///
    /// Report it before feeding the core any octet or frame that arrived
    /// after the transmission ended, so they are judged in the state the
    /// transmission leads to, and after polling every action queued before
    /// it.
    ///
    /// The station's own echo must reach the core, or be dropped, before
    /// this call, on either input path: octets for [`Self::on_octet`], and
    /// a frame a host assembler built from the echo. Fed afterwards, the
    /// echo is taken for another station's: its own Token heard back in
    /// PASS_TOKEN would read as the successor using the token (SawTokenUser).
    ///
    /// The start must have been confirmed first. Completion without it is
    /// refused with [`CoreError::TransmitNotStarted`] and counted in
    /// [`MstpCounters::adapter_errors`]; the transmit stays active until it
    /// is started and completed, aborted, or caught by the watchdog.
    pub fn on_transmit_complete(&mut self, id: TransmitId, now: Instant) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        let tx = self
            .active_transmit
            .filter(|tx| tx.id == id)
            .ok_or(CoreError::StaleTransmit)?;
        if !tx.started {
            self.counters.record_adapter_error();
            return Err(CoreError::TransmitNotStarted);
        }
        self.actions.queue_receive();
        self.active_transmit = None;
        self.last_activity = now;
        self.counters
            .record_de_assertion(now.saturating_duration_since(tx.asserted_at));
        self.counters.record_frame_transmitted();
        match tx.post {
            PostTransmit::Idle => self.enter_idle(now),
            PostTransmit::ContinueToken => {
                self.set_state(MasterState::UseToken);
                self.drive_token(now)
            }
            PostTransmit::WaitForReply => {
                self.set_state(MasterState::WaitForReply);
                self.protocol_deadline = Some(now + self.config.t_reply_timeout);
                Ok(())
            }
            PostTransmit::PassToken => {
                self.set_state(MasterState::PassToken);
                self.event_count = 0;
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                self.counters.record_token_rotation();
                Ok(())
            }
            PostTransmit::PollForMaster => {
                self.set_state(MasterState::PollForMaster);
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                Ok(())
            }
        }
    }

    /// Abort the matching transmit and restore logical receive direction.
    ///
    /// Its actions not yet polled are withdrawn, so the next action is the
    /// switch back to receive. The core's transmit watchdog uses it too, for
    /// a transmit the adapter never started or never completed. A frame
    /// aborted before its start was confirmed never went out, so it adds no
    /// driver-on time and is counted as given up: in
    /// [`MstpCounters::expired_requests`] for an answer, which leaves its
    /// request unanswered, and in [`MstpCounters::cancelled_sends`] for any
    /// other frame. Every abort counts under its `reason`, in
    /// [`MstpCounters::transmit_aborts`] and the three beside it.
    pub fn on_transmit_aborted(
        &mut self,
        id: TransmitId,
        reason: TransmitFailure,
        now: Instant,
    ) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        let tx = self
            .active_transmit
            .filter(|tx| tx.id == id)
            .ok_or(CoreError::StaleTransmit)?;
        self.counters.record_transmit_failure(reason);
        self.withdraw_transmit(id);
        self.forget_pending();
        if tx.started {
            // The driver was on until now, and each octet it sent restarted
            // the line's silence (Clause 9.5.5.1), so T_no_token and the
            // generation slots run from here, as they do from a completion.
            self.last_activity = now;
            self.counters
                .record_de_assertion(now.saturating_duration_since(tx.asserted_at));
        }
        self.set_state(MasterState::Idle);
        self.protocol_deadline = Some(now + T_NO_TOKEN);
        self.take_pending_token(now)
    }

    /// Whether the adapter is sending a frame whose start it confirmed: what
    /// the core hears then is its own echo.
    pub(super) fn transmitting(&self) -> bool {
        self.active_transmit.is_some_and(|tx| tx.started)
    }

    /// Hand the encoded frame to the adapter as a direction change and a
    /// transmit. Only the waiting frame comes here, and only while no
    /// transmit is active.
    pub(super) fn dispatch_frame(&mut self, post: PostTransmit, now: Instant) {
        debug_assert!(self.active_transmit.is_none(), "a second transmit");
        let id = TransmitId::new(self.next_transmit_generation);
        self.next_transmit_generation = self.next_transmit_generation.wrapping_add(1);
        self.actions.queue_transmit(id);
        self.active_transmit = Some(ActiveTransmit {
            id,
            asserted_at: now,
            started: false,
            post,
        });
        self.protocol_deadline = None;
    }

    /// Take a transmit back: withdraw its unpolled actions and queue the
    /// switch back to receive.
    pub(super) fn withdraw_transmit(&mut self, id: TransmitId) {
        self.actions.purge_transmit(id);
        self.actions.queue_receive();
        self.active_transmit = None;
    }
}

//! Transmit actions and the adapter's transmit events.

use super::*;

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Confirm that the platform is about to start the matching UART
    /// transfer, and take the frame to send: its whole encoded octets,
    /// preamble and CRCs included.
    ///
    /// Call it after polling [`Action::Transmit`] and right before enabling
    /// the line driver. Only a confirmed start returns the octets, so the
    /// line is driven only on success: there is nothing to send otherwise.
    /// The octets borrow the core until its next `&mut` call. Send them from
    /// there where the platform can; an adapter that needs its own copy (an
    /// interrupt-driven UART that also feeds the core its echo, or a host
    /// echo filter) makes it now, and keeps that copy short, since the
    /// time from this call to the driver going on is time the core cannot
    /// see. Until the call, keep the receiver on and feed the core what it
    /// hears: those octets are line activity, not echo. Right before it,
    /// drain the receive FIFO or DMA buffer and feed the core every octet
    /// and receive error in it; an adapter that cannot tell when they
    /// arrived may stamp them with the drain time, which is safe because a
    /// later stamp can only defer the start. After an error for a
    /// transmit, or once it is aborted, there is nothing to send for its id;
    /// the one exception is [`TransmitError::NotPolled`], below.
    ///
    /// Handing out the octets here (design decision D10, #1769) keeps the
    /// turnaround contract as it was: the T_turnaround check runs at this
    /// call, as before; a start too soon after line activity is refused
    /// and the frame waits again for a quiet line (Clause 9.5.5.1); and
    /// input counts as echo only once a start is confirmed. What changed is
    /// only that a refused start has no octets to send by mistake.
    ///
    /// A host assembler that was part way through a frame drops it now: the
    /// receiver is off while the driver is on, so it can never complete, and
    /// the core drops its own decoder's partial frame here too.
    ///
    /// - A start for a transmit whose [`Action::Transmit`] has not been
    ///   polled is refused with [`TransmitError::NotPolled`] and counted
    ///   in [`MstpCounters::adapter_errors`]. It is an adapter bug: an id
    ///   learned elsewhere, such as from [`Self::snapshot`], is no frame to
    ///   send. Nothing changes and nothing is queued; the id stays live, and
    ///   the fix is to poll the frame, then start it.
    /// - If the line was active within T_turnaround of `now` (activity that
    ///   reached the core after the frame was handed over), the start is
    ///   refused with [`TransmitError::Deferred`] and counted in
    ///   [`MstpCounters::deferred_starts`]. The frame waits for a quiet line
    ///   again (Clause 9.5.5.1) and comes back as a new [`Action::Transmit`],
    ///   unless its waiting rules drop it.
    /// - An answer that starts after its send-by time (T_reply_delay, or
    ///   T_usage_delay for Reply To Poll For Master) still goes out and is
    ///   counted in [`MstpCounters::late_replies`]. One that would start once
    ///   the asker may have stopped waiting is refused with
    ///   [`TransmitError::Expired`] and counted as an expired request.
    ///
    /// A deferred or expired start queues
    /// [`Direction::Receive`](crate::Direction::Receive); the other
    /// refusals queue nothing.
    pub fn on_transmit_started(
        &mut self,
        id: TransmitId,
        now: Instant,
    ) -> Result<&[u8], TransmitError> {
        let now = self.observe_time(now);
        let tx = self
            .active_transmit
            .filter(|tx| tx.id == id)
            .ok_or(TransmitError::Stale)?;
        if tx.started {
            self.counters.record_adapter_error();
            return Err(TransmitError::AlreadyStarted);
        }
        // The control actions stay within a switch to receive and one
        // transmit only because a transmit is polled before it starts.
        if self.actions.transmit_queued(id) {
            self.counters.record_adapter_error();
            return Err(TransmitError::NotPolled);
        }
        // An unconfirmed transmit is still the waiting frame.
        let pending = self.pending_send.ok_or(TransmitError::Stale)?;
        let window = match pending.origin {
            Origin::Answer(window) => Some(window),
            Origin::Holder | Origin::NextPoll | Origin::PassToken | Origin::NoToken => None,
        };
        if window.is_some_and(|window| now >= window.give_up) {
            self.drop_pending(now);
            return Err(TransmitError::Expired);
        }
        if now < pending.earliest {
            // Activity since the hand-over: the line has not been quiet for
            // T_turnaround, so the frame waits again.
            self.withdraw_transmit(id);
            self.counters.record_deferred_start();
            // Past its limit: drop it now, rather than leave a deadline that
            // is already due.
            if now >= pending.limit {
                self.drop_pending(now);
            } else {
                self.protocol_deadline = Some(pending.deadline());
            }
            return Err(TransmitError::Deferred);
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
        Ok(self.actions.frame())
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
    /// The start must have been confirmed first. Completion without it is an
    /// adapter bug: it is ignored and counted in
    /// [`MstpCounters::adapter_errors`], and the transmit stays active until
    /// it is started and completed, aborted, or caught by the watchdog. A
    /// completion for an id that is not the active transmit's (one the
    /// watchdog already aborted, say) is ignored.
    pub fn on_transmit_complete(&mut self, id: TransmitId, now: Instant) {
        let now = self.observe_time(now);
        let Some(tx) = self.active_transmit.filter(|tx| tx.id == id) else {
            return;
        };
        if !tx.started {
            self.counters.record_adapter_error();
            return;
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
            }
            PostTransmit::PassToken => {
                self.set_state(MasterState::PassToken);
                self.event_count = 0;
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
                self.counters.record_token_rotation();
            }
            PostTransmit::PollForMaster => {
                self.set_state(MasterState::PollForMaster);
                self.protocol_deadline = Some(now + self.config.t_usage_timeout);
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
    /// [`MstpCounters::transmit_aborts`] and the three beside it. An abort
    /// for an id that is not the active transmit's (one already completed,
    /// aborted or taken back) is ignored.
    pub fn on_transmit_aborted(&mut self, id: TransmitId, reason: TransmitFailure, now: Instant) {
        let now = self.observe_time(now);
        let Some(tx) = self.active_transmit.filter(|tx| tx.id == id) else {
            return;
        };
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

    /// The frame handed over as [`Action::Transmit`] and not started yet,
    /// with its id: for a test oracle that checks what the core will send
    /// before the adapter starts it (feature `inspect`). Never drive the
    /// line from it: an adapter gets the octets to send from
    /// [`Self::on_transmit_started`], and only when the start is confirmed.
    ///
    /// A test and fuzz hook, hidden from the docs and not covered by the
    /// crate's semver: it may change or go in any release.
    #[cfg(feature = "inspect")]
    #[doc(hidden)]
    #[must_use]
    pub fn handed_over_frame(&self) -> Option<(TransmitId, &[u8])> {
        self.active_transmit
            .filter(|tx| !tx.started && !self.actions.transmit_queued(tx.id))
            .map(|tx| (tx.id, self.actions.frame()))
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

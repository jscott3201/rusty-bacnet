//! Frames this station owes the line but has not started yet.
//!
//! The SendFrame procedure (Clause 9.5.5.1) waits for T_turnaround of quiet
//! line before the driver goes on, and nothing in it abandons the frame. The
//! core keeps such a frame here from the moment a transition produces it
//! until the adapter confirms its start: first while it waits out the
//! turnaround, then, once handed over as [`Action::Transmit`], until
//! [`MasterCore::on_transmit_started`] succeeds. Only then are received
//! octets the station's own echo.
//!
//! Line activity in either phase (an octet, a receive error, host-reported
//! octets, or a frame a host assembler found valid or invalid) moves the
//! frame's earliest start to T_turnaround after it, and a start reported
//! sooner is refused with [`CoreError::TransmitDeferred`]: the frame waits
//! again and comes back as a new transmit. The state the frame was made for
//! takes none of its own transitions meanwhile, because the transition that
//! made the frame has not finished. Two exceptions end the wait early:
//! activity that is exactly the evidence the sending state watches for, so
//! that state's own transition wins. And every frame has a limit taken from
//! its state; a frame that still cannot start by then is dropped and
//! counted.
//!
//! Every place a frame can wait, with the core state named as Clause 9.5.6
//! names it (events are octets and receive errors, EventCount):
//!
//! | Frame, and the state that sends it | Activity while it waits | Then | Limit, and its counter | Clauses |
//! | --- | --- | --- | --- | --- |
//! | Data reply or Reply Postponed, from ANSWER_DATA_REQUEST | any | waits T_turnaround again; frames that complete meanwhile are handled as while answering a request | the reply window: the asker's shortest T_reply_timeout (255 ms) less one octet and 1 ms; `expired_requests` | 9.5.6.9, 9.5.3 |
//! | Test_Response to a Test_Request, from IDLE | any | as above | as above | 9.5.6.2, 9.5.6.9 |
//! | Reply To Poll For Master, from IDLE | any | as above | the poller's shortest T_usage_timeout (20 ms) less one octet and 1 ms; `expired_requests` | 9.5.6.2, 9.5.6.8, 9.5.3 |
//! | Data frame, from USE_TOKEN (sole master included) | any | waits again; data that needs no answer from this station is delivered, and a Token, Poll For Master, data request, Test_Request or late Reply To Poll For Master for it is dropped (`discarded_while_busy`) | T_no_token after the frame was made; `cancelled_sends`, then IDLE | 9.5.6.3, 9.5.5.1 |
//! | Token, from DONE_WITH_TOKEN or POLL_FOR_MASTER (after a reply, a reply timeout or an invalid reply too; DoneWithPFM's included) | any | as for a data frame | as for a data frame | 9.5.6.4, 9.5.6.5, 9.5.6.8 |
//! | Poll For Master, from DONE_WITH_TOKEN | any | as for a data frame | as for a data frame | 9.5.6.5 |
//! | Poll For Master for the next address, from POLL_FOR_MASTER (SendNextPFM) | any | as for a data frame | as for a data frame | 9.5.6.8 |
//! | the same | a whole Reply To Poll For Master for this station, from any station (usually a late answer to the poll before) | dropped (`cancelled_sends`); the replier becomes the successor and gets the Token (ReceivedReplyToPFM) | none | 9.5.6.8 |
//! | Token retry, from PASS_TOKEN | up to Nmin_octets events, counted from when the retry was made | waits again | T_usage_timeout after the frame was made; `cancelled_sends` | 9.5.6.6 |
//! | Poll For Master for a new successor, from PASS_TOKEN | up to Nmin_octets events since this station's last Token went out (up to Nmin_octets can carry over) | waits again | as for a retry | 9.5.6.6 |
//! | either | more than Nmin_octets events, or a whole valid frame: the successor uses the token (SawTokenUser) | dropped (`cancelled_sends`); IDLE takes up what follows | none | 9.5.6.6 |
//! | Token-generation Poll For Master, from NO_TOKEN | up to Nmin_octets events since NO_TOKEN began | waits again | T_no_token after the frame was made; `cancelled_sends` | 9.5.6.7 |
//! | the same | more than Nmin_octets events, or a whole valid frame: another station is active (SawFrame) | dropped (`cancelled_sends`); IDLE takes up what follows | none | 9.5.6.7 |
//!
//! EventCount is zeroed as each Token goes out (Clauses 9.5.6.5, 9.5.6.6),
//! so the poll for a new successor counts what followed the last retry.
//! The retry itself counts from when it is made, a local extension: the
//! standard zeroes EventCount only once the retry is out, and evaluates
//! nothing in PASS_TOKEN while it waits.
//!
//! The PASS_TOKEN and NO_TOKEN frames follow a silence of at least
//! T_usage_timeout or T_no_token, so they go to the adapter at once;
//! activity can only reach them before an adapter confirms the start. Their
//! event count bounds that activity to Nmin_octets events, each worth up to
//! T_turnaround of delay, so their time limits bind only for a slow adapter
//! or, for PASS_TOKEN, a long T_turnaround.
//!
//! A frame aborted before it starts, by the adapter or by the core's
//! transmit watchdog, is counted as if dropped: `expired_requests` for an
//! answer, `cancelled_sends` for any other. The confirmed start itself drops
//! any frame part received meanwhile, counted in `frame_aborts`, a local
//! extension: the standard turns the receiver off while the driver is on
//! (Clause 9.5.5.1), so that frame can never complete, but says nothing of
//! resetting the receive state machine.
//!
//! An answer's limit applies to its start as well: one the adapter starts at
//! or after it is refused with [`CoreError::RequestExpired`], since the asker
//! may be transmitting by then. Other frames' limits bound only waits caused
//! by line activity.

use super::*;

/// What made a frame that waits to go out, which decides what may end the
/// wait early and how long it may last.
#[derive(Debug, Clone, Copy)]
pub(super) enum Origin {
    /// An answer to a frame this station received (a data reply, Reply
    /// Postponed, Test_Response or Reply To Poll For Master), inside the
    /// window its asker waits for it.
    Answer(ReplyWindow),
    /// A frame this station sends while it holds the token.
    Holder,
    /// The poll for the next address in a successor search, from
    /// POLL_FOR_MASTER (SendNextPFM): a late answer to the poll before it
    /// still finds the successor.
    NextPoll,
    /// A token retry, or a poll for a new successor, from PASS_TOKEN.
    PassToken,
    /// The poll that generates a token, from NO_TOKEN.
    NoToken,
}

/// A frame this station has made but not yet started on the wire. Its octets
/// wait in the action buffers' frame buffer, which nothing else uses until
/// the frame is over.
#[derive(Debug, Clone, Copy)]
pub(super) struct PendingSend {
    pub(super) post: PostTransmit,
    pub(super) origin: Origin,
    /// T_turnaround after the last line activity.
    pub(super) earliest: Instant,
    /// From here on the frame is dropped rather than sent.
    pub(super) limit: Instant,
}

impl PendingSend {
    /// When the core must look at this frame again: once its turnaround has
    /// passed, or at its limit if that comes first.
    pub(super) fn deadline(&self) -> Instant {
        core::cmp::min(self.earliest, self.limit)
    }
}

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Encode a frame and send it once the line has been quiet for
    /// T_turnaround: at once if it already has, otherwise from the deadline.
    ///
    /// Only one frame of this station's waits or is on the wire at a time:
    /// every caller makes a frame only from a state that has none, which is
    /// what lets the frame buffer and the control actions stay so small.
    pub(super) fn queue_frame(
        &mut self,
        frame_type: FrameType,
        destination: u8,
        data: &[u8],
        post: PostTransmit,
        origin: Origin,
        now: Instant,
    ) -> Result<(), CoreError> {
        self.actions.encode_frame(Frame::new(
            frame_type,
            destination,
            self.config.station,
            data,
        ))?;
        self.send_encoded(post, origin, now)
    }

    /// Send the frame just encoded into the action buffers' frame buffer, as
    /// [`Self::queue_frame`] does. A queued NPDU is encoded where the queue
    /// lends it, so its frame comes here directly.
    pub(super) fn send_encoded(
        &mut self,
        post: PostTransmit,
        origin: Origin,
        now: Instant,
    ) -> Result<(), CoreError> {
        debug_assert!(
            self.pending_send.is_none() && self.active_transmit.is_none(),
            "a second frame of this station's"
        );
        let limit = match origin {
            Origin::Answer(window) => window.give_up,
            Origin::Holder | Origin::NextPoll | Origin::NoToken => now + T_NO_TOKEN,
            Origin::PassToken => now + self.config.t_usage_timeout,
        };
        self.pending_send = Some(PendingSend {
            post,
            origin,
            earliest: self.last_activity + self.config.t_turnaround,
            limit,
        });
        self.dispatch_pending_send(now)
    }

    /// Hand the waiting frame to the adapter if its turnaround has passed.
    /// It stays here until the adapter confirms the start.
    pub(super) fn dispatch_pending_send(&mut self, now: Instant) -> Result<(), CoreError> {
        let Some(pending) = self.pending_send else {
            return Ok(());
        };
        // The limit first: under steady line activity the turnaround keeps
        // moving, and the wait must still end.
        if now >= pending.limit {
            return self.drop_pending(now);
        }
        if now < pending.earliest {
            self.protocol_deadline = Some(pending.deadline());
            return Ok(());
        }
        self.dispatch_frame(pending.post, now);
        Ok(())
    }

    /// Line activity while a frame waits, before or after it went to the
    /// adapter: drop it at its limit or on its state's evidence, otherwise
    /// wait T_turnaround from this activity.
    pub(super) fn pending_activity(&mut self, now: Instant) -> Result<(), CoreError> {
        let Some(pending) = self.pending_send else {
            return Ok(());
        };
        let evidence = matches!(pending.origin, Origin::PassToken | Origin::NoToken)
            && self.event_count > u32::from(N_MIN_OCTETS);
        if now >= pending.limit || evidence {
            return self.drop_pending(now);
        }
        let earliest = self.last_activity + self.config.t_turnaround;
        if let Some(pending) = self.pending_send.as_mut() {
            pending.earliest = earliest;
            if self.active_transmit.is_none() {
                self.protocol_deadline = Some(pending.deadline());
            }
        }
        Ok(())
    }

    /// A valid frame completed while a frame of this station's waits. Return
    /// whether the frame was handled here; when not, the core has gone to
    /// IDLE, which takes it up.
    pub(super) fn frame_while_pending(
        &mut self,
        frame: Frame<'_>,
        now: Instant,
    ) -> Result<bool, CoreError> {
        let Some(pending) = self.pending_send else {
            return Ok(false);
        };
        let header = frame.header;
        let late_poll_reply = header.frame_type == FrameType::REPLY_TO_POLL_FOR_MASTER
            && header.destination == self.config.station;
        match pending.origin {
            Origin::Answer(_) => self.frame_while_busy(frame, true),
            // A station answered a poll after all, usually the one polled
            // before, though any source counts: as when the answer is pending
            // on entry to POLL_FOR_MASTER (ReceivedReplyToPFM, Clause
            // 9.5.6.8), it becomes the successor, and this poll is moot.
            Origin::NextPoll if late_poll_reply => {
                self.discard_pending();
                self.take_successor(header.source, now)?;
            }
            Origin::Holder | Origin::NextPoll => self.frame_while_busy(frame, false),
            // A whole frame is more than Nmin_octets: the successor took the
            // token (SawTokenUser) or another station is active (SawFrame).
            Origin::PassToken | Origin::NoToken => {
                self.drop_pending(now)?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Give up the waiting frame and count it, then go to IDLE.
    pub(super) fn drop_pending(&mut self, now: Instant) -> Result<(), CoreError> {
        self.discard_pending();
        self.enter_idle(now)
    }

    /// Give up the waiting frame, taking it back from the adapter if it was
    /// handed over, and count it.
    fn discard_pending(&mut self) {
        if let Some(tx) = self.active_transmit.filter(|tx| !tx.started) {
            self.withdraw_transmit(tx.id);
        }
        self.forget_pending();
    }

    /// Forget the waiting frame, counting it as given up: `expired_requests`
    /// for an answer, `cancelled_sends` for any other frame.
    pub(super) fn forget_pending(&mut self) {
        let Some(pending) = self.pending_send.take() else {
            return;
        };
        match pending.origin {
            Origin::Answer(_) => self.counters.record_expired_request(),
            Origin::Holder | Origin::NextPoll | Origin::PassToken | Origin::NoToken => {
                self.counters.record_cancelled_send();
            }
        }
    }
}

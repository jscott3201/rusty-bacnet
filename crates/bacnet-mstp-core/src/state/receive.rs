//! Receive-frame classification and application delivery.

use super::*;

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Abort an expired partial receive and report whether it had progressed
    /// beyond preamble search into a candidate HEADER or DATA frame. The data
    /// phase includes the codec skipping a dropped frame by count, whatever
    /// its destination, as the skip state counts a timeout (Clause 9.5.4.7).
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

    /// Push one octet, then drain the decoder with empty pushes until it needs
    /// more input, as its stream protocol asks, acting on each frame and each
    /// dropped candidate on the way.
    ///
    /// A frame is acted on where the decoder holds it, with no copy: the
    /// decoder is lent out of the core for the drain, so the frame can borrow
    /// its storage while the core changes state. Nothing that a frame or a
    /// dropped candidate leads to touches the decoder, and it is back in
    /// place before this returns, whatever the outcome.
    pub(super) fn decode_octet(&mut self, byte: u8, now: Instant) {
        let mut decoder = core::mem::replace(&mut self.decoder, StreamDecoder::new(&mut []));
        self.drain_decoder(&mut decoder, byte, now);
        self.decoder = decoder;
    }

    fn drain_decoder(&mut self, decoder: &mut StreamDecoder<'rx>, byte: u8, now: Instant) {
        let octet = [byte];
        let mut input: &[u8] = &octet;
        loop {
            let dropped = match decoder.push_slice(input) {
                DecodeEvent::NeedMore => return,
                DecodeEvent::Invalid { .. } => None,
                // A node without COBS support has no extended frames: one
                // that the storage was large enough to decode is dropped as
                // such a node drops it, but at its last octet rather than at
                // its header (the decoder skips reserved types for it).
                DecodeEvent::Frame { frame, .. }
                    if frame.header.frame_type.is_cobs() && !self.config.extended_frames =>
                {
                    Some((
                        FrameError::UnsupportedCobsFrame {
                            raw_type: frame.header.frame_type.to_raw(),
                        },
                        Some(frame.header.destination),
                    ))
                }
                DecodeEvent::Frame { frame, .. } => {
                    self.process_frame(frame, now);
                    None
                }
                DecodeEvent::Malformed {
                    error, destination, ..
                } => Some((error, destination)),
                // Only storage too small to hold a header refuses one octet.
                DecodeEvent::Error(error) => Some((error, None)),
            };
            input = &[];
            if let Some((error, destination)) = dropped {
                if self.dropped_candidate_is_invalid(error, destination) {
                    self.counters.record_parser_error();
                    self.invalid_frame_event(now);
                }
            }
        }
    }

    /// Whether a candidate the decoder dropped is an invalid frame for this
    /// station, as the receive state machine of Clause 9.5.4 would see it.
    ///
    /// A header that fails the Clause 9.5.8 checks counts wherever the frame
    /// was going. Trouble found past the header (a length beyond this
    /// station's buffer, an encoding it can't take, a bad data CRC) counts
    /// only on a frame for this station or broadcast (Clause 9.5.4.4); a frame
    /// for another station is skipped without looking at its data. The codec
    /// checks a header's source and Length before its type, so an unsupported
    /// COBS type always has a good header.
    fn dropped_candidate_is_invalid(&self, error: FrameError, destination: Option<u8>) -> bool {
        match error {
            FrameError::HeaderCrcMismatch
            | FrameError::BroadcastSource
            | FrameError::InvalidCobsLength { .. }
            | FrameError::Oversize { .. } => true,
            // No destination means the header never completed, which the
            // storage `new` accepts rules out; count it to be safe.
            FrameError::FrameTooLong { .. }
            | FrameError::UnsupportedCobsFrame { .. }
            | FrameError::DataCrcMismatch { .. } => destination
                .is_none_or(|address| address == self.config.station || address == BROADCAST_MAC),
            // The stream decoder reports none of these for a dropped
            // candidate; a refused octet (BufferExhausted) is lost input.
            FrameError::TooShort { .. }
            | FrameError::InvalidPreamble { .. }
            | FrameError::ExtendedFrame { .. }
            | FrameError::UnexpectedData { .. }
            | FrameError::MissingVendorId { .. }
            | FrameError::Truncated { .. }
            | FrameError::BufferExhausted { .. } => true,
            // An error the codec adds later counts too, until it is placed.
            _ => true,
        }
    }

    pub(super) fn invalid_frame_event(&mut self, now: Instant) {
        // While a frame of this station's waits, its octets were only
        // activity for that frame (see the `pending` module). While one is
        // on the wire, nothing received can be judged: the start dropped any
        // partial frame, so this guard is only a second line.
        if self.pending_send.is_some() || self.active_transmit.is_some() {
            return;
        }
        if self.state == MasterState::WaitForReply {
            self.set_state(MasterState::DoneWithToken);
            self.drive_token(now);
        } else if self.state == MasterState::PollForMaster {
            self.poll_timeout(now);
        }
    }

    /// Act on a valid frame, from the core's own decoder or a host assembler.
    pub(super) fn process_frame(&mut self, frame: Frame<'_>, now: Instant) {
        self.counters.record_frame_received();
        // Never this station's own echo, which is ignored before it gets
        // here: another station uses this address. Clause 9 has no rule for
        // it, so it is counted and then handled like any other frame.
        if frame.header.source == self.config.station {
            self.counters.record_own_address_frame();
        }
        if self.frame_while_pending(frame, now) {
            return;
        }
        if self.pending_request.is_some() || self.state == MasterState::AnswerDataRequest {
            self.frame_while_busy(frame, true);
            return;
        }
        // A complete valid frame proves that another station began using the
        // token. The octet ingress reaches this transition through
        // Nmin_octets before decoding completes; validated host ingress must
        // apply the same transition explicitly because it arrives as one
        // event rather than as individual octets.
        if self.state == MasterState::PassToken {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + T_NO_TOKEN);
        }
        if self.state == MasterState::WaitForReply {
            return self.process_wait_for_reply(frame, now);
        }
        if self.state == MasterState::PollForMaster {
            if frame.header.frame_type == FrameType::REPLY_TO_POLL_FOR_MASTER
                && frame.header.destination == self.config.station
            {
                return self.take_successor(frame.header.source, now);
            }
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + T_NO_TOKEN);
            return;
        }
        let for_us = frame.header.destination == self.config.station;
        let broadcast = frame.header.destination == BROADCAST_MAC;
        match frame.header.frame_type {
            FrameType::TOKEN if for_us => self.use_received_token(now),
            FrameType::POLL_FOR_MASTER if for_us => {
                let window = self.poll_reply_window(now);
                self.queue_frame(
                    FrameType::REPLY_TO_POLL_FOR_MASTER,
                    frame.header.source,
                    &[],
                    PostTransmit::Idle,
                    Origin::Answer(window),
                    now,
                )
            }
            // ReceivedDataNoReply, and BroadcastDataNeedingReply for data
            // that asks for a reply but went to everyone: up, without a
            // request (Clause 9.5.6.2).
            frame_type if is_data_not_expecting_reply(frame_type) && (for_us || broadcast) => {
                self.deliver(frame, None);
            }
            frame_type if is_data_expecting_reply(frame_type) && broadcast => {
                self.deliver(frame, None);
            }
            // ReceivedDataNeedingReply.
            frame_type if is_data_expecting_reply(frame_type) && for_us => {
                self.begin_request(frame, now)
            }
            FrameType::TEST_REQUEST if for_us => {
                let window = self.reply_window(now);
                self.queue_frame(
                    FrameType::TEST_RESPONSE,
                    frame.header.source,
                    frame.data,
                    PostTransmit::Idle,
                    Origin::Answer(window),
                    now,
                )
            }
            _ if self.goes_up_whole(frame) => {
                self.deliver(frame, None);
            }
            // ReceivedUnwantedFrame: for another station, or a type this
            // node doesn't know, a reserved COBS type with a good CRC-32K
            // included (Clause 9.5.4.9).
            _ => {}
        }
    }

    /// Whether a frame's data goes up whole, as [`Action::DeliverFrame`]: a
    /// Test_Response, or a proprietary frame (types 128-255) that opens with
    /// its 2-octet vendor identifier (Clause 9.3.11), sent to this station or
    /// broadcast. Clause 9.5.6.2 passes both up as data that needs no reply
    /// (ReceivedDataNoReply). Whether a proprietary type asks for a reply is
    /// the vendor's to define; the core knows of none that does, so it
    /// treats every one as needing none, broadcast included.
    fn goes_up_whole(&self, frame: Frame<'_>) -> bool {
        let header = frame.header;
        let for_us =
            header.destination == self.config.station || header.destination == BROADCAST_MAC;
        for_us
            && (header.frame_type == FrameType::TEST_RESPONSE
                || (header.frame_type.is_proprietary() && frame.data.len() >= 2))
    }

    /// A station answered this station's Poll For Master: it becomes the
    /// successor and gets the token (ReceivedReplyToPFM, Clause 9.5.6.8).
    pub(super) fn take_successor(&mut self, source: u8, now: Instant) {
        self.next_station = source;
        self.poll_station = self.config.station;
        self.token_count = 0;
        self.retry_token_count = 0;
        self.sole_master = false;
        self.queue_token(self.next_station, Origin::Holder, now)
    }

    /// Take up a Token addressed to this station (Clause 9.5.6.2).
    pub(super) fn use_received_token(&mut self, now: Instant) {
        self.sole_master = false;
        self.frame_count = 0;
        self.retry_token_count = 0;
        self.set_state(MasterState::UseToken);
        self.drive_token(now)
    }

    /// A valid frame that completes while this station is busy: answering a
    /// request, or waiting to send a frame of its own (Clause 9.5.5.1).
    ///
    /// No transition out of ANSWER_DATA_REQUEST consumes a received frame
    /// (Clause 9.5.6.9), and none can fire while SendFrame waits, so the
    /// standard leaves the frame pending for the state that follows. The core
    /// keeps no second frame buffer for that, and never lets the frame
    /// displace its own:
    ///
    /// - Data that needs no reply from this station is delivered at once: an
    ///   NPDU, a Test_Response or a proprietary frame.
    /// - A Token for this station is remembered when `keep_token` is set (the
    ///   station answers from IDLE, and goes back there) and taken up on the
    ///   way to IDLE. Otherwise this station already holds the token, and the
    ///   Token is dropped and counted.
    /// - A Poll For Master, data request or Test_Request for this station is
    ///   dropped and counted in [`MstpCounters::discarded_while_busy`]: an
    ///   answer after this station's own frame would come too late, and could
    ///   collide with the station that asked. So is a Reply To Poll For
    ///   Master for it that nothing is waiting for.
    ///
    /// Any frame but delivered data replaces a remembered Token, as a newer
    /// frame replaces a pending one in the receive state machine.
    pub(super) fn frame_while_busy(&mut self, frame: Frame<'_>, keep_token: bool) {
        let header = frame.header;
        let for_us = header.destination == self.config.station;
        let broadcast = header.destination == BROADCAST_MAC;
        match header.frame_type {
            frame_type if is_data_not_expecting_reply(frame_type) && (for_us || broadcast) => {
                self.deliver(frame, None);
            }
            frame_type if is_data_expecting_reply(frame_type) && broadcast => {
                self.deliver(frame, None);
            }
            _ if self.goes_up_whole(frame) => {
                self.deliver(frame, None);
            }
            FrameType::TOKEN if for_us && keep_token => self.pending_token = true,
            FrameType::TOKEN
            | FrameType::POLL_FOR_MASTER
            | FrameType::BACNET_DATA_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
            | FrameType::TEST_REQUEST
            | FrameType::REPLY_TO_POLL_FOR_MASTER
                if for_us =>
            {
                self.pending_token = false;
                self.counters.record_discarded_while_busy();
            }
            _ => self.pending_token = false,
        }
    }

    /// Clause 9.5.6.4 takes any reply-type frame addressed to this station as
    /// the answer, whichever station sent it; the source is not compared with
    /// the station that was asked.
    ///
    /// A data reply, standard or extended, or a Test_Response is passed up
    /// (ReceivedReply). Any other frame, a proprietary one included since the
    /// core knows no proprietary reply, is unexpected: the station goes to
    /// IDLE without passing it up, and drops the token
    /// (ReceivedUnexpectedFrame). So is a reserved COBS type with a good
    /// CRC-32K, a valid frame of a type this node doesn't know (Clause
    /// 9.5.4.9).
    fn process_wait_for_reply(&mut self, frame: Frame<'_>, now: Instant) {
        let frame_type = frame.header.frame_type;
        let is_reply = frame.header.destination == self.config.station
            && (is_data_not_expecting_reply(frame_type)
                || matches!(
                    frame_type,
                    FrameType::TEST_RESPONSE | FrameType::REPLY_POSTPONED
                ));
        if !is_reply {
            self.set_state(MasterState::Idle);
            self.protocol_deadline = Some(now + T_NO_TOKEN);
            return;
        }
        if frame_type != FrameType::REPLY_POSTPONED {
            self.deliver(frame, None);
        }
        self.set_state(MasterState::DoneWithToken);
        self.drive_token(now)
    }

    /// Queue a frame's data for the layers above: BACnet data as an NPDU,
    /// anything else whole. Return whether it was queued; data that finds
    /// every delivery slot full, or is longer than one, is dropped and
    /// counted.
    pub(super) fn deliver(&mut self, frame: Frame<'_>, request_id: Option<RequestId>) -> bool {
        let header = frame.header;
        match self
            .actions
            .enqueue_delivery(header, frame.data, request_id)
        {
            Ok(()) => {
                if carries_npdu(header.frame_type) {
                    self.counters.record_npdu_delivered();
                } else {
                    self.counters.record_frame_delivered();
                }
                true
            }
            Err(_) => {
                self.counters.record_delivery_drop();
                false
            }
        }
    }
}

/// BACnet data that asks for a reply: Type 5, or Type 32 for a longer NPDU
/// (Clauses 9.3.6, 9.3.9). The core sees a Type 32 only with extended
/// frames on.
fn is_data_expecting_reply(frame_type: FrameType) -> bool {
    matches!(
        frame_type,
        FrameType::BACNET_DATA_EXPECTING_REPLY | FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY
    )
}

/// BACnet data that doesn't: Type 6, or Type 33 (Clauses 9.3.7, 9.3.10).
fn is_data_not_expecting_reply(frame_type: FrameType) -> bool {
    matches!(
        frame_type,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
    )
}

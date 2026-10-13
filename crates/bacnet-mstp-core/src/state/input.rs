//! What the line does: received octets and receive errors, or the frames,
//! invalid frames and octet counts of a backend that assembles frames
//! itself.
//!
//! # Two input paths
//!
//! The octet path: an adapter feeds every octet to [`MasterCore::on_octet`]
//! and every receive error to [`MasterCore::on_receive_error`]; the core's
//! own stream decoder assembles frames and finds invalid ones. A UART
//! adapter stamps each octet as it arrives. A host adapter, whose serial API
//! hands over chunks without per-octet times, stamps each octet with the
//! time of the read that returned it; a late stamp only defers the
//! station's own sends. The host MS/TP transport in `bacnet-transport` takes
//! this path, with a T_frame_abort long enough for chunk gaps.
//!
//! The frame path, for a backend that assembles frames in hardware and has
//! no octets to feed: it reports the octets it saw with
//! [`MasterCore::on_host_activity`], then each frame that completes with
//! [`MasterCore::on_validated_frame`] and each that fails with
//! [`MasterCore::on_invalid_frame`]. Both paths drive the same transitions.
//!
//! # EventCount
//!
//! Clause 9.5.4 adds one to EventCount for each octet received and each
//! receive error. `on_octet` and `on_receive_error` count one each, and
//! `on_host_activity` counts the octets it reports. A frame-path frame,
//! `on_validated_frame` or `on_invalid_frame`, counts one more event on top
//! of its octets, so such a backend must report its octets for the count to
//! match: PASS_TOKEN and NO_TOKEN judge their successor or rival by more than
//! Nmin_octets events (SawTokenUser, SawFrame). A whole valid frame is that
//! evidence on its own, whichever path brings it.
//!
//! # Echo
//!
//! From a confirmed transmit start until it completes, every input is the
//! station's own echo and is ignored, whichever call reports it. Before the
//! start, input is line activity that can push the start back.

use super::*;

impl<'rx, Q: QueueStorage, const NPDU: usize> MasterCore<'rx, Q, NPDU> {
    /// Feed one wire-timestamped octet through the incremental codec.
    ///
    /// `now` is when the octet arrived. When it completes a frame, every
    /// reply time for that frame is measured from it.
    ///
    /// A candidate the codec drops counts as an invalid frame only as
    /// Clause 9.5.4 would count it for this station; see
    /// [`DecodeEvent::Malformed`].
    ///
    /// Once a transmit's start is confirmed, octets are taken for the
    /// station's own echo and ignored until it completes. Before that they
    /// are line activity like any other (see [`Self::on_transmit_started`]).
    pub fn on_octet(&mut self, byte: u8, now: Instant) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        if self.transmitting() {
            return Ok(());
        }
        let timed_out_candidate = self.abort_partial_if_late(now);
        self.note_bus_activity(now, 1)?;
        if timed_out_candidate {
            self.invalid_frame_event(now)?;
        }
        self.last_octet = Some(now);
        let result = self.decode_octet(byte, now);
        self.parser_partial = self.decoder.phase() != StreamPhase::Sync;
        result
    }

    /// Feed a frame already validated by a host-side chunk-tolerant assembler.
    ///
    /// `now` should be the adapter's best estimate of when the frame's final
    /// octet arrived: a reply to the frame is timed from it, and a late
    /// estimate leaves the requester less time than the core assumes.
    ///
    /// A frame the codec would not have decoded is refused with
    /// [`CoreError::Frame`] and changes nothing, not even the core's clock:
    /// more data than its type carries, source 255, or, without
    /// [`MstpCoreConfig::extended_frames`], a COBS-encoded type. A standard
    /// type carries up to 501 octets. With extended frames on, Types 32 and
    /// 33 carry up to 1497 and are taken as Types 5 and 6 are, and a
    /// reserved COBS type (34-127), whatever its length, is a valid frame of
    /// a type the core doesn't know (Clause 9.5.4.9): the assembler checked
    /// its CRC-32K.
    ///
    /// - **Echo.** Between a confirmed transmit start and its completion the
    ///   frame is the station's own echo and is ignored, as octets are. A
    ///   frame the assembler was part way through when
    ///   [`Self::on_transmit_started`] succeeded can never complete, since
    ///   the receiver is off while the driver is on: drop it there and then,
    ///   as the core drops its own decoder's partial frame. Report
    ///   completion before any frame assembled from what arrived after it.
    /// - **This station's address.** Clause 9 gives a frame whose source is
    ///   this station no rule of its own. Outside a transmission such a frame
    ///   is another station's using the same address, so the core counts it
    ///   in [`MstpCounters::own_address_frames`] as evidence of a duplicate
    ///   and handles it like any other frame. An adapter must therefore never
    ///   feed back its own echo once the transmission has completed.
    pub fn on_validated_frame(&mut self, frame: Frame<'_>, now: Instant) -> Result<(), CoreError> {
        let header = frame.header;
        let extended = self.config.extended_frames && header.frame_type.is_cobs();
        let maximum = match header.frame_type {
            _ if !extended => MAX_STANDARD_MPDU_DATA,
            frame_type if frame_type.is_extended_data() => MAX_EXTENDED_MPDU_DATA,
            // A reserved type's data is never used.
            _ => usize::MAX,
        };
        if frame.data.len() > maximum {
            return Err(CoreError::Frame(FrameError::Oversize {
                length: frame.data.len(),
                maximum,
            }));
        }
        if header.source == BROADCAST_MAC {
            return Err(CoreError::Frame(FrameError::BroadcastSource));
        }
        if header.frame_type.is_cobs() && !extended {
            return Err(CoreError::Frame(FrameError::UnsupportedCobsFrame {
                raw_type: header.frame_type.to_raw(),
            }));
        }
        let now = self.observe_time(now);
        if self.transmitting() {
            return Ok(());
        }
        self.note_bus_activity(now, 1)?;
        self.process_frame(frame, now)
    }

    /// Report a frame that a host-side assembler found invalid, as the core's
    /// own decoder reports one (Clause 9.5.4): `now` is when the trouble was
    /// found, usually the frame's last octet.
    ///
    /// Report a frame as invalid exactly when Clause 9.5.4 would count it for
    /// this station: a header that fails its checks wherever it was going,
    /// but trouble past the header (a bad data CRC, a length beyond this
    /// station's buffer, an encoding it can't take) only on a frame for this
    /// station or broadcast; skip any other. It counts in
    /// [`MstpCounters::parser_errors`] and as one event of line activity, and
    /// drives the transitions an invalid frame drives: WAIT_FOR_REPLY ends
    /// the token use (InvalidFrame, Clause 9.5.6.4), and POLL_FOR_MASTER
    /// takes the poll as unanswered (SoleMaster, DoneWithPFM, SendNextPFM or
    /// DeclareSoleMaster, Clause 9.5.6.8). In any other state it is line
    /// activity and nothing more: it restarts the line's silence and adds
    /// one to EventCount, so in PASS_TOKEN it can complete the evidence that
    /// the successor took the token (SawTokenUser, Clause 9.5.6.6), and in
    /// NO_TOKEN it ends the wait for a slot, as any activity does. While a
    /// frame of this station's waits to go out, it is only activity for that
    /// frame, which waits for a quiet line again, as for any octet. It is
    /// ignored as echo between a confirmed transmit start and its
    /// completion.
    pub fn on_invalid_frame(&mut self, now: Instant) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        if self.transmitting() {
            return Ok(());
        }
        self.note_bus_activity(now, 1)?;
        self.counters.record_parser_error();
        self.invalid_frame_event(now)
    }

    /// Report host-observed bus octets before chunk-tolerant frame assembly.
    ///
    /// Host serial APIs do not preserve reliable per-octet timestamps. This
    /// updates silence and token-use accounting without feeding the exact
    /// inter-octet parser used by [`Self::on_octet`]: each octet is one
    /// event toward EventCount, as an octet fed to `on_octet` is.
    ///
    /// Like [`Self::on_octet`], it is ignored as echo only once a transmit's
    /// start is confirmed.
    pub fn on_host_activity(&mut self, octets: usize, now: Instant) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        if self.transmitting() || octets == 0 {
            return Ok(());
        }
        self.note_bus_activity(now, u32::try_from(octets).unwrap_or(u32::MAX))
    }

    /// Report a UART receive failure as bus activity and abort partial input.
    ///
    /// A failure inside a frame counts as an invalid frame, a header or data
    /// in progress as well as a dropped frame the decoder is skipping by
    /// count, whatever its destination (Clause 9.5.4).
    ///
    /// It is taken for the station's own echo exactly when
    /// [`Self::on_octet`] would be: once a transmit's start is confirmed.
    /// Otherwise it counts under its kind, in
    /// [`MstpCounters::framing_errors`] and the three beside it.
    pub fn on_receive_error(&mut self, error: ReceiveError, now: Instant) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        if self.transmitting() {
            return Ok(());
        }
        self.counters.record_receive_error(error);
        self.note_bus_activity(now, 1)?;
        let invalid_candidate = matches!(
            self.decoder.phase(),
            StreamPhase::Header | StreamPhase::Data
        );
        self.decoder.reset();
        self.parser_partial = false;
        if invalid_candidate {
            self.counters.record_parser_error();
            self.invalid_frame_event(now)?;
        }
        Ok(())
    }

    /// Apply exact wire silence; a partial frame aborts only when silence is
    /// strictly greater than T_frame_abort. An aborted header or data,
    /// including a dropped frame being skipped by count, counts as an invalid
    /// frame whatever its destination (Clause 9.5.4).
    pub fn on_silence(&mut self, now: Instant) -> Result<(), CoreError> {
        let now = self.observe_time(now);
        if self.abort_partial_if_late(now) {
            self.invalid_frame_event(now)?;
        }
        Ok(())
    }
}

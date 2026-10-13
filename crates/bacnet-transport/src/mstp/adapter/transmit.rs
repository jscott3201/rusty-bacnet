//! Sending a frame the core handed over, and stripping its echo.
//!
//! A frame goes out in four steps, following the core's transmit contract:
//!
//! 1. Octets already waiting in the port are read without blocking and fed
//!    to the core, stamped with the time of the read, and so are octets the
//!    echo filter still holds in probe mode (they may be a peer's). They are
//!    line activity, and may make the core take the frame back. The read is
//!    polled once, outside Tokio's cooperative budget; octets that reached
//!    the port after the reactor last looked at it are not seen here, and
//!    are fed after the frame like any later octets.
//! 2. [`MasterCore::on_transmit_started`](bacnet_mstp_core::MasterCore::on_transmit_started); on any refusal the frame is
//!    dropped (a deferred one comes back as a new transmit), and the echo
//!    hunt goes on. Once the start is confirmed, what the filter still
//!    holds of the last frame's echo is dropped as its continuation, and
//!    counted.
//! 3. [`SerialPort::write`], then [`SerialPort::drain`]. The frame ended
//!    when the drain returned, but never before its time on the wire at the
//!    configured baud rate, counted from just before the write: one whose
//!    drain returns early (a USB adapter still shifting bits out) is held to
//!    it. A port without a drain is timed by that alone, plus
//!    [`MstpConfig::transmit_latency_pad`](crate::mstp::MstpConfig::transmit_latency_pad)
//!    for the time its first octet took to leave. A write or drain error
//!    aborts the frame.
//! 4. [`MasterCore::on_transmit_complete`](bacnet_mstp_core::MasterCore::on_transmit_complete), stamped with the frame's end.
//!    On a port with [`Echo::Full`] the echo filter (the `echo` module)
//!    hunts for the echo in what is read. Completion comes when the echo
//!    has matched, at the first octet the filter passes on, or when the
//!    grace runs out, whichever is first, and always before the core is fed
//!    anything read after the frame: until completion the core takes every
//!    octet for echo. The hunt goes on in the main loop after completion
//!    (see the `echo` module), so the task is back to servicing deadlines
//!    as soon as the frame is complete.
//!
//! Per port kind: `TokioSerialPort` writes to the driver and drains with
//! `tcdrain` on Unix. On Windows it has no drain: its write returns once the
//! system has the data, so the wire time plus the pad is used, which
//! defaults to the USB adapter's there. Its echo is whatever the hardware
//! does, `Echo::None` unless configured.
//! Kernel RS-485 is the same port with the kernel switching RTS around each
//! write. `GpioDirectionPort` drains inside its write and holds the driver
//! for its post-transmit delay before switching back, so the frame
//! completes when its write returns; its echo is the inner port's.
//! [`LoopbackSerial`](crate::mstp::LoopbackSerial) delivers at once and is
//! timed by the wire time, echoing only when built with an echo.

use bacnet_mstp_core::TransmitFailure;
use bacnet_types::error::Error;
use tokio::time::Instant;
use tracing::{debug, warn};

use super::{increment, poll_once, Core, Echo, Mac, Outgoing, ReadFailed, SentKind, SerialPort};

/// Reads at most in the non-blocking drain before a start, so a line that
/// never falls quiet cannot hold the task there.
const DRAIN_READS: usize = 8;

impl<S: SerialPort> Mac<S> {
    /// Send the frame the core handed over.
    pub(super) async fn transmit(&mut self, core: &mut Core<'_>) -> Result<(), ReadFailed> {
        self.drain_input(core).await?;
        // Octets held in probe mode may be a peer's: line activity, before
        // the start. What the other phases hold waits for a confirmed start,
        // since the core may still take the frame back or refuse it.
        let ended = self.filter.before_start(Instant::now());
        let now = self.clock.now();
        self.hunt_ended(core, ended, now);
        let Some(outgoing) = self.outgoing.take() else {
            return Ok(());
        };
        let started = Instant::now();
        if let Err(error) = core.on_transmit_started(outgoing.id, self.clock.at(started)) {
            debug!(?error, "MS/TP transmit start refused");
            return Ok(());
        }
        // What the last echo's hunt still holds was its continuation.
        if self.filter.confirmed_start() > 0 {
            increment(&self.counts.echo_fragments_dropped);
        }
        let octets = u32::try_from(self.tx_frame.len()).unwrap_or(u32::MAX);
        let wire_end = started + self.octet_time * octets;
        // The earliest the frame can have ended: its wire time, and with no
        // drain to say when the adapter finished, the transmit-latency pad
        // on top, since its first octet left after the write began.
        let sent = match self.serial.write(&self.tx_frame).await {
            Ok(()) => match self.serial.drain().await {
                Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::Unsupported => {
                    Ok(wire_end + self.transmit_pad)
                }
                drained => drained.map(|()| wire_end),
            },
            Err(error) => {
                // The failed write may have queued part of the frame.
                let _ = self.serial.drain().await;
                Err(error)
            }
        };
        let earliest_end = match sent {
            Ok(earliest_end) => earliest_end,
            Err(error) => {
                increment(&self.counts.serial_write_errors);
                warn!("MS/TP write error: {error}");
                let now = self.clock.now();
                if let Err(error) = core.on_transmit_aborted(outgoing.id, TransmitFailure::Io, now)
                {
                    debug!(?error, "MS/TP abort refused");
                }
                return Ok(());
            }
        };
        // The frame ended when the drain returned, or at that earliest end
        // if it is later. The timer may wake a little after that; the core
        // is told the end itself.
        let ended = earliest_end.max(Instant::now());
        if Instant::now() < ended {
            tokio::time::sleep_until(ended).await;
        }
        match self.echo {
            Echo::None => self.complete(core, &outgoing, ended),
            Echo::Full { grace } => self.strip_echo(core, &outgoing, ended, grace).await?,
        }
        Ok(())
    }

    /// Report the frame complete, stamped with its end.
    fn complete(&mut self, core: &mut Core<'_>, outgoing: &Outgoing, ended: Instant) {
        match core.on_transmit_complete(outgoing.id, self.clock.at(ended)) {
            Ok(()) => self.count_sent(outgoing.kind),
            Err(error) => debug!(?error, "MS/TP completion refused"),
        }
        self.drain_actions(core);
    }

    /// Hunt for the echo of the frame just sent until the frame can be
    /// completed: the echo matched, an octet passes on, or the grace ran out.
    /// Feed the core what passes on, after completion; the hunt goes on in
    /// the main loop.
    async fn strip_echo(
        &mut self,
        core: &mut Core<'_>,
        outgoing: &Outgoing,
        ended: Instant,
        grace: std::time::Duration,
    ) -> Result<(), ReadFailed> {
        self.filter.start(&self.tx_frame, ended, grace);
        let grace_end = ended + grace;
        loop {
            let read = tokio::select! {
                biased;
                read = self.serial.read(&mut self.read_buf) => Some(read),
                () = tokio::time::sleep_until(grace_end) => None,
            };
            match read.map(|read| self.outcome(read)).transpose()? {
                None => {}
                // Nothing read, short of the end of file: let other work, an
                // abort included, run.
                Some(0) => tokio::task::yield_now().await,
                Some(count) => {
                    let read_at = Instant::now();
                    let now = self.clock.now();
                    for index in 0..count {
                        let octet = self.read_buf[index];
                        let pushed = self.filter.push(octet, read_at);
                        if pushed.echo || pushed.passes() {
                            self.complete(core, outgoing, ended);
                            self.pass_on(core, pushed, octet, now);
                            self.receive(core, index + 1..count, now);
                            return Ok(());
                        }
                        if pushed.stale > 0 {
                            increment(&self.counts.echo_fragments_dropped);
                        }
                    }
                }
            }
            // Checked after every read too, so a port that is always
            // readable cannot hold the frame open past its grace. Empty
            // reads now end the task at the third, so this is nearly
            // unreachable for them; a read of held echo octets that lands
            // after the grace still needs it.
            if Instant::now() >= grace_end {
                let timed_out = self.filter.timer(Instant::now());
                self.complete(core, outgoing, ended);
                let now = self.clock.now();
                self.hunt_ended(core, timed_out, now);
                return Ok(());
            }
        }
    }

    /// Read without waiting and feed the core what is already there, until
    /// nothing is or the core takes the frame back.
    async fn drain_input(&mut self, core: &mut Core<'_>) -> Result<(), ReadFailed> {
        for _ in 0..DRAIN_READS {
            let Some(read) = poll_once(self.serial.read(&mut self.read_buf)).await else {
                return Ok(());
            };
            let count = self.outcome(read)?;
            if count == 0 {
                return Ok(());
            }
            let now = self.clock.now();
            self.receive(core, 0..count, now);
            if self.outgoing.is_none() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn count_sent(&self, kind: SentKind) {
        let counter = match kind {
            SentKind::DataExpectingReply => &self.counts.der_tx,
            SentKind::DirectAnswer => &self.counts.dner_tx_direct,
            SentKind::QueuedData => &self.counts.dner_tx_queued,
            SentKind::ReplyPostponed => &self.counts.reply_postponed_tx,
            SentKind::Other => return,
        };
        increment(counter);
    }
}

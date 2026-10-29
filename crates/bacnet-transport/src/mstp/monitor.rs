//! A receive-only MS/TP monitor: listen to a trunk without joining it.
//!
//! [`MstpMonitor`] reads a line, decodes it with the codec's
//! [`StreamDecoder`] and keeps [`MonitorStats`]: frames per station and
//! type (Clause 9.3), header and data CRC errors and other rejected frames,
//! token passes, rotation times and retries, lost tokens, each master's
//! Max_Master as inferred from its polls, and a histogram of the silences
//! between frames. The statistics are published through a
//! [`tokio::sync::watch`] channel, and every frame and error also goes out
//! on a bounded, lossy stream ([`MonitorFrames`]) for live views.
//!
//! # Receive only
//!
//! The monitor takes a [`MonitorRead`] input with explicit awaited close.
//! With the `serial` feature, `mstp_serial::ReceiveOnlyPort` uses native
//! read-only access. It configures no kernel RS-485 mode and touches no GPIO.
//! Driver/open transitions can still affect modem lines; this is not an
//! electrical-silence guarantee.
//!
//! # Timestamps
//!
//! Each read is stamped with the host's monotonic clock, and each frame's
//! time is worked back from its read at the configured baud rate. Times
//! are measured from when the monitor started. A USB adapter delays and
//! batches what it receives, so these times are only as good as
//! [`MonitorConfig::timestamp_resolution`], which the statistics carry in
//! [`TimingQuality`] for a user interface to show.
//!
//! # Extended frames
//!
//! COBS-encoded frames (Clause 9.10), the extended data frames and the
//! reserved COBS types, are decoded and their CRC-32K checked, but nothing
//! above that: [`MonitorRecord::is_decode_only`] marks them.
//!
//! ```
//! use bacnet_transport::mstp::codec::{Frame, FrameType};
//! use bacnet_transport::mstp::monitor::{DropOnClose, MonitorConfig, MstpMonitor};
//! use tokio::io::AsyncWriteExt;
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // A stand-in for the line: the monitor gets only the read side.
//! let (mut line, monitor_end) = tokio::io::duplex(1024);
//! let (reader, _writer) = tokio::io::split(monitor_end);
//! let mut monitor = MstpMonitor::start(DropOnClose::new(reader), MonitorConfig::new(38_400)).await?;
//! let mut stats = monitor.statistics();
//!
//! let mut wire = [0u8; 8];
//! Frame::new(FrameType::TOKEN, 2, 1, &[]).encode_into(&mut wire).unwrap();
//! line.write_all(&wire).await.unwrap();
//! stats.wait_for(|stats| stats.token_passes == 1).await.unwrap();
//! assert_eq!(stats.borrow().station(2).unwrap().tokens_received, 1);
//!
//! let last = monitor.stop().await?;
//! assert_eq!(last.present_stations().collect::<Vec<_>>(), [1]);
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use bacnet_mstp_codec::{MAX_COBS_FRAME_LENGTH, StreamDecoder};
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::io::AsyncReadExt;

mod admission;
pub use admission::MonitorAdmissionError;
mod reader;
pub use reader::{DropOnClose, MonitorRead};
pub(crate) use reader::{Failure, combine, context};
use tokio::sync::{broadcast, oneshot, watch};
use tokio::task::JoinHandle;

mod line;
mod stats;

pub use stats::{
    CountStats, DurationStats, FrameTypeCounts, LineErrorKind, LineErrors, LineEvent, LostToken,
    MaxMaster, MonitorStats, NO_TOKEN_SILENCE, SILENCE_BUCKET_BOUNDS, SilenceHistogram,
    StationStats, TimingQuality, octet_time,
};

/// [`MonitorConfig::timestamp_resolution`]'s default: 16 ms, the receive
/// latency FTDI USB adapters use unless told otherwise. Set it lower for a
/// native UART.
pub const DEFAULT_TIMESTAMP_RESOLUTION: Duration = Duration::from_millis(16);

/// [`MonitorConfig::record_capacity`]'s default.
pub const DEFAULT_RECORD_CAPACITY: usize = 1024;

/// Octets asked for per read.
const READ_LENGTH: usize = 512;

/// How the monitor reads the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MonitorConfig {
    /// The line's baud rate, which the monitor uses to time octets. The
    /// port itself must be opened at this rate.
    pub baud_rate: u32,
    /// How uncertain the host's timestamps are; see [`TimingQuality`].
    pub timestamp_resolution: Duration,
    /// How long the line may go quiet in the middle of a frame before the
    /// monitor gives the frame up. The default is the longest
    /// T_frame_abort Clause 9.5.3 allows, 100 ms, as an adapter can hold
    /// a frame's tail back for a while.
    pub frame_abort: Duration,
    /// Records the live stream keeps for a subscriber that falls behind;
    /// past that it loses the oldest.
    pub record_capacity: usize,
}

impl MonitorConfig {
    /// The defaults for a line at `baud_rate`.
    #[must_use]
    pub fn new(baud_rate: u32) -> Self {
        Self {
            baud_rate,
            timestamp_resolution: DEFAULT_TIMESTAMP_RESOLUTION,
            frame_abort: Duration::from_millis(super::HOST_FRAME_ABORT_MS),
            record_capacity: DEFAULT_RECORD_CAPACITY,
        }
    }

    /// The same, with `resolution` as the timestamp resolution.
    #[must_use]
    pub fn with_timestamp_resolution(mut self, resolution: Duration) -> Self {
        self.timestamp_resolution = resolution;
        self
    }

    /// Check the settings as [`MstpMonitor::start`] does.
    ///
    /// # Errors
    ///
    /// [`Error::Encoding`] for a zero baud rate, a zero frame-abort time,
    /// or a record capacity of zero or above `usize::MAX / 2`.
    pub fn validate(&self) -> Result<(), Error> {
        if self.baud_rate == 0 {
            return Err(Error::Encoding(
                "MS/TP monitor baud_rate must not be zero".into(),
            ));
        }
        if self.frame_abort.is_zero() {
            return Err(Error::Encoding(
                "MS/TP monitor frame_abort must not be zero".into(),
            ));
        }
        if self.record_capacity == 0 || self.record_capacity > usize::MAX / 2 {
            return Err(Error::Encoding(format!(
                "MS/TP monitor record_capacity {} is out of range",
                self.record_capacity
            )));
        }
        Ok(())
    }
}

/// One event on the live stream.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MonitorRecord {
    /// When its last octet arrived, from the monitor's start.
    pub at: Duration,
    /// What happened.
    pub event: LineEvent,
    /// A valid frame's data, decoded for a COBS-encoded one; empty for
    /// anything else.
    pub data: Bytes,
}

impl MonitorRecord {
    /// Whether this is a COBS-encoded frame (Clause 9.10), which the
    /// monitor decodes and checks but doesn't interpret further.
    #[must_use]
    pub fn is_decode_only(&self) -> bool {
        matches!(self.event, LineEvent::Frame { header, .. } if header.frame_type.is_cobs())
    }
}

/// A subscription to the live stream of frames and errors.
///
/// It is lossy: when this subscriber falls more than
/// [`MonitorConfig::record_capacity`] records behind, the oldest go, and
/// [`dropped`](Self::dropped) counts them. The monitor never waits for a
/// subscriber.
#[derive(Debug)]
pub struct MonitorFrames {
    receiver: broadcast::Receiver<MonitorRecord>,
    dropped: u64,
}

impl MonitorFrames {
    /// The next record, or `None` once the monitor has ended and every
    /// record has been read. Cancel safe.
    pub async fn recv(&mut self) -> Option<MonitorRecord> {
        loop {
            match self.receiver.recv().await {
                Ok(record) => return Some(record),
                Err(broadcast::error::RecvError::Lagged(lost)) => {
                    self.dropped = self.dropped.saturating_add(lost);
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// The next record if one is waiting.
    pub fn try_recv(&mut self) -> Option<MonitorRecord> {
        loop {
            match self.receiver.try_recv() {
                Ok(record) => return Some(record),
                Err(broadcast::error::TryRecvError::Lagged(lost)) => {
                    self.dropped = self.dropped.saturating_add(lost);
                }
                Err(_) => return None,
            }
        }
    }

    /// Records this subscriber lost by falling behind.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// A receive-only MS/TP monitor; see the [module docs](crate::mstp::monitor).
///
/// Dropping it requests shutdown without waiting. Await [`stop`](Self::stop)
/// to release the input; a cancelled stop can be resumed on the same monitor.
#[derive(Debug)]
pub struct MstpMonitor {
    statistics: watch::Receiver<MonitorStats>,
    records: broadcast::WeakSender<MonitorRecord>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<std::io::Result<()>>,
    completed: Option<Result<(), Failure>>,
}

impl MstpMonitor {
    /// Start monitoring what `reader` returns, on a task of the current
    /// Tokio runtime.
    ///
    /// The monitor only ever reads. It stops at the end of the stream, at
    /// a read error, at [`stop`](Self::stop) or when dropped.
    ///
    /// # Errors
    ///
    /// The errors of [`MonitorConfig::validate`] and closing rejected input.
    /// Check [`MonitorAdmissionError::release_confirmed`] before reopening.
    ///
    /// # Panics
    ///
    /// Outside a Tokio runtime.
    pub async fn start<R>(
        mut reader: R,
        config: MonitorConfig,
    ) -> Result<Self, MonitorAdmissionError>
    where
        R: MonitorRead + 'static,
    {
        if let Err(error) = config.validate() {
            let closed = std::future::poll_fn(|cx| reader.poll_close(cx)).await;
            return Err(MonitorAdmissionError::after_close(error, closed));
        }
        let (statistics_tx, statistics) = watch::channel(MonitorStats::new(
            config.baud_rate,
            config.timestamp_resolution,
        ));
        let (records, _) = broadcast::channel(config.record_capacity);
        let (stop, stop_rx) = oneshot::channel();
        let task = tokio::spawn(run(reader, config, statistics_tx, records.clone(), stop_rx));
        Ok(Self {
            statistics,
            records: records.downgrade(),
            stop: Some(stop),
            task,
            completed: None,
        })
    }

    /// The statistics, updated after every read. Don't hold a borrow of
    /// them for long: the monitor waits for it to update them.
    #[must_use]
    pub fn statistics(&self) -> watch::Receiver<MonitorStats> {
        self.statistics.clone()
    }

    /// Subscribe to the live stream of frames and errors, from now on.
    #[must_use]
    pub fn frames(&self) -> MonitorFrames {
        MonitorFrames {
            receiver: self
                .records
                .upgrade()
                .map_or_else(|| broadcast::channel(1).1, |records| records.subscribe()),
            dropped: 0,
        }
    }

    /// Whether the monitor's task has ended: the stream ended, a read
    /// failed, or it was stopped.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Stop the monitor, wait for its task to end and return the final
    /// statistics. Cancelling this future retains the shutdown and join state;
    /// call it again to await the same barrier. Repeated errors remain errors.
    ///
    /// # Errors
    ///
    /// A read, input-close or task failure. Statistics recorded before the
    /// failure remain in every [`statistics`](Self::statistics) receiver.
    pub async fn stop(&mut self) -> Result<MonitorStats, Error> {
        drop(self.stop.take());
        if self.completed.is_none() {
            let result = (&mut self.task).await.unwrap_or_else(|error| {
                Err(std::io::Error::other(format!(
                    "MS/TP monitor task failed: {error}"
                )))
            });
            self.completed = Some(result.map_err(Failure::new));
        }
        match self.completed.as_ref().expect("completion stored") {
            Ok(()) => Ok(self.statistics.borrow().clone()),
            Err(error) => Err(Error::Transport(error.error())),
        }
    }
}

impl Drop for MstpMonitor {
    fn drop(&mut self) {
        // Dropping the stop sender ends the task at its next wake.
        drop(self.stop.take());
    }
}

/// The monitor's task. `reader` is only ever read: this function has no
/// way to write to it.
async fn run<R>(
    mut reader: R,
    config: MonitorConfig,
    statistics: watch::Sender<MonitorStats>,
    records: broadcast::Sender<MonitorRecord>,
    mut stop: oneshot::Receiver<()>,
) -> std::io::Result<()>
where
    R: MonitorRead,
{
    let result = read_loop(&mut reader, config, statistics, records, &mut stop).await;
    let closed = std::future::poll_fn(|cx| reader.poll_close(cx)).await;
    combine(result, closed)
}

async fn read_loop<R: MonitorRead>(
    reader: &mut R,
    config: MonitorConfig,
    statistics: watch::Sender<MonitorStats>,
    records: broadcast::Sender<MonitorRecord>,
    stop: &mut oneshot::Receiver<()>,
) -> std::io::Result<()> {
    let started = tokio::time::Instant::now();
    let mut storage = vec![0u8; MAX_COBS_FRAME_LENGTH + READ_LENGTH];
    let mut decoder = StreamDecoder::new(&mut storage).with_reserved_cobs();
    let mut clock = line::LineClock::new(config.baud_rate, config.frame_abort);
    let mut buffer = [0u8; READ_LENGTH];
    loop {
        let count = tokio::select! {
            biased;
            _ = &mut *stop => return Ok(()),
            read = reader.read(&mut buffer) => read?,
        };
        if count == 0 {
            return Ok(());
        }
        let at = started.elapsed();
        let chunk = &buffer[..count];
        let live = records.receiver_count() > 0;
        statistics.send_modify(|stats| {
            stats.record_read(count);
            line::feed_chunk(&mut decoder, &mut clock, chunk, at, |at, event, frame| {
                stats.record(at, &event);
                if live {
                    let data =
                        frame.map_or_else(Bytes::new, |frame| Bytes::copy_from_slice(frame.data));
                    // Lossy by design: with no subscriber left, nothing to do.
                    let _ = records.send(MonitorRecord { at, event, data });
                }
            });
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod script;
#[cfg(test)]
mod stats_tests;
#[cfg(test)]
mod tests;

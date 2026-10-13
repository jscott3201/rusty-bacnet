//! MS/TP (Master-Slave/Token-Passing) transport over RS-485 (Clause 9).
//!
//! The master node state machine is the portable core in
//! [`bacnet_mstp_core`], and frames are encoded and decoded by
//! [`bacnet_mstp_codec`] (re-exported here as [`codec`]). This module adds
//! what a host needs around them:
//!
//! - [`SerialPort`], the serial I/O the transport runs over: `tokio-serial`,
//!   kernel RS-485 and GPIO direction control in `mstp_serial` (the
//!   `serial` feature), and [`LoopbackSerial`] and [`NoSerial`] for tests
//!   and non-MS/TP builds.
//! - [`MstpTransport`], whose Tokio task drives the core: it feeds every
//!   received octet to the core, services the core's deadlines with Tokio
//!   time, and carries out the core's actions (frames to send, NPDUs to
//!   deliver). The `adapter` module documents the details.
//! - [`MstpConfig`], mapped onto [`MstpCoreConfig`] and validated by the
//!   core's rules.
//! - [`MstpDiagnostics`], the core's counters added up ([`MstpCounters`])
//!   plus counts kept at the transport boundary.

use bacnet_mstp_core::{ConfigError, Duration as CoreDuration, MstpCoreConfig};
use bacnet_types::error::Error;
use tokio::sync::oneshot;

pub use bacnet_mstp_codec as codec;
mod adapter;
mod diagnostics;
mod echo;
mod queues;
mod replies;
mod totals;
pub use diagnostics::{MstpDiagnostics, MstpDiagnosticsSnapshot};
pub use totals::{MstpCounters, QueueCounters};

// ---------------------------------------------------------------------------
// Serial port abstraction
// ---------------------------------------------------------------------------

/// What a serial port's receiver hears while the port itself transmits.
///
/// RS-485 is half duplex. Whether a station hears its own frames depends on
/// how the transceiver is wired: with the receiver enable tied to the driver
/// enable (the usual MAX485 hookup) the receiver is off while the driver is
/// on, and some drivers and USB adapters filter their own output, so
/// nothing comes back. A transceiver whose receiver stays on returns every
/// octet the port sends.
///
/// The transport must know which: the core counts what it hears after a
/// transmission as line activity, and its own Token heard back would look
/// like the successor using the token. With [`Echo::Full`] the transport
/// compares what it reads after a frame with what it sent and drops the
/// octets that match, in order; the first octet that does not match, and
/// everything after it, goes to the core as line activity. It never drops
/// an octet it cannot match to the frame, so a peer's octets always reach
/// the core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Echo {
    /// Nothing the port sends comes back to its receiver.
    #[default]
    None,
    /// Every octet the port sends comes back to its receiver, in order.
    Full {
        /// How long after the frame's last octet its echo may still be
        /// arriving before the transport reports the frame complete
        /// regardless. An echo already under way then is still recognized
        /// and dropped as it continues, up to this station's next frame or
        /// T_frame_abort of silence; an echo not begun by then is looked for
        /// during one more grace, however long.
        /// [`Echo::full`] uses [`DEFAULT_ECHO_GRACE`].
        grace: std::time::Duration,
    },
}

/// The echo grace of [`Echo::full`]: room for a USB adapter's receive
/// latency, which holds received octets back for up to 16 ms by default on
/// FTDI parts, plus host scheduling.
pub const DEFAULT_ECHO_GRACE: std::time::Duration = std::time::Duration::from_millis(40);

impl Echo {
    /// [`Echo::Full`] with [`DEFAULT_ECHO_GRACE`].
    pub const fn full() -> Self {
        Self::Full {
            grace: DEFAULT_ECHO_GRACE,
        }
    }

    /// Check the setting as [`start`](crate::port::TransportPort::start)
    /// does.
    ///
    /// # Errors
    ///
    /// [`Error::Encoding`] for [`Echo::Full`] with a zero grace, in which no
    /// echo could arrive.
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Full { grace } if grace.is_zero() => Err(Error::Encoding(
                "MS/TP echo grace must not be zero".to_string(),
            )),
            Self::None | Self::Full { .. } => Ok(()),
        }
    }
}

/// Abstraction over an RS-485 serial port.
///
/// Implementations wrap the platform serial driver (e.g. `tokio-serial`).
/// [`LoopbackSerial`] is an in-memory implementation for tests.
///
/// The transport reads and writes from one task, never at the same time, and
/// may drop a pending [`read`](Self::read) future (to service a deadline, or
/// to look for octets that are already waiting just before it transmits), so
/// `read` must be cancel safe: a dropped read must lose no data.
pub trait SerialPort: Send + Sync + 'static {
    /// Write bytes to the serial port. Success may mean only driver acceptance,
    /// not that the final stop bit has left the UART; use [`Self::drain`] for that.
    ///
    /// The transport writes one whole frame per call, and only once the core
    /// has confirmed the frame may start: a port that controls the
    /// transceiver direction itself enables the line driver here.
    fn write(&self, data: &[u8]) -> impl std::future::Future<Output = Result<(), Error>> + Send;
    /// Wait until all accepted output has finished transmission, including the
    /// UART shift register. Required for software-controlled RS-485 direction.
    ///
    /// Call even after a write error, which may have accepted a partial frame.
    /// An error leaves transmit completion unknown. The default is unsupported,
    /// rather than falsely reporting completion for existing custom backends;
    /// the transport then takes the frame's time on the wire at the configured
    /// baud rate, plus [`MstpConfig::transmit_latency_pad`], as its
    /// completion.
    fn drain(&self) -> impl std::future::Future<Output = Result<(), Error>> + Send {
        async {
            Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Serial transmit-complete drain is not supported by this backend",
            )))
        }
    }
    /// Read the octets available into `buf`, which is never empty, and
    /// return how many were read.
    ///
    /// Wait until at least one octet has arrived. `Ok(0)` means the end of
    /// file: no more data will come, as when a pseudo-terminal's other end
    /// has closed, or on some platforms a USB adapter has been unplugged. It
    /// must not stand for "nothing yet", on a timeout or a spurious wake
    /// say. The transport ends its task after three `Ok(0)` reads in a row,
    /// as it does at the first read error: it counts the end in
    /// [`MstpDiagnosticsSnapshot::serial_read_errors`] and logs it, and the
    /// application finds the NPDU channel closed and its sends refused.
    fn read(
        &self,
        buf: &mut [u8],
    ) -> impl std::future::Future<Output = Result<usize, Error>> + Send;
    /// What the port's receiver hears while the port transmits; see [`Echo`].
    /// The default is [`Echo::None`].
    fn echo(&self) -> Echo {
        Echo::None
    }
}

/// Execution placement for an MS/TP transport, independent of master-node settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MstpExecutionMode {
    /// Spawn the MAC loop on the caller's Tokio runtime (the existing default).
    #[default]
    Tokio,
    /// Poll the same MAC loop on a dedicated OS thread with its own Tokio runtime.
    /// Blocking drain work uses that runtime's blocking pool, not the application's.
    /// This does not configure real-time scheduling or guarantee wire timing.
    DedicatedThread,
}

/// Owns the isolated reactor. Dropping the shutdown sender also handles cancelled
/// startup and abort; only `stop` waits for runtime and blocking-I/O completion.
struct DedicatedRuntime {
    handle: tokio::runtime::Handle,
    shutdown: oneshot::Sender<()>,
    finished: oneshot::Receiver<()>,
}

impl DedicatedRuntime {
    async fn new() -> Result<Self, Error> {
        let (started_tx, started_rx) = oneshot::channel();
        let (shutdown, shutdown_rx) = oneshot::channel();
        let (finished_tx, finished) = oneshot::channel();
        std::thread::Builder::new()
            .name("bacnet-mstp".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        if started_tx.send(Ok(runtime.handle().clone())).is_ok() {
                            let _ = runtime.block_on(shutdown_rx);
                        }
                        // Drop on the OS thread, never in the caller's async context.
                        // This waits for any already-started blocking drain calls.
                        drop(runtime);
                    }
                    Err(error) => {
                        let _ = started_tx.send(Err(error));
                    }
                }
                let _ = finished_tx.send(());
            })
            .map_err(Error::Transport)?;
        let handle = started_rx
            .await
            .map_err(|_| Error::Encoding("MS/TP execution thread failed to start".into()))?
            .map_err(Error::Transport)?;
        Ok(Self {
            handle,
            shutdown,
            finished,
        })
    }

    async fn stop(self) {
        drop(self.shutdown);
        let _ = self.finished.await;
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// T_frame_abort for a host serial port (Clause 9.5.3 allows up to 100 ms).
///
/// A host reads the line in chunks, and a USB adapter can hold a frame's
/// tail back for several milliseconds (16 ms by default on FTDI parts), so
/// the 60 bit times a UART-driven station uses would abort frames that are
/// still arriving. The core times each octet from the read that returned it.
/// It is [`bacnet_mstp_core::MAX_FRAME_ABORT`], in milliseconds.
pub const HOST_FRAME_ABORT_MS: u64 = bacnet_mstp_core::MAX_FRAME_ABORT.as_millis();

/// [`MstpConfig::usage_timeout`]'s default: 35 ms, the longest T_usage_timeout
/// Clause 9.5.3 allows ([`bacnet_mstp_core::MAX_USAGE_TIMEOUT`]).
///
/// It is how long this station waits, after passing the token or polling,
/// for the successor or the polled station to be heard. The successor may
/// take up to T_usage_delay (15 ms) to begin, and on a host its first octets
/// then come through the same adapter that delays a frame's tail (16 ms by
/// default on FTDI parts), so the 20 ms minimum can run out while the answer
/// is still in the adapter: the station would send the token again over it,
/// or poll the next address. A USB adapter can't be told apart from a native
/// UART on every platform, so every host port gets the longest value.
pub const HOST_USAGE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_nanos(bacnet_mstp_core::MAX_USAGE_TIMEOUT.as_nanos());

/// [`MstpConfig::reply_timeout`]'s default: 300 ms, the longest
/// T_reply_timeout Clause 9.5.3 allows
/// ([`bacnet_mstp_core::MAX_REPLY_TIMEOUT`]).
///
/// It is how long this station waits for an answer to a request it sent. A
/// peer may begin its answer as late as T_reply_delay (250 ms), and on a
/// host the answer's first octets reach the station late by the adapter's
/// receive latency, so the 255 ms minimum leaves 5 ms where 300 ms leaves 50.
/// As for [`HOST_USAGE_TIMEOUT`], every host port gets the longest value.
pub const HOST_REPLY_TIMEOUT: std::time::Duration =
    std::time::Duration::from_nanos(bacnet_mstp_core::MAX_REPLY_TIMEOUT.as_nanos());

/// The transmit-latency pad for a USB serial adapter, and
/// [`MstpConfig::transmit_latency_pad`]'s default on Windows: 4 ms.
///
/// A port with no drain can't say when its last stop bit left, so the
/// transport counts the frame's time on the wire from just before the write.
/// Through a USB adapter the first octet leaves later than that: USB full
/// speed runs in 1 ms frames, and a host transfer requested between two
/// frames waits for the next one to begin (FTDI's application note
/// AN232B-04, "Data Throughput, Latency and Handshaking"; Silicon Labs
/// CP210x and WCH CH340 parts are full-speed devices too). The pad allows
/// one frame of waiting, one frame for the transfer itself, and 2 ms for the
/// host controller and the serial driver to queue it. Unlike the receive
/// side, where FTDI's latency timer holds back a short packet for 16 ms by
/// default, nothing in this path waits for more data. A driver or a loaded
/// host slower than that needs a larger pad, up to T_usage_delay (15 ms).
pub const USB_TRANSMIT_LATENCY_PAD: std::time::Duration = std::time::Duration::from_millis(4);

/// Master node configuration.
///
/// [`MstpTransport::start`](crate::port::TransportPort::start) maps it onto
/// the core's [`MstpCoreConfig`] and refuses a configuration the core
/// refuses; [`Self::validate`] runs the same checks without starting. Every
/// Clause 9 timer the core uses comes from the baud rate and the fixed
/// Clause 9.5.3 values, except three that 9.5.3 lets a station lengthen and
/// a host needs longer, for the latency of a USB adapter: T_frame_abort,
/// fixed at its 100 ms maximum ([`HOST_FRAME_ABORT_MS`]), and
/// T_usage_timeout and T_reply_timeout, which default to their maxima
/// ([`Self::usage_timeout`], [`Self::reply_timeout`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MstpConfig {
    /// This station's MAC address (0..=max_master).
    pub this_station: u8,
    /// Highest master address on the link, Max_Master (at most 127).
    pub max_master: u8,
    /// Max_Info_Frames: information frames per token use (1..=255).
    pub max_info_frames: u8,
    /// Baud rate, from which the bit-time timers (T_turnaround and the
    /// rest) are derived (default: 9600). The core needs at least 2667 baud,
    /// so that 40 bit times of turnaround fit inside T_usage_delay.
    pub baud_rate: u32,
    /// Take and send extended frames, Types 32 and 33, for NPDUs of up to
    /// 1497 octets (default: off). Clause 9.3 requires them of a routing node and
    /// leaves them optional for any other, so `bacnet_network`'s
    /// `BACnetRouter` turns this on for every MS/TP port it routes (see
    /// [`TransportPort::prepare_for_routing`](crate::port::TransportPort::prepare_for_routing)),
    /// and a device keeps it off unless set here. With it on the port takes
    /// APDUs of 1476 octets instead of 480, and its transmit watchdog covers
    /// the longest extended frame
    /// ([`MstpCoreConfig::with_extended_frames`]). See
    /// [`MstpCoreConfig::extended_frames`] for what the core does with it.
    ///
    /// Memory per port: the queues reserve 512 slots, 768 KiB with the flag
    /// on (1536-octet slots, for 1497-octet NPDUs) against 272 KiB without
    /// (544-octet slots); pages a queue never fills usually don't become
    /// resident. The core's action buffers, about 13 KiB, are sized for
    /// extended frames either way.
    pub extended_frames: bool,
    /// T_usage_timeout: how long after passing the token, or after a poll,
    /// this station waits to hear the successor or the polled station.
    /// Clause 9.5.3 allows 20 to 35 ms, and [`Self::validate`] refuses
    /// anything else. Default: [`HOST_USAGE_TIMEOUT`], 35 ms, which leaves
    /// room for a USB adapter's receive latency.
    pub usage_timeout: std::time::Duration,
    /// T_reply_timeout: how long after a data-expecting-reply frame this
    /// station waits for the answer to begin. Clause 9.5.3 allows 255 to
    /// 300 ms, and [`Self::validate`] refuses anything else. Default:
    /// [`HOST_REPLY_TIMEOUT`], 300 ms.
    pub reply_timeout: std::time::Duration,
    /// Added to a frame's time on the wire when the port has no drain
    /// ([`SerialPort::drain`] returns `Unsupported`), so that the frame is
    /// not taken to have ended before its last stop bit left the adapter.
    ///
    /// The frame then ends no sooner than its write began plus its octets'
    /// time at the baud rate plus this pad. That end starts T_turnaround and
    /// the reply and usage timeouts, so a pad too short starts them early.
    /// One too long delays this station's next frame, and what the station
    /// hears while the pad runs reaches the core only after it. A port whose
    /// drain works (`tcdrain` on Unix) is not padded.
    ///
    /// Default: [`USB_TRANSMIT_LATENCY_PAD`] on Windows, where
    /// `TokioSerialPort` has no drain and the port is usually a USB adapter,
    /// and zero elsewhere. [`Self::validate`] refuses a pad over
    /// T_usage_delay (15 ms), past which a token or a poll that arrived
    /// during the pad could only be answered late (Clause 9.5.3). The core's
    /// transmit watchdog is lengthened by the pad where it would otherwise
    /// not cover the longest padded frame.
    pub transmit_latency_pad: std::time::Duration,
}

impl Default for MstpConfig {
    fn default() -> Self {
        Self {
            this_station: 0,
            max_master: codec::MAX_MASTER,
            max_info_frames: 1,
            baud_rate: 9600,
            extended_frames: false,
            usage_timeout: HOST_USAGE_TIMEOUT,
            reply_timeout: HOST_REPLY_TIMEOUT,
            transmit_latency_pad: if cfg!(windows) {
                USB_TRANSMIT_LATENCY_PAD
            } else {
                std::time::Duration::ZERO
            },
        }
    }
}

impl MstpConfig {
    /// Check the configuration as [`start`](crate::port::TransportPort::start)
    /// does, without opening anything.
    ///
    /// # Errors
    ///
    /// [`Error::Encoding`] naming the field: `max_master` above 127,
    /// `this_station` above `max_master`, `max_info_frames` of zero, a
    /// `baud_rate` of zero or too low for Clause 9 timing, a `usage_timeout`
    /// or `reply_timeout` outside what Clause 9.5.3 allows, or a
    /// `transmit_latency_pad` over T_usage_delay (15 ms).
    pub fn validate(&self) -> Result<(), Error> {
        self.core_config().map(drop)
    }

    /// The longest NPDU the port sends or takes: 1497 octets with
    /// [`Self::extended_frames`] on, 501 otherwise.
    pub fn max_npdu_length(&self) -> usize {
        if self.extended_frames {
            codec::MAX_EXTENDED_MPDU_DATA
        } else {
            codec::MAX_STANDARD_MPDU_DATA
        }
    }

    /// The longest APDU the port carries, as the network layer sees it:
    /// [`Self::max_npdu_length`] less room for the network header, 1476
    /// octets with extended frames and 480 without.
    pub fn max_apdu_length(&self) -> u16 {
        if self.extended_frames {
            1476
        } else {
            480
        }
    }

    /// The core configuration this one maps onto.
    pub(crate) fn core_config(&self) -> Result<MstpCoreConfig, Error> {
        let mut config = MstpCoreConfig::for_baud_rate(self.baud_rate)
            .map_err(|error| self.config_error(error))?;
        config.station = self.this_station;
        config.max_master = self.max_master;
        config.max_info_frames = self.max_info_frames;
        config.t_frame_abort = CoreDuration::from_millis(HOST_FRAME_ABORT_MS);
        config.t_usage_timeout = CoreDuration::saturating_from_core(self.usage_timeout);
        config.t_reply_timeout = CoreDuration::saturating_from_core(self.reply_timeout);
        if self.extended_frames {
            config = config.with_extended_frames();
        }
        config
            .validate()
            .map_err(|error| self.config_error(error))?;
        // Octets that arrive while the pad runs reach the core only after
        // it, so a pad past T_usage_delay would make the station answer a
        // token or a poll late however quick the rest of the host is.
        let pad = CoreDuration::saturating_from_core(self.transmit_latency_pad);
        if pad > config.t_usage_delay {
            return Err(Error::Encoding(format!(
                "MS/TP transmit_latency_pad {:?} exceeds T_usage_delay ({:?})",
                self.transmit_latency_pad,
                std::time::Duration::from(config.t_usage_delay)
            )));
        }
        // A padded frame must complete inside the watchdog too, or the core
        // would abort a frame still on its way out: give it the pad.
        config.transmit_watchdog = config
            .transmit_watchdog
            .max(config.minimum_transmit_watchdog().saturating_add(pad));
        Ok(config)
    }

    fn config_error(&self, error: ConfigError) -> Error {
        let message = match error {
            ConfigError::MaxMasterOutOfRange { value } => {
                format!("MS/TP max_master {value} exceeds MAX_MASTER (127)")
            }
            ConfigError::StationExceedsMaxMaster {
                station,
                max_master,
            } => {
                format!("MS/TP this_station {station} exceeds configured max_master ({max_master})")
            }
            ConfigError::MaxInfoFramesOutOfRange { .. } => {
                "MS/TP max_info_frames must be in 1..=255".to_string()
            }
            ConfigError::ZeroBaudRate => "MS/TP baud_rate must not be zero".to_string(),
            ConfigError::UsageTimeoutOutOfRange {
                minimum, maximum, ..
            } => format!(
                "MS/TP usage_timeout {:?} is outside {:?}..={:?} (Clause 9.5.3)",
                self.usage_timeout,
                std::time::Duration::from(minimum),
                std::time::Duration::from(maximum)
            ),
            ConfigError::ReplyTimeoutOutOfRange {
                minimum, maximum, ..
            } => format!(
                "MS/TP reply_timeout {:?} is outside {:?}..={:?} (Clause 9.5.3)",
                self.reply_timeout,
                std::time::Duration::from(minimum),
                std::time::Duration::from(maximum)
            ),
            other => format!(
                "MS/TP baud_rate {} is outside what Clause 9 timing allows ({other:?})",
                self.baud_rate
            ),
        };
        Error::Encoding(message)
    }
}

mod port;
pub use port::{LoopbackSerial, MstpTransport, NoSerial};

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod clause9_token_tests;
#[cfg(test)]
mod clause9_wait_tests;
#[cfg(test)]
mod diagnostics_tests;
#[cfg(test)]
mod echo_tests;
#[cfg(test)]
pub(crate) mod execution_tests;
#[cfg(test)]
mod extended_tests;
#[cfg(test)]
mod host_timing_tests;
#[cfg(test)]
mod late_answer_tests;
#[cfg(test)]
mod late_echo_tests;
#[cfg(test)]
mod reply_tests;
#[cfg(test)]
mod slow_adapter_tests;
#[cfg(test)]
mod tests;

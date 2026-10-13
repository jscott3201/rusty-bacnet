use bacnet_mstp_codec::BROADCAST_MAC;
use bacnet_mstp_core::{Duration as CoreDuration, QueueRefusal};
use bacnet_types::error::Error;
use tokio::sync::{mpsc, Mutex};

use super::adapter::Task;
use super::diagnostics::increment;
use super::queues::{CoreClock, EnqueueError, SharedQueues};
use super::{DedicatedRuntime, Echo, MstpConfig, MstpDiagnostics, MstpExecutionMode, SerialPort};
use crate::port::{ReceivedNpdu, TransportPort};

// ---------------------------------------------------------------------------
// MS/TP Transport
// ---------------------------------------------------------------------------

/// MS/TP transport implementing [`TransportPort`].
///
/// [`start`](TransportPort::start) validates the [`MstpConfig`] and spawns a
/// task that owns the serial port and runs the portable master core (see the
/// [module docs](super)). Sends queue NPDUs for the core, which sends them
/// when this station holds the token; a full queue or an NPDU longer than
/// the port sends ([`Self::max_npdu_length`]) is refused at once.
///
/// `bacnet_network`'s `BACnetRouter` turns on
/// [`MstpConfig::extended_frames`] for every MS/TP port it routes,
/// through [`TransportPort::prepare_for_routing`], since Clause 9.3 requires
/// them of a routing node; any other use keeps the configured setting.
pub struct MstpTransport<S: SerialPort> {
    serial: Option<S>,
    config: MstpConfig,
    local_mac: [u8; 1],
    link: Option<Link>,
    task: Option<tokio::task::JoinHandle<()>>,
    execution_mode: MstpExecutionMode,
    dedicated_runtime: Option<DedicatedRuntime>,
    diagnostics: MstpDiagnostics,
}

/// What a send needs from a started transport.
struct Link {
    queues: SharedQueues,
    clock: CoreClock,
    queue_max_age: CoreDuration,
}

impl<S: SerialPort> MstpTransport<S> {
    /// Create an MS/TP transport over `serial`; the node starts when the transport is started.
    pub fn new(serial: S, config: MstpConfig) -> Self {
        let mac = config.this_station;
        Self {
            serial: Some(serial),
            config,
            local_mac: [mac],
            link: None,
            task: None,
            execution_mode: MstpExecutionMode::default(),
            dedicated_runtime: None,
            diagnostics: MstpDiagnostics::new(),
        }
    }

    /// Configure execution placement for the next `start`; call before starting.
    /// The default is [`MstpExecutionMode::Tokio`]. No serial or master-node
    /// settings change, and an already-running loop is not migrated.
    ///
    /// Dedicated execution isolates MAC polling and backend blocking work from
    /// the application pool, not from OS scheduling delays. Async serial resources
    /// opened on another reactor still require that reactor to remain running.
    /// `stop` waits for teardown; `abort`/drop request cancellation without waiting.
    /// Neither can interrupt an in-progress blocking syscall or undo queued bytes.
    pub fn with_execution_mode(mut self, mode: MstpExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Obtain a cloneable, counts-only handle before transferring transport ownership.
    /// Cloning retains no serial or runtime ownership; snapshots remain readable
    /// after stop/drop. See [`MstpDiagnostics`] for lifecycle and coherence limits.
    ///
    /// ```
    /// use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
    /// let (serial, _peer) = LoopbackSerial::pair();
    /// let transport = MstpTransport::new(serial, MstpConfig::default());
    /// let diagnostics = transport.diagnostics();
    /// let observer = diagnostics.clone();
    /// let before = diagnostics.snapshot();
    /// // Move `transport` into your BACnetRouter/endpoint here. The observer
    /// // remains independent; no second serial open or TransportPort API needed.
    /// let owned_transport = transport;
    /// drop(owned_transport);
    /// assert_eq!(observer.snapshot(), before);
    /// ```
    pub fn diagnostics(&self) -> MstpDiagnostics {
        self.diagnostics.clone()
    }

    /// The configuration the next `start` uses: what [`Self::new`] was
    /// given, with extended frames on if a router prepared the port
    /// ([`TransportPort::prepare_for_routing`]).
    pub fn config(&self) -> &MstpConfig {
        &self.config
    }

    /// The longest NPDU the port sends or takes, as the network layer sees
    /// it: 1497 octets with extended frames, 501 without
    /// ([`MstpConfig::max_npdu_length`]).
    pub fn max_npdu_length(&self) -> usize {
        self.config.max_npdu_length()
    }

    fn abort_task_and_release_state(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.dedicated_runtime.take();
        self.serial.take();
        self.link.take();
    }

    /// Queue an NPDU for the core, refusing it at once if it cannot be sent.
    fn enqueue(&self, destination: u8, npdu: &[u8]) -> Result<(), Error> {
        let Some(link) = &self.link else {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "MS/TP transport not started",
            )));
        };
        let result = link
            .queues
            .enqueue(destination, npdu, link.clock.now(), link.queue_max_age);
        let counts = &self.diagnostics.counters;
        counts.publish_queues(&link.queues);
        match result {
            Ok(()) => Ok(()),
            Err(EnqueueError::Closed) => Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "MS/TP transport is no longer running",
            ))),
            Err(EnqueueError::Rejected(QueueRefusal::Oversize)) => {
                increment(&counts.outbound_oversize);
                let class = if self.config.extended_frames {
                    "extended"
                } else {
                    "standard"
                };
                Err(Error::Encoding(format!(
                    "MS/TP NPDU length {} exceeds {class}-frame maximum {}",
                    npdu.len(),
                    self.config.max_npdu_length()
                )))
            }
            Err(EnqueueError::Rejected(QueueRefusal::Full)) => {
                increment(&counts.outbound_queue_full);
                Err(Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "MS/TP TX queue full",
                )))
            }
            Err(EnqueueError::Rejected(reason)) => Err(Error::Transport(std::io::Error::other(
                format!("MS/TP NPDU not queued: {reason:?}"),
            ))),
        }
    }
}

impl<S: SerialPort> TransportPort for MstpTransport<S> {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        true
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        /// NPDU receive channel capacity — smaller than BIP/Ethernet for low-bandwidth serial.
        const NPDU_CHANNEL_CAPACITY: usize = 64;

        let config = self.config.core_config()?;
        if let Some(serial) = &self.serial {
            serial.echo().validate()?;
        }
        let serial = self
            .serial
            .take()
            .ok_or_else(|| Error::Encoding("MS/TP transport already started".into()))?;

        let dedicated_runtime = match self.execution_mode {
            MstpExecutionMode::Tokio => None,
            MstpExecutionMode::DedicatedThread => Some(DedicatedRuntime::new().await?),
        };

        let (npdu_tx, npdu_rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
        let queues = SharedQueues::new(self.config.extended_frames);
        let clock = CoreClock::new();
        let task = Task {
            serial,
            config,
            queues: queues.clone(),
            clock,
            npdus: npdu_tx,
            counts: self.diagnostics.counters.clone(),
            transmit_pad: self.config.transmit_latency_pad,
        };
        let task = match &dedicated_runtime {
            Some(runtime) => runtime.handle.spawn(task.run()),
            None => tokio::spawn(task.run()),
        };
        self.dedicated_runtime = dedicated_runtime;
        self.task = Some(task);
        self.link = Some(Link {
            queues,
            clock,
            queue_max_age: config.queue_max_age,
        });
        Ok(npdu_rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(runtime) = self.dedicated_runtime.take() {
            runtime.stop().await;
        }
        // Queued NPDUs go with the task; later sends are refused.
        self.link = None;
        Ok(())
    }

    fn abort(&mut self) {
        self.abort_task_and_release_state();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let [destination] = mac else {
            return Err(Error::Encoding(format!(
                "MS/TP MAC must be 1 byte, got {}",
                mac.len()
            )));
        };
        self.enqueue(*destination, npdu)
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.enqueue(BROADCAST_MAC, npdu)
    }

    /// Turns on extended frames, which Clause 9.3 requires of a routing
    /// node, unless the transport has started already.
    fn prepare_for_routing(&mut self) {
        if self.serial.is_some() {
            self.config.extended_frames = true;
        }
    }

    /// [`MstpConfig::max_apdu_length`]: 1476 with extended frames, 480
    /// without.
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.config.max_apdu_length()
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// [`MstpConfig::max_apdu_length`], as for the receive capacity.
    fn egress_apdu_limit(&self) -> u16 {
        self.config.max_apdu_length()
    }

    /// [`MstpConfig::max_npdu_length`] both ways: 1497 octets with extended
    /// frames, 501 without.
    fn max_npdu(&self) -> crate::port::NpduLimits {
        let octets = u16::try_from(self.config.max_npdu_length()).unwrap_or(u16::MAX);
        crate::port::NpduLimits::both(octets)
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        mac == [BROADCAST_MAC]
    }

    fn group_destinations(&self) -> crate::port::GroupDestinations {
        crate::port::GroupDestinations::new(|mac| mac == [BROADCAST_MAC])
    }
}

impl<S: SerialPort> Drop for MstpTransport<S> {
    fn drop(&mut self) {
        self.abort_task_and_release_state();
    }
}

// ---------------------------------------------------------------------------
// Loopback serial port for testing
// ---------------------------------------------------------------------------

/// In-memory loopback serial port for unit testing.
///
/// [`Self::pair`] connects two ports: what one writes, the other reads, in
/// whole writes (an empty write delivers nothing, and a read always returns
/// at least one octet). Writes are delivered at once, so a frame takes no
/// time on this "wire"; the transport times it by the baud rate (see the
/// [module docs](super)). By default a port does not hear its own writes;
/// [`Self::with_echo`] makes it hear them, as a transceiver whose receiver
/// stays on does.
pub struct LoopbackSerial {
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    tx: mpsc::Sender<Vec<u8>>,
    /// This port's own receive channel, for its echo. Weak, so that a port
    /// still sees its peer go away.
    own: mpsc::WeakSender<Vec<u8>>,
    echo: Echo,
    /// Leftover bytes from a previous read that didn't fit in the caller's buffer.
    leftover: Mutex<Vec<u8>>,
}

impl LoopbackSerial {
    /// Create a pair of connected loopback serial ports.
    pub fn pair() -> (Self, Self) {
        let (tx_a, rx_b) = mpsc::channel(64);
        let (tx_b, rx_a) = mpsc::channel(64);
        let own_a = tx_b.downgrade();
        let own_b = tx_a.downgrade();
        (
            Self {
                rx: Mutex::new(rx_a),
                tx: tx_a,
                own: own_a,
                echo: Echo::None,
                leftover: Mutex::new(Vec::new()),
            },
            Self {
                rx: Mutex::new(rx_b),
                tx: tx_b,
                own: own_b,
                echo: Echo::None,
                leftover: Mutex::new(Vec::new()),
            },
        )
    }

    /// Set whether this port hears its own writes. With [`Echo::Full`] each
    /// write is delivered to this port's own reader first, then to the peer;
    /// its grace is what the port reports to the transport.
    pub fn with_echo(mut self, echo: Echo) -> Self {
        self.echo = echo;
        self
    }
}

impl SerialPort for LoopbackSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        if matches!(self.echo, Echo::Full { .. }) {
            if let Some(own) = self.own.upgrade() {
                own.send(data.to_vec())
                    .await
                    .map_err(|_| Error::Encoding("loopback echo failed".into()))?;
            }
        }
        self.tx
            .send(data.to_vec())
            .await
            .map_err(|_| Error::Encoding("loopback write failed".into()))
    }

    async fn drain(&self) -> Result<(), Error> {
        // In-memory writes are delivered before returning; there is no UART.
        Ok(())
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        // Hold the remainder's lock across the receive, so nothing taken off
        // the channel waits on a later await: a read dropped at any await
        // point loses no data.
        let mut leftover = self.leftover.lock().await;
        if !leftover.is_empty() {
            let len = leftover.len().min(buf.len());
            buf[..len].copy_from_slice(&leftover[..len]);
            leftover.drain(..len);
            return Ok(len);
        }
        let mut rx = self.rx.lock().await;
        loop {
            match rx.recv().await {
                // An empty write puts nothing on the line, and a read that
                // returns nothing would mean the end of it.
                Some(data) if data.is_empty() => {}
                Some(data) => {
                    let len = data.len().min(buf.len());
                    buf[..len].copy_from_slice(&data[..len]);
                    // Keep what did not fit for the next read.
                    leftover.extend_from_slice(&data[len..]);
                    return Ok(len);
                }
                None => return Err(Error::Encoding("loopback channel closed".into())),
            }
        }
    }

    fn echo(&self) -> Echo {
        self.echo
    }
}

// ---------------------------------------------------------------------------
// NoSerial: zero-sized SerialPort for non-MS/TP contexts
// ---------------------------------------------------------------------------

/// A serial port implementation that always errors.
///
/// Used to satisfy the `AnyTransport<S>` generic when MS/TP is not needed
/// (e.g., in Python bindings where serial access isn't exposed).
pub struct NoSerial;

impl SerialPort for NoSerial {
    async fn write(&self, _data: &[u8]) -> Result<(), Error> {
        Err(Error::Encoding("NoSerial: MS/TP not available".into()))
    }

    async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
        Err(Error::Encoding("NoSerial: MS/TP not available".into()))
    }
}

#[cfg(test)]
mod abort_tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    struct ExclusiveSerial(Arc<AtomicBool>);

    impl ExclusiveSerial {
        fn open(in_use: Arc<AtomicBool>) -> Option<Self> {
            in_use
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .ok()
                .map(|_| Self(in_use))
        }
    }

    impl Drop for ExclusiveSerial {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }

    impl SerialPort for ExclusiveSerial {
        async fn write(&self, _data: &[u8]) -> Result<(), Error> {
            Ok(())
        }

        async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
            pending().await
        }
    }

    async fn reopen(in_use: &Arc<AtomicBool>) -> ExclusiveSerial {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(serial) = ExclusiveSerial::open(in_use.clone()) {
                    break serial;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("aborted receive task retained the serial resource")
    }

    #[tokio::test]
    async fn abort_takes_task_and_releases_serial_for_reopen() {
        let in_use = Arc::new(AtomicBool::new(false));
        let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
        let mut transport = MstpTransport::new(serial, MstpConfig::default());
        let _rx = transport.start().await.unwrap();
        assert!(ExclusiveSerial::open(in_use.clone()).is_none());

        transport.abort();
        assert!(transport.task.is_none());
        assert!(transport.link.is_none());
        assert!(transport.send_broadcast(&[1, 0]).await.is_err());
        drop(reopen(&in_use).await);
        assert!(!in_use.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn drop_aborts_task_and_releases_serial_for_reopen() {
        let in_use = Arc::new(AtomicBool::new(false));
        let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
        let mut transport = MstpTransport::new(serial, MstpConfig::default());
        let _rx = transport.start().await.unwrap();

        drop(transport);
        drop(reopen(&in_use).await);
        assert!(!in_use.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_refused_configuration_keeps_the_serial_port() {
        let in_use = Arc::new(AtomicBool::new(false));
        let serial = ExclusiveSerial::open(in_use.clone()).unwrap();
        let config = MstpConfig {
            this_station: 9,
            max_master: 7,
            ..MstpConfig::default()
        };
        let mut transport = MstpTransport::new(serial, config);
        let error = transport.start().await.unwrap_err();
        assert!(error.to_string().contains("exceeds configured max_master"));
        assert!(transport.serial.is_some(), "the serial port was consumed");
    }
}

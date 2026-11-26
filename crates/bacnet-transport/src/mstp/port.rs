use bacnet_mstp_codec::BROADCAST_MAC;
use bacnet_mstp_core::{Duration as CoreDuration, QueueRefusal};
use bacnet_time::HostClock;
use bacnet_types::data_link::DataLink;
use bacnet_types::error::{Error, Lifecycle, Refusal};
use bacnet_types::sync::MutexExt;
use std::sync::Mutex as StdMutex;
use tokio::sync::{Mutex, mpsc};

use super::adapter::Task;
use super::diagnostics::increment;
use super::queues::{EnqueueError, SharedQueues};
use super::{
    BoxedSerial, DedicatedRuntime, Echo, MstpConfig, MstpDiagnostics, MstpExecutionMode,
    MstpExecutionStatus, SerialPort,
};
use crate::port::{ReceivedNpdu, TransportPort};
use crate::runtime_clock::tokio_clock;
use crate::stopping::StopGate;

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
    /// What a send needs, from start on. A stop leaves it and closes its
    /// queues, so every later send is refused (#2617).
    link: Option<Link>,
    /// The node's task and its thread, which a stop through `&self` ends;
    /// only `start`, `stop` and `abort` lock it.
    running: StdMutex<Running>,
    /// One stop at a time (#2617).
    stop_gate: StopGate,
    execution_mode: MstpExecutionMode,
    diagnostics: MstpDiagnostics,
    /// See [`Self::frame_tap`].
    frame_tap: crate::tap::FrameTap,
}

/// The started node: its task and, in dedicated execution, its thread. A
/// stop takes each only once it has ended, so a cancelled stop leaves the
/// rest to the next.
#[derive(Default)]
struct Running {
    task: Option<tokio::task::JoinHandle<()>>,
    runtime: Option<DedicatedRuntime>,
}

/// The node's task, which a stop waits for: back in [`Running`] if the
/// wait is cancelled.
struct Joining<'a> {
    running: &'a StdMutex<Running>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Joining<'_> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            lock(self.running).task = Some(task);
        }
    }
}

fn lock(running: &StdMutex<Running>) -> std::sync::MutexGuard<'_, Running> {
    running.locked()
}

/// What a send needs from a started transport.
struct Link {
    queues: SharedQueues,
    clock: HostClock,
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
            running: StdMutex::default(),
            stop_gate: StopGate::default(),
            execution_mode: MstpExecutionMode::default(),
            diagnostics: MstpDiagnostics::new(),
            frame_tap: crate::tap::FrameTap::new(),
        }
    }

    /// Configure execution placement for the next `start`; call before starting.
    /// The default is [`MstpExecutionMode::Tokio`]. No serial or master-node
    /// settings change, and an already-running loop is not migrated.
    ///
    /// Dedicated execution isolates MAC polling and backend blocking work from
    /// the application pool. On its own it does not shorten OS scheduling
    /// delays; on Linux its [`MstpThreadConfig`](super::MstpThreadConfig)
    /// can ask for `SCHED_FIFO` and CPU affinity, which `start` applies or
    /// fails on, and [`Self::execution_status`] reports. Async serial resources
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

    /// How the MAC loop runs, once [`start`](TransportPort::start) has
    /// succeeded: whether on a thread of its own, and the real-time
    /// priority and CPU affinity that thread reads back. `None` before
    /// then. The [diagnostics handle](Self::diagnostics) reports the same,
    /// after the transport has moved
    /// ([`MstpDiagnostics::execution_status`]).
    pub fn execution_status(&self) -> Option<MstpExecutionStatus> {
        self.diagnostics.execution_status()
    }

    /// This transport's frame tap: every frame this station sends and
    /// every frame it hears on the line, preamble to CRC, for whoever
    /// subscribes: data frames and the control frames the master node
    /// handles itself (the token, polls, postponed replies) alike. Like
    /// [`Self::diagnostics`], the tap outlives a move of the transport:
    /// clone it, or subscribe, before handing the transport over. See
    /// [`crate::tap`].
    pub fn frame_tap(&self) -> &crate::tap::FrameTap {
        &self.frame_tap
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

    /// This transport with its serial port boxed, the form
    /// [`AnyTransport`](crate::any::AnyTransport)'s MS/TP variant holds;
    /// `From<MstpTransport<S>> for AnyTransport` calls it. Its
    /// configuration, execution mode, diagnostics and frame tap carry over,
    /// and so does a running task: the port it owns stays unboxed.
    pub fn into_boxed(mut self) -> MstpTransport<BoxedSerial> {
        MstpTransport {
            serial: self.serial.take().map(BoxedSerial::new),
            config: self.config.clone(),
            local_mac: self.local_mac,
            link: self.link.take(),
            running: StdMutex::new(std::mem::take(self.running_mut())),
            stop_gate: std::mem::take(&mut self.stop_gate),
            execution_mode: std::mem::take(&mut self.execution_mode),
            diagnostics: std::mem::replace(&mut self.diagnostics, MstpDiagnostics::new()),
            frame_tap: std::mem::replace(&mut self.frame_tap, crate::tap::FrameTap::new()),
        }
        // `self` now holds nothing its `Drop` would release.
    }

    fn running_mut(&mut self) -> &mut Running {
        self.running.get_mut_locked()
    }

    /// End the node without waiting, through `&self` (#2617): what an
    /// abort and a drop do. The queues close, so every later send is
    /// refused at once; the node's task is aborted, and the serial port
    /// goes with it as the runtime drops it; a dedicated thread is told to
    /// end. The task and the thread stay here, so a stop after an abort
    /// still waits for both, and so for the serial port.
    fn abort_task(&self) {
        if let Some(link) = &self.link {
            link.queues.close();
        }
        let mut running = lock(&self.running);
        let Running { task, runtime } = &mut *running;
        if let Some(task) = task {
            if let Some(runtime) = runtime {
                runtime.hold_until_node_dropped();
            }
            task.abort();
        }
        if let Some(runtime) = runtime {
            // The thread ends once its runtime has dropped the aborted node.
            drop(runtime.stopping());
        }
    }

    /// Queue an NPDU for the core, refusing it at once if it cannot be sent.
    fn enqueue(&self, destination: u8, npdu: &[u8]) -> Result<(), Error> {
        let Some(link) = &self.link else {
            return Err(not_started());
        };
        let result = link
            .queues
            .enqueue(destination, npdu, link.clock.now(), link.queue_max_age);
        let counts = &self.diagnostics.counters;
        counts.publish_queues(&link.queues);
        match result {
            Ok(()) => Ok(()),
            // Stopped, aborted, or the node's task ended: checked before the
            // length, so a stopped port's refusal never draws a router's
            // too-long reject.
            Err(EnqueueError::Closed) => Err(no_longer_running()),
            // Longer than the port's frames carry (#1833): a router answers
            // it with reject reason 4, and a client may try a smaller one.
            Err(EnqueueError::Rejected(QueueRefusal::Oversize)) => {
                increment(&counts.outbound_oversize);
                Err(Error::SendTooLong {
                    length: npdu.len(),
                    limit: self.config.max_npdu_length(),
                })
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

/// What a send before start gets (#1833): the transport can start, so
/// [`Lifecycle::NotRunning`].
#[cold]
#[inline(never)]
fn not_started() -> Error {
    Error::state(Lifecycle::NotRunning, "MS/TP transport not started")
}

/// What a start gets once the serial port went with an earlier start's
/// task (#1833): [`Lifecycle::AlreadyStarted`] while that node runs, and
/// [`Lifecycle::Closed`] once it `ended`, since the transport doesn't start
/// again.
#[cold]
#[inline(never)]
fn started_refusal(ended: bool) -> Error {
    if ended {
        Error::state(
            Lifecycle::Closed,
            "MS/TP transport is no longer running: it does not start again",
        )
    } else {
        Error::state(Lifecycle::AlreadyStarted, "MS/TP transport already started")
    }
}

/// What a send gets once the node's queues closed (#1833): after a stop or
/// an abort, or once the node's task ended on a serial failure, which it
/// reported then. The transport doesn't start again, so
/// [`Lifecycle::Closed`]; nothing of the send was attempted.
#[cold]
#[inline(never)]
fn no_longer_running() -> Error {
    Error::state(Lifecycle::Closed, "MS/TP transport is no longer running")
}

impl<S: SerialPort> TransportPort for MstpTransport<S> {
    fn data_link(&self) -> Option<DataLink> {
        Some(DataLink::Mstp)
    }

    fn supports_local_nonrouter_number_controls(&self) -> bool {
        true
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        /// NPDU receive channel capacity — smaller than BIP/Ethernet for low-bandwidth serial.
        const NPDU_CHANNEL_CAPACITY: usize = 64;

        let config = self.config.core_config()?;
        self.execution_mode.validate()?;
        // The serial port went with the first start's task: running, that
        // is a second start; once the node ended (a stop, an abort, a
        // serial failure), the transport doesn't start again (#1833).
        let ended = self
            .link
            .as_ref()
            .is_some_and(|link| link.queues.is_closed());
        let not_started = || started_refusal(ended);
        self.serial
            .as_ref()
            .ok_or_else(not_started)?
            .echo()
            .validate()?;

        // The thread and its settings come first: if they fail, the serial
        // port stays here for another try.
        let (mut dedicated_runtime, status) = match &self.execution_mode {
            MstpExecutionMode::Tokio => (None, MstpExecutionStatus::default()),
            MstpExecutionMode::DedicatedThread(thread) => {
                let (runtime, status) = DedicatedRuntime::new(thread.clone()).await?;
                (Some(runtime), status)
            }
        };
        let serial = self.serial.take().ok_or_else(not_started)?;

        let (npdu_tx, npdu_rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);
        let queues = SharedQueues::new(self.config.extended_frames);
        let clock = tokio_clock();
        let task = Task {
            serial,
            config,
            queues: queues.clone(),
            clock,
            npdus: npdu_tx,
            counts: self.diagnostics.counters.clone(),
            transmit_pad: self.config.transmit_latency_pad,
            tap: self.frame_tap.clone(),
        };
        let task = match &mut dedicated_runtime {
            Some(runtime) => runtime.spawn(task.run()),
            None => tokio::spawn(task.run()),
        };
        self.diagnostics.counters.publish_execution(status);
        *self.running_mut() = Running {
            task: Some(task),
            runtime: dedicated_runtime,
        };
        self.link = Some(Link {
            queues,
            clock,
            queue_max_age: config.queue_max_age,
        });
        self.stop_gate.open();
        Ok(npdu_rx)
    }

    /// Stop through a shared reference (#2617). Later sends are refused at
    /// once, with the error for a transport no longer running; queued NPDUs
    /// go with the task, which is aborted and joined, and the serial port
    /// with it; then a dedicated thread ends. The stop hands no token on.
    /// The transport does not start again after a stop: its serial port
    /// went with the task.
    async fn stop(&self) -> Result<(), Error> {
        let Some(stopping) = self.stop_gate.enter().await else {
            return Ok(());
        };
        // The node's own close on exit misses a task aborted before it ran.
        if let Some(link) = &self.link {
            link.queues.close();
        }
        let joining = {
            let mut running = lock(&self.running);
            running.task.take().map(|task| {
                // The node ends on its own thread; test time waits for it.
                let held = running.runtime.as_ref().map(DedicatedRuntime::hold_time);
                if let Some(runtime) = &running.runtime {
                    runtime.hold_until_node_dropped();
                }
                task.abort();
                (held, task)
            })
        };
        if let Some((_held, task)) = joining {
            // A join cancelled here puts the task back for the next stop.
            let mut joining = Joining {
                running: &self.running,
                task: Some(task),
            };
            if let Some(task) = joining.task.as_mut() {
                let _ = task.await;
            }
            joining.task = None;
        }
        // The thread ends once the task is joined; it stays here until it
        // has, so a cancelled stop leaves the wait to the next.
        let ending = lock(&self.running)
            .runtime
            .as_mut()
            .map(|runtime| (runtime.hold_time(), runtime.stopping()));
        if let Some((_held, mut finished)) = ending {
            // Returns once the thread sends, or with an error once it has
            // gone: either way, it has ended.
            let _ = finished.changed().await;
        }
        lock(&self.running).runtime.take();
        stopping.finish();
        Ok(())
    }

    fn abort(&self) {
        self.abort_task();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let [destination] = mac else {
            return Err(Error::InvalidArgument(
                Refusal::new("MS/TP MAC must be 1 byte, got {}").with_value(mac.len() as u64),
            ));
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
    fn max_npdu(&self) -> bacnet_npdu::NpduLimits {
        let octets = u16::try_from(self.config.max_npdu_length()).unwrap_or(u16::MAX);
        bacnet_npdu::NpduLimits::both(octets)
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
        self.abort_task();
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
        if matches!(self.echo, Echo::Full { .. })
            && let Some(own) = self.own.upgrade()
        {
            own.send(data.to_vec())
                .await
                .map_err(|_| Error::state(Lifecycle::Closed, "loopback echo failed"))?;
        }
        self.tx
            .send(data.to_vec())
            .await
            .map_err(|_| Error::state(Lifecycle::Closed, "loopback write failed"))
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
                None => return Err(Error::state(Lifecycle::Closed, "loopback channel closed")),
            }
        }
    }

    fn echo(&self) -> Echo {
        self.echo
    }
}

#[cfg(test)]
#[path = "port_abort_tests.rs"]
mod abort_tests;

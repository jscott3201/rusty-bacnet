//! The one `TransportPort` double shared by the crate's unit tests (#904).
//!
//! `BACnetServer<T>` and `NetworkLayer<T>` are generic over their transport, so
//! every distinct test transport type compiles the whole server, and every task
//! it spawns, again. In-crate tests therefore share this single non-generic
//! [`TestTransport`] and customise it at runtime: plain builder fields, the
//! [`StartMode`] and [`SendMode`] enums, the built-in send controls on
//! [`TestTransportHandle`], and hooks stored as `Arc<dyn Fn ..>`. Builder
//! methods that take a closure erase it at once, so no closure type ever
//! reaches `TestTransport` or the server.
//!
//! Every send runs this pipeline:
//! 1. The [`SendMode`] for its kind (unicast or broadcast): `Ignore` returns
//!    `Ok(())` untouched, skipping every later step, and `Panic` panics.
//! 2. The frame is appended to the [`SendLog`].
//! 3. The [`TestTransportBuilder::on_send`] hook, if any, runs to completion;
//!    its error ends the send.
//! 4. Built-in controls: a pending `fail_next_send` is taken; a held send
//!    (`block_next_send`) signals `wait_blocked` and waits for one
//!    `release_sends` permit; then the send fails if `fail_next_send` was
//!    taken.

use std::any::Any;
use std::future::Future;
use std::net::SocketAddrV4;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::{decode_npdu, Npdu};
use bacnet_transport::port::{BipPort, ReceivedNpdu, TransportPort};
use bacnet_types::bip_port::BipPortMode;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::{mpsc, Notify, Semaphore};

/// The boxed future a send or stop hook returns.
type HookFuture = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

type SendHook = Arc<dyn Fn(SentFrame) -> HookFuture + Send + Sync>;
type StopHook = Arc<dyn Fn() -> HookFuture + Send + Sync>;
type Callback = Arc<dyn Fn() + Send + Sync>;
type MacPredicate = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;
type EndpointHook = Arc<dyn Fn() -> Option<SocketAddrV4> + Send + Sync>;

/// A B/IP-shaped local MAC (127.0.0.1:47808) for tests that want six octets.
pub(crate) const BIP_LOCAL_MAC: [u8; 6] = [127, 0, 0, 1, 0xBA, 0xC0];

/// One NPDU handed to the transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SentFrame {
    /// The NPDU bytes exactly as sent.
    pub(crate) npdu: Bytes,
    /// Destination MAC. Empty for a broadcast.
    pub(crate) mac: MacAddr,
    /// Whether this was `send_broadcast`.
    pub(crate) broadcast: bool,
}

impl SentFrame {
    /// Decode the NPDU, panicking on malformed bytes.
    pub(crate) fn decode_npdu(&self) -> Npdu {
        decode_npdu(self.npdu.clone()).expect("sent frame should decode as NPDU")
    }

    /// Decode the APDU the NPDU carries, panicking on malformed bytes.
    pub(crate) fn apdu(&self) -> Apdu {
        decode_apdu(self.decode_npdu().payload).expect("sent NPDU should carry an APDU")
    }
}

/// How `start` behaves.
pub(crate) enum StartMode {
    /// Return a receiver whose sender is already gone: the link is closed.
    Closed,
    /// Return this receiver, which the test feeds. `start` takes it, so a
    /// second start fails.
    Inbound(Option<mpsc::Receiver<ReceivedNpdu>>),
    /// Panic with this message, for tests proving startup is never reached.
    Panic(&'static str),
}

/// What one kind of send (unicast or broadcast) does.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SendMode {
    /// Run the full pipeline (record, report, hook, built-in controls).
    Record,
    /// Return `Ok(())` without recording or running anything.
    Ignore,
    /// Panic with this message.
    Panic(&'static str),
}

/// Shared, cloneable log of recorded sends.
#[derive(Clone, Default)]
pub(crate) struct SendLog {
    inner: Arc<LogInner>,
}

#[derive(Default)]
struct LogInner {
    frames: Mutex<Vec<SentFrame>>,
    pushed: Notify,
}

impl SendLog {
    /// Lock the underlying frames for in-place inspection or mutation. A
    /// poisoned lock is recovered so a panicking assertion in one test thread
    /// does not bury its failure under later pushes from the server.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Vec<SentFrame>> {
        self.inner
            .frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Append a frame. The transport does this itself; hooks may record extra.
    pub(crate) fn push(&self, frame: SentFrame) {
        self.lock().push(frame);
        self.inner.pushed.notify_waiters();
    }

    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Snapshot of every recorded frame.
    pub(crate) fn frames(&self) -> Vec<SentFrame> {
        self.lock().clone()
    }

    /// The frame at `index`, panicking when absent.
    pub(crate) fn frame(&self, index: usize) -> SentFrame {
        let frame = self.lock().get(index).cloned();
        frame.unwrap_or_else(|| {
            panic!(
                "no sent frame at index {index}; the log holds {}",
                self.len()
            )
        })
    }

    /// Remove and return every recorded frame.
    pub(crate) fn take(&self) -> Vec<SentFrame> {
        std::mem::take(&mut *self.lock())
    }

    pub(crate) fn clear(&self) {
        self.lock().clear();
    }

    /// Recorded unicast frames, in order.
    pub(crate) fn unicasts(&self) -> Vec<SentFrame> {
        self.lock()
            .iter()
            .filter(|f| !f.broadcast)
            .cloned()
            .collect()
    }

    /// Recorded broadcast frames, in order.
    pub(crate) fn broadcasts(&self) -> Vec<SentFrame> {
        self.lock()
            .iter()
            .filter(|f| f.broadcast)
            .cloned()
            .collect()
    }

    /// NPDU bytes of every recorded frame.
    pub(crate) fn npdus(&self) -> Vec<Bytes> {
        self.lock().iter().map(|f| f.npdu.clone()).collect()
    }

    /// Wait until at least `len` frames are recorded. Callers add a timeout.
    pub(crate) async fn wait_for_len(&self, len: usize) {
        loop {
            let pushed = self.inner.pushed.notified();
            if self.len() >= len {
                return;
            }
            pushed.await;
        }
    }
}

struct Shared {
    sent: SendLog,
    starts: AtomicUsize,
    fail_next: AtomicBool,
    block_next: AtomicBool,
    blocked: Semaphore,
    release: Semaphore,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            sent: SendLog::default(),
            starts: AtomicUsize::new(0),
            fail_next: AtomicBool::new(false),
            block_next: AtomicBool::new(false),
            blocked: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }
}

impl Shared {
    async fn controls(&self) -> Result<(), Error> {
        let fail_once = self.fail_next.swap(false, Ordering::SeqCst);
        if self.block_next.swap(false, Ordering::SeqCst) {
            self.blocked.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
        if fail_once {
            return Err(Error::Encoding(
                "injected test transport send failure".into(),
            ));
        }
        Ok(())
    }
}

/// Cloneable view of a transport's log, start counter and built-in send controls.
///
/// Take it from the builder or the transport before the transport moves into
/// the server; `server.test_network().transport().handle()` also works.
#[derive(Clone)]
pub(crate) struct TestTransportHandle {
    shared: Arc<Shared>,
}

impl TestTransportHandle {
    pub(crate) fn sent(&self) -> SendLog {
        self.shared.sent.clone()
    }

    /// Calls to `start`, counted before the start mode runs.
    pub(crate) fn starts(&self) -> usize {
        self.shared.starts.load(Ordering::SeqCst)
    }

    /// Fail the next recorded send only.
    pub(crate) fn fail_next_send(&self) {
        self.shared.fail_next.store(true, Ordering::SeqCst);
    }

    /// Hold the next recorded send only, until released.
    pub(crate) fn block_next_send(&self) {
        self.shared.block_next.store(true, Ordering::SeqCst);
    }

    /// Let `permits` held sends (current or future) continue.
    pub(crate) fn release_sends(&self, permits: usize) {
        self.shared.release.add_permits(permits);
    }

    /// Wait until one more send has become held. Each held send is counted
    /// once, so a send held before the wait still satisfies it.
    pub(crate) async fn wait_blocked(&self) {
        self.shared.blocked.acquire().await.unwrap().forget();
    }
}

#[derive(Default)]
struct Hooks {
    on_start: Option<Callback>,
    on_send: Option<SendHook>,
    on_stop: Option<StopHook>,
    on_drop: Option<Callback>,
    is_broadcast_mac: Option<MacPredicate>,
    is_group_destination: Option<MacPredicate>,
    bip_broadcast_endpoint: Option<EndpointHook>,
}

/// The shared, non-generic test transport. See the module docs.
pub(crate) struct TestTransport {
    local_mac: MacAddr,
    broadcast_macs: Vec<MacAddr>,
    /// Group destinations beside the broadcast MACs (#1493).
    group_macs: Vec<MacAddr>,
    start: StartMode,
    unicast: SendMode,
    broadcast: SendMode,
    hooks: Hooks,
    state: Option<Arc<dyn Any + Send + Sync>>,
    shared: Arc<Shared>,
    /// See [`TestTransportBuilder::number_controls`].
    number_controls: bool,
    /// See [`TestTransportBuilder::bip_port`].
    bip_port: Option<BipPort>,
    /// A registered Network Port's lease, held until the transport drops.
    port_lease: Option<Arc<()>>,
}

impl TestTransport {
    /// A closed link with local MAC `[1]` that records every send and succeeds.
    pub(crate) fn new() -> Self {
        Self::builder().build()
    }

    /// A transport whose `start` panics, for tests that fail before startup.
    pub(crate) fn never_start() -> Self {
        Self::builder()
            .start(StartMode::Panic("test transport must never start"))
            .build()
    }

    /// [`Self::new`] fed by an inbound channel of `capacity`.
    pub(crate) fn inbound(capacity: usize) -> (Self, mpsc::Sender<ReceivedNpdu>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self::builder().inbound(rx).build(), tx)
    }

    pub(crate) fn builder() -> TestTransportBuilder {
        TestTransportBuilder {
            transport: Self {
                local_mac: MacAddr::from_slice(&[1]),
                broadcast_macs: Vec::new(),
                group_macs: Vec::new(),
                start: StartMode::Closed,
                unicast: SendMode::Record,
                broadcast: SendMode::Record,
                hooks: Hooks::default(),
                state: None,
                shared: Arc::default(),
                number_controls: false,
                bip_port: None,
                port_lease: None,
            },
        }
    }

    pub(crate) fn handle(&self) -> TestTransportHandle {
        TestTransportHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    pub(crate) fn sent(&self) -> SendLog {
        self.shared.sent.clone()
    }

    /// The value attached with [`TestTransportBuilder::state`], for tests that
    /// only hold the server. Panics when absent or of another type.
    pub(crate) fn state<T: Any + Send + Sync>(&self) -> &T {
        self.state
            .as_deref()
            .and_then(|state| state.downcast_ref::<T>())
            .expect("test transport state is missing or has another type")
    }

    async fn send_frame(&self, mode: SendMode, frame: SentFrame) -> Result<(), Error> {
        match mode {
            SendMode::Record => {}
            SendMode::Ignore => return Ok(()),
            SendMode::Panic(message) => panic!("{message}"),
        }
        self.shared.sent.push(frame.clone());
        if let Some(hook) = &self.hooks.on_send {
            hook(frame).await?;
        }
        self.shared.controls().await
    }

    fn frame(npdu: &[u8], mac: &[u8], broadcast: bool) -> SentFrame {
        SentFrame {
            npdu: Bytes::copy_from_slice(npdu),
            mac: MacAddr::from_slice(mac),
            broadcast,
        }
    }
}

impl Drop for TestTransport {
    fn drop(&mut self) {
        if let Some(hook) = &self.hooks.on_drop {
            hook();
        }
    }
}

impl TransportPort for TestTransport {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        self.number_controls
    }

    fn bip_port(&self) -> Option<BipPort> {
        self.bip_port.clone()
    }

    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        match self.bip_port {
            Some(_) => {
                self.port_lease = Some(lease);
                Ok(())
            }
            None => Err(Error::Encoding("not a B/IP test link".into())),
        }
    }

    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        self.hooks
            .bip_broadcast_endpoint
            .as_ref()
            .and_then(|hook| hook())
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.shared.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = &self.hooks.on_start {
            hook();
        }
        match &mut self.start {
            StartMode::Closed => Ok(mpsc::channel(1).1),
            StartMode::Inbound(receiver) => receiver
                .take()
                .ok_or_else(|| Error::Encoding("test transport already started".into())),
            StartMode::Panic(message) => panic!("{message}"),
        }
    }

    async fn stop(&mut self) -> Result<(), Error> {
        match &self.hooks.on_stop {
            Some(hook) => hook().await,
            None => Ok(()),
        }
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.send_frame(self.unicast, Self::frame(npdu, mac, false))
            .await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_frame(self.broadcast, Self::frame(npdu, &[], true))
            .await
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        match &self.hooks.is_broadcast_mac {
            Some(hook) => hook(mac),
            None => self.broadcast_macs.iter().any(|b| b.as_slice() == mac),
        }
    }

    /// `is_broadcast_mac`, one of the [`TestTransportBuilder::group_mac`]s,
    /// or what the [`TestTransportBuilder::on_is_group_destination`] hook
    /// adds.
    fn is_group_destination(&self, mac: &[u8]) -> bool {
        self.is_broadcast_mac(mac)
            || self.group_macs.iter().any(|g| g.as_slice() == mac)
            || self
                .hooks
                .is_group_destination
                .as_ref()
                .is_some_and(|hook| hook(mac))
    }

    /// The broadcast and group MAC lists, copied, so the rule never calls a
    /// hook.
    fn group_destinations(&self) -> bacnet_transport::port::GroupDestinations {
        let macs: Vec<MacAddr> = self
            .broadcast_macs
            .iter()
            .chain(&self.group_macs)
            .cloned()
            .collect();
        bacnet_transport::port::GroupDestinations::new(move |mac| {
            macs.iter().any(|g| g.as_slice() == mac)
        })
    }
}

/// Configures a [`TestTransport`]. Defaults: local MAC `[1]`, receive capacity
/// 1476, [`StartMode::Closed`], [`SendMode::Record`] for both kinds, no
/// broadcast MACs, no B/IP endpoint, no Number controls, no hooks.
pub(crate) struct TestTransportBuilder {
    transport: TestTransport,
}

impl TestTransportBuilder {
    pub(crate) fn local_mac(mut self, mac: &[u8]) -> Self {
        self.transport.local_mac = MacAddr::from_slice(mac);
        self
    }

    /// Add a MAC that `is_broadcast_mac` recognises (unless a hook overrides).
    pub(crate) fn broadcast_mac(mut self, mac: &[u8]) -> Self {
        self.transport.broadcast_macs.push(MacAddr::from_slice(mac));
        self
    }

    /// Add a MAC that reaches a group of nodes without being the link's
    /// broadcast, such as a B/IP multicast address: `is_group_destination`
    /// and `group_destinations` recognise it, `is_broadcast_mac` doesn't.
    pub(crate) fn group_mac(mut self, mac: &[u8]) -> Self {
        self.transport.group_macs.push(MacAddr::from_slice(mac));
        self
    }

    /// Recognise more group destinations with a hook, beside the broadcast
    /// and group MACs: `is_group_destination` asks it, `group_destinations`
    /// doesn't.
    pub(crate) fn on_is_group_destination(
        mut self,
        hook: impl Fn(&[u8]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.is_group_destination = Some(Arc::new(hook));
        self
    }

    /// Decide `is_broadcast_mac` with a hook instead of the MAC list.
    pub(crate) fn on_is_broadcast_mac(
        mut self,
        hook: impl Fn(&[u8]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.is_broadcast_mac = Some(Arc::new(hook));
        self
    }

    /// Answer `bip_broadcast_endpoint` with a hook (default: `None`).
    pub(crate) fn on_bip_broadcast_endpoint(
        mut self,
        hook: impl Fn() -> Option<SocketAddrV4> + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.bip_broadcast_endpoint = Some(Arc::new(hook));
        self
    }

    pub(crate) fn start(mut self, mode: StartMode) -> Self {
        self.transport.start = mode;
        self
    }

    /// Opt in to the local Network Number controls, so a server spawns its
    /// Number worker on this link.
    pub(crate) fn number_controls(mut self) -> Self {
        self.transport.number_controls = true;
        self
    }

    /// Report `endpoint` as a NORMAL B/IP bind, so a server can register a
    /// Network Port on this link; the transport then holds the port's lease.
    pub(crate) fn normal_bip(self, endpoint: SocketAddrV4) -> Self {
        self.bip_port(BipPort {
            endpoint,
            mode: BipPortMode::Normal,
        })
    }

    /// Report `port`, a B/IP bind in any mode, for a server to register a
    /// Network Port on; the transport then holds the port's lease. A BBMD
    /// mode's tables are whatever view the test lends.
    pub(crate) fn bip_port(mut self, port: BipPort) -> Self {
        self.transport.bip_port = Some(port);
        self
    }

    /// Shorthand for `start(StartMode::Inbound(Some(receiver)))`.
    pub(crate) fn inbound(self, receiver: mpsc::Receiver<ReceivedNpdu>) -> Self {
        self.start(StartMode::Inbound(Some(receiver)))
    }

    pub(crate) fn unicast(mut self, mode: SendMode) -> Self {
        self.transport.unicast = mode;
        self
    }

    pub(crate) fn broadcast(mut self, mode: SendMode) -> Self {
        self.transport.broadcast = mode;
        self
    }

    /// Run a callback at the top of every `start`, before the start mode.
    pub(crate) fn on_start(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.transport.hooks.on_start = Some(Arc::new(hook));
        self
    }

    /// Run a hook on every recorded send, after it is logged and before the
    /// built-in controls. The hook's future is part of the send future, so
    /// dropping the send drops it; its error becomes the send's result.
    pub(crate) fn on_send<F>(
        mut self,
        hook: impl Fn(SentFrame) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: SendHook =
            Arc::new(move |frame: SentFrame| -> HookFuture { Box::pin(hook(frame)) });
        self.transport.hooks.on_send = Some(hook);
        self
    }

    /// Replace `stop`'s `Ok(())` with this hook's result.
    pub(crate) fn on_stop<F>(mut self, hook: impl Fn() -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: StopHook = Arc::new(move || -> HookFuture { Box::pin(hook()) });
        self.transport.hooks.on_stop = Some(hook);
        self
    }

    /// Run a callback when the transport is dropped.
    pub(crate) fn on_drop(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.transport.hooks.on_drop = Some(Arc::new(hook));
        self
    }

    /// Attach test-owned state, read back with [`TestTransport::state`].
    pub(crate) fn state<T: Any + Send + Sync>(mut self, state: Arc<T>) -> Self {
        self.transport.state = Some(state);
        self
    }

    pub(crate) fn sent(&self) -> SendLog {
        self.transport.sent()
    }

    pub(crate) fn build(self) -> TestTransport {
        self.transport
    }
}

#[path = "test_transport_tests.rs"]
mod tests;

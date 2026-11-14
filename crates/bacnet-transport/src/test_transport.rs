//! The one `TransportPort` double the workspace's tests share (#904, #2557).
//!
//! Test support, behind the `test-support` feature and hidden from the
//! documentation: it is not a stable surface, and nothing outside a
//! `dev-dependencies` table may enable the feature.
//!
//! `BACnetServer<T>`, `NetworkLayer<T>` and `BACnetRouter` are generic over
//! their transport, so every distinct test transport type compiles the whole
//! stack, and every task it spawns, again. Tests therefore share this single
//! non-generic [`TestTransport`] and customise it at runtime: plain builder
//! fields, the [`StartMode`] and [`SendMode`] enums, the built-in send
//! controls on [`TestTransportHandle`], and hooks stored as `Arc<dyn Fn ..>`.
//! Builder methods that take a closure erase it at once, so no closure type
//! ever reaches `TestTransport` or the stack above it.
//!
//! A transport can stand in front of a real one: [`TestTransportBuilder::inner`]
//! names an [`AnyTransport`] that supplies the link's identity (MAC, limits,
//! group rules) and its start, stop and abort, and that unicasts go on to
//! under [`SendMode::Forward`]. A test then watches or changes what a real
//! B/IP or MS/TP port does, with no second transport type.
//!
//! Every send runs this pipeline:
//! 1. The [`SendMode`] for its kind (unicast or broadcast): `Ignore` returns
//!    `Ok(())` untouched, skipping every later step, and `Panic` panics.
//! 2. The frame, with any data attributes, is appended to the [`SendLog`].
//! 3. The [`TestTransportBuilder::on_send`] hook, if any, runs to completion;
//!    its error ends the send.
//! 4. Built-in controls: a pending `fail_next_send` is taken; a held send
//!    (`block_next_send`) signals `wait_blocked` and waits for one
//!    `release_sends` permit; then the send fails if `fail_next_send` was
//!    taken.
//! 5. `Forward` only: the send goes on to the inner transport and returns its
//!    result.

use std::any::Any;
use std::future::Future;
use std::net::SocketAddrV4;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu};
use bacnet_npdu::NpduLimits;
use bacnet_types::MacAddr;
use bacnet_types::bip_port::BipPortMode;
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::{Notify, Semaphore, mpsc, watch};

use crate::any::AnyTransport;
use crate::port::{
    Bip6Port, BipPort, DataAttribute, GroupDestinations, ReceivedNpdu, TransportPort,
};

/// The boxed future a send or stop hook returns.
type HookFuture = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

type SendHook = Arc<dyn Fn(SentFrame) -> HookFuture + Send + Sync>;
type StopHook = Arc<dyn Fn() -> HookFuture + Send + Sync>;
type Callback = Arc<dyn Fn() + Send + Sync>;
type StartedHook = Arc<dyn Fn(Started<'_>) + Send + Sync>;
type MacPredicate = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;
type EndpointHook = Arc<dyn Fn() -> Option<SocketAddrV4> + Send + Sync>;
type LimitsHook = Arc<dyn Fn() -> NpduLimits + Send + Sync>;

/// A B/IP-shaped local MAC (127.0.0.1:47808) for tests that want six octets.
pub const BIP_LOCAL_MAC: [u8; 6] = [127, 0, 0, 1, 0xBA, 0xC0];

/// One NPDU handed to the transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentFrame {
    /// The NPDU bytes exactly as sent.
    pub npdu: Bytes,
    /// Destination MAC. Empty for a broadcast.
    pub mac: MacAddr,
    /// Whether this was `send_broadcast`.
    pub broadcast: bool,
    /// The data attributes the send carried: empty for a send that named none.
    pub attributes: Vec<DataAttribute>,
}

impl SentFrame {
    /// Decode the NPDU, panicking on malformed bytes.
    pub fn decode_npdu(&self) -> Npdu {
        decode_npdu(self.npdu.clone()).expect("sent frame should decode as NPDU")
    }

    /// Decode the APDU the NPDU carries, panicking on malformed bytes.
    pub fn apdu(&self) -> Apdu {
        decode_apdu(self.decode_npdu().payload).expect("sent NPDU should carry an APDU")
    }
}

/// How `start` behaves.
pub enum StartMode {
    /// Return a receiver whose sender is already gone: the link is closed.
    /// With an [`inner`](TestTransportBuilder::inner) transport, start it and
    /// return its receiver instead.
    Closed,
    /// Return this receiver, which the test feeds. `start` takes it, so a
    /// second start fails.
    Inbound(Option<mpsc::Receiver<ReceivedNpdu>>),
    /// Panic with this message, for tests proving startup is never reached.
    Panic(&'static str),
}

/// What one kind of send (unicast or broadcast) does.
#[derive(Clone, Copy, Debug)]
pub enum SendMode {
    /// Run the full pipeline (record, hook, built-in controls).
    Record,
    /// Run the full pipeline, then hand the send to the inner transport and
    /// return its result. Without an inner transport the send fails.
    Forward,
    /// Return `Ok(())` without recording or running anything.
    Ignore,
    /// Panic with this message.
    Panic(&'static str),
}

/// What a transport reports once its start has succeeded, for
/// [`TestTransportBuilder::on_started`].
#[derive(Clone, Copy, Debug)]
pub struct Started<'a> {
    /// The local MAC, as the transport reports it after the start.
    pub local_mac: &'a [u8],
    /// The local receive capacity, as reported after the start.
    pub receive_capacity: u16,
    /// The egress limit, as reported after the start.
    pub egress_limit: u16,
}

/// Shared, cloneable log of recorded sends.
#[derive(Clone, Default)]
pub struct SendLog {
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
    /// does not bury its failure under later pushes from the stack.
    pub fn lock(&self) -> MutexGuard<'_, Vec<SentFrame>> {
        self.inner
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Append a frame. The transport does this itself; hooks may record extra.
    pub fn push(&self, frame: SentFrame) {
        self.lock().push(frame);
        self.inner.pushed.notify_waiters();
    }

    /// How many frames are recorded.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether no frame is recorded.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Snapshot of every recorded frame.
    pub fn frames(&self) -> Vec<SentFrame> {
        self.lock().clone()
    }

    /// The frame at `index`, panicking when absent.
    pub fn frame(&self, index: usize) -> SentFrame {
        let frame = self.lock().get(index).cloned();
        frame.unwrap_or_else(|| {
            panic!(
                "no sent frame at index {index}; the log holds {}",
                self.len()
            )
        })
    }

    /// Remove and return every recorded frame.
    pub fn take(&self) -> Vec<SentFrame> {
        std::mem::take(&mut *self.lock())
    }

    /// Forget every recorded frame.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Recorded unicast frames, in order.
    pub fn unicasts(&self) -> Vec<SentFrame> {
        self.lock()
            .iter()
            .filter(|f| !f.broadcast)
            .cloned()
            .collect()
    }

    /// Recorded broadcast frames, in order.
    pub fn broadcasts(&self) -> Vec<SentFrame> {
        self.lock()
            .iter()
            .filter(|f| f.broadcast)
            .cloned()
            .collect()
    }

    /// NPDU bytes of every recorded frame.
    pub fn npdus(&self) -> Vec<Bytes> {
        self.lock().iter().map(|f| f.npdu.clone()).collect()
    }

    /// Wait until at least `len` frames are recorded. Callers add a timeout.
    pub async fn wait_for_len(&self, len: usize) {
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
    stops: AtomicUsize,
    aborts: AtomicUsize,
    prepares: AtomicUsize,
    /// What `local_receive_apdu_capacity` reports, or 0 for the inner
    /// transport's (1476 without one); see
    /// [`TestTransportHandle::set_receive_capacity`].
    receive_capacity: AtomicU16,
    /// What `egress_apdu_limit` reports, or 0 for the inner transport's
    /// (1476 without one); see [`TestTransportHandle::set_egress_limit`].
    egress_limit: AtomicU16,
    /// What `max_npdu` reports, or `None` for the inner transport's
    /// (standard without one).
    max_npdu: Mutex<Option<NpduLimits>>,
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
            stops: AtomicUsize::new(0),
            aborts: AtomicUsize::new(0),
            prepares: AtomicUsize::new(0),
            receive_capacity: AtomicU16::new(0),
            egress_limit: AtomicU16::new(0),
            max_npdu: Mutex::new(None),
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

/// Cloneable view of a transport's log, counters and built-in send controls.
///
/// Take it from the builder or the transport before the transport moves into
/// the stack under test.
#[derive(Clone)]
pub struct TestTransportHandle {
    shared: Arc<Shared>,
}

impl TestTransportHandle {
    /// The send log.
    pub fn sent(&self) -> SendLog {
        self.shared.sent.clone()
    }

    /// Calls to `start`, counted before the start mode runs.
    pub fn starts(&self) -> usize {
        self.shared.starts.load(Ordering::SeqCst)
    }

    /// Calls to `stop`, counted before the stop hook runs.
    pub fn stops(&self) -> usize {
        self.shared.stops.load(Ordering::SeqCst)
    }

    /// Calls to `abort`.
    pub fn aborts(&self) -> usize {
        self.shared.aborts.load(Ordering::SeqCst)
    }

    /// Calls to `prepare_for_routing`, whether or not they reach an inner
    /// transport.
    pub fn prepares(&self) -> usize {
        self.shared.prepares.load(Ordering::SeqCst)
    }

    /// Change what the transport reports as its receive capacity (1476 at
    /// first), as a real link might after start (#2093).
    pub fn set_receive_capacity(&self, capacity: u16) {
        self.shared
            .receive_capacity
            .store(capacity, Ordering::SeqCst);
    }

    /// Change what the transport reports as its egress limit (1476 at first).
    pub fn set_egress_limit(&self, limit: u16) {
        self.shared.egress_limit.store(limit, Ordering::SeqCst);
    }

    /// Change what the transport reports as its NPDU limits, unless an
    /// [`on_max_npdu`](TestTransportBuilder::on_max_npdu) hook answers.
    pub fn set_max_npdu(&self, limits: NpduLimits) {
        *self
            .shared
            .max_npdu
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(limits);
    }

    /// Fail the next recorded send only.
    pub fn fail_next_send(&self) {
        self.shared.fail_next.store(true, Ordering::SeqCst);
    }

    /// Hold the next recorded send only, until released.
    pub fn block_next_send(&self) {
        self.shared.block_next.store(true, Ordering::SeqCst);
    }

    /// Let `permits` held sends (current or future) continue.
    pub fn release_sends(&self, permits: usize) {
        self.shared.release.add_permits(permits);
    }

    /// Wait until one more send has become held. Each held send is counted
    /// once, so a send held before the wait still satisfies it.
    pub async fn wait_blocked(&self) {
        self.shared.blocked.acquire().await.unwrap().forget();
    }
}

#[derive(Default)]
struct Hooks {
    on_start: Option<Callback>,
    /// Awaited after `on_start`, before `start` answers: a start the test
    /// holds, or fails.
    start_gate: Option<StopHook>,
    on_started: Option<StartedHook>,
    on_send: Option<SendHook>,
    on_stop: Option<StopHook>,
    on_abort: Option<Callback>,
    on_drop: Option<Callback>,
    is_broadcast_mac: Option<MacPredicate>,
    is_group_destination: Option<MacPredicate>,
    bip_broadcast_endpoint: Option<EndpointHook>,
    max_npdu: Option<LimitsHook>,
}

/// The shared, non-generic test transport. See the module docs.
pub struct TestTransport {
    /// `None` takes the inner transport's MAC, or `[1]` without one.
    local_mac: Option<MacAddr>,
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
    /// See [`TestTransportBuilder::bip6_port`].
    bip6_port: Option<Bip6Port>,
    /// See [`TestTransportBuilder::max_npdu_changes`].
    max_npdu_changes: Option<watch::Receiver<NpduLimits>>,
    /// See [`TestTransportBuilder::inner`].
    inner: Option<AnyTransport>,
    /// See [`TestTransportBuilder::skip_prepare_for_routing`].
    prepare_inner: bool,
    /// A registered Network Port's lease, held until the transport drops.
    port_lease: Option<Arc<()>>,
}

impl TestTransport {
    /// A closed link with local MAC `[1]` that records every send and succeeds.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// A transport whose `start` panics, for tests that fail before startup.
    pub fn never_start() -> Self {
        Self::builder()
            .start(StartMode::Panic("test transport must never start"))
            .build()
    }

    /// [`Self::new`] fed by an inbound channel of `capacity`.
    pub fn inbound(capacity: usize) -> (Self, mpsc::Sender<ReceivedNpdu>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self::builder().inbound(rx).build(), tx)
    }

    /// A builder with the defaults named on [`TestTransportBuilder`].
    pub fn builder() -> TestTransportBuilder {
        TestTransportBuilder {
            transport: Self {
                local_mac: None,
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
                bip6_port: None,
                max_npdu_changes: None,
                inner: None,
                prepare_inner: true,
                port_lease: None,
            },
        }
    }

    /// The cloneable view of this transport's log, counters and controls.
    pub fn handle(&self) -> TestTransportHandle {
        TestTransportHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    /// The send log.
    pub fn sent(&self) -> SendLog {
        self.shared.sent.clone()
    }

    /// The value attached with [`TestTransportBuilder::state`], for tests that
    /// only hold the stack. Panics when absent or of another type.
    pub fn state<T: Any + Send + Sync>(&self) -> &T {
        self.state
            .as_deref()
            .and_then(|state| state.downcast_ref::<T>())
            .expect("test transport state is missing or has another type")
    }

    /// Whether the link's identity and group rules come from the inner
    /// transport: it exists and no list or hook here names a rule.
    fn rules_from_inner(&self) -> Option<&AnyTransport> {
        let own = !self.broadcast_macs.is_empty()
            || !self.group_macs.is_empty()
            || self.hooks.is_broadcast_mac.is_some()
            || self.hooks.is_group_destination.is_some();
        self.inner.as_ref().filter(|_| !own)
    }

    async fn send_frame(&self, mode: SendMode, frame: SentFrame) -> Result<(), Error> {
        let forward = match mode {
            SendMode::Record => false,
            SendMode::Forward => true,
            SendMode::Ignore => return Ok(()),
            SendMode::Panic(message) => panic!("{message}"),
        };
        self.shared.sent.push(frame.clone());
        if let Some(hook) = &self.hooks.on_send {
            hook(frame.clone()).await?;
        }
        self.shared.controls().await?;
        if !forward {
            return Ok(());
        }
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| Error::Encoding("test transport has no inner transport".into()))?;
        if frame.broadcast {
            inner
                .send_broadcast_with_data_attributes(&frame.npdu, &frame.attributes)
                .await
        } else {
            inner
                .send_unicast_with_data_attributes(&frame.npdu, &frame.mac, &frame.attributes)
                .await
        }
    }

    fn frame(npdu: &[u8], mac: &[u8], broadcast: bool, attributes: &[DataAttribute]) -> SentFrame {
        SentFrame {
            npdu: Bytes::copy_from_slice(npdu),
            mac: MacAddr::from_slice(mac),
            broadcast,
            attributes: attributes.to_vec(),
        }
    }
}

impl Default for TestTransport {
    fn default() -> Self {
        Self::new()
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

    fn bip6_port(&self) -> Option<Bip6Port> {
        self.bip6_port.clone()
    }

    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        match (&self.bip_port, &self.bip6_port) {
            (Some(_), _) | (_, Some(_)) => {
                self.port_lease = Some(lease);
                Ok(())
            }
            (None, None) => Err(Error::Encoding("not a B/IP test link".into())),
        }
    }

    fn bip_broadcast_endpoint(&self) -> Option<SocketAddrV4> {
        match &self.hooks.bip_broadcast_endpoint {
            Some(hook) => hook(),
            None => self.inner.as_ref().and_then(|i| i.bip_broadcast_endpoint()),
        }
    }

    fn prepare_for_routing(&mut self) {
        self.shared.prepares.fetch_add(1, Ordering::SeqCst);
        if self.prepare_inner
            && let Some(inner) = &mut self.inner
        {
            inner.prepare_for_routing();
        }
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.shared.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = &self.hooks.on_start {
            hook();
        }
        if let Some(gate) = &self.hooks.start_gate {
            gate().await?;
        }
        let received = match &mut self.start {
            StartMode::Closed => match &mut self.inner {
                Some(inner) => inner.start().await?,
                None => mpsc::channel(1).1,
            },
            StartMode::Inbound(receiver) => receiver
                .take()
                .ok_or_else(|| Error::Encoding("test transport already started".into()))?,
            StartMode::Panic(message) => panic!("{message}"),
        };
        if let Some(hook) = self.hooks.on_started.clone() {
            hook(Started {
                local_mac: self.local_mac(),
                receive_capacity: self.local_receive_apdu_capacity(),
                egress_limit: self.egress_apdu_limit(),
            });
        }
        Ok(received)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.shared.stops.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = &self.hooks.on_stop {
            return hook().await;
        }
        match &mut self.inner {
            Some(inner) => inner.stop().await,
            None => Ok(()),
        }
    }

    fn abort(&mut self) {
        self.shared.aborts.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = &self.hooks.on_abort {
            hook();
        }
        if let Some(inner) = &mut self.inner {
            inner.abort();
        }
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.send_frame(self.unicast, Self::frame(npdu, mac, false, &[]))
            .await
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.send_frame(self.unicast, Self::frame(npdu, mac, false, data_attributes))
            .await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_frame(self.broadcast, Self::frame(npdu, &[], true, &[]))
            .await
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.send_frame(
            self.broadcast,
            Self::frame(npdu, &[], true, data_attributes),
        )
        .await
    }

    fn local_mac(&self) -> &[u8] {
        match (&self.local_mac, &self.inner) {
            (Some(mac), _) => mac,
            (None, Some(inner)) => inner.local_mac(),
            (None, None) => &[1],
        }
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        match (
            self.shared.receive_capacity.load(Ordering::SeqCst),
            &self.inner,
        ) {
            (0, Some(inner)) => inner.local_receive_apdu_capacity(),
            (0, None) => 1476,
            (set, _) => set,
        }
    }

    fn egress_apdu_limit(&self) -> u16 {
        match (self.shared.egress_limit.load(Ordering::SeqCst), &self.inner) {
            (0, Some(inner)) => inner.egress_apdu_limit(),
            (0, None) => 1476,
            (set, _) => set,
        }
    }

    fn max_npdu(&self) -> NpduLimits {
        if let Some(hook) = &self.hooks.max_npdu {
            return hook();
        }
        let own = *self
            .shared
            .max_npdu
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match (own, &self.inner) {
            (Some(own), _) => own,
            (None, Some(inner)) => inner.max_npdu(),
            (None, None) => NpduLimits::default(),
        }
    }

    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>> {
        self.max_npdu_changes.clone()
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        if let Some(inner) = self.rules_from_inner() {
            return inner.is_broadcast_mac(mac);
        }
        match &self.hooks.is_broadcast_mac {
            Some(hook) => hook(mac),
            None => self.broadcast_macs.iter().any(|b| b.as_slice() == mac),
        }
    }

    /// `is_broadcast_mac`, one of the [`TestTransportBuilder::group_mac`]s,
    /// or what the [`TestTransportBuilder::on_is_group_destination`] hook
    /// adds.
    fn is_group_destination(&self, mac: &[u8]) -> bool {
        if let Some(inner) = self.rules_from_inner() {
            return inner.is_group_destination(mac);
        }
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
    fn group_destinations(&self) -> GroupDestinations {
        if let Some(inner) = self.rules_from_inner() {
            return inner.group_destinations();
        }
        let macs: Vec<MacAddr> = self
            .broadcast_macs
            .iter()
            .chain(&self.group_macs)
            .cloned()
            .collect();
        GroupDestinations::new(move |mac| macs.iter().any(|g| g.as_slice() == mac))
    }

    fn local_broadcast_destinations(&self) -> GroupDestinations {
        match self.rules_from_inner() {
            Some(inner) => inner.local_broadcast_destinations(),
            None => self.group_destinations(),
        }
    }
}

/// Configures a [`TestTransport`]. Defaults: local MAC `[1]`, receive
/// capacity and egress limit 1476, standard NPDU limits, [`StartMode::Closed`],
/// [`SendMode::Record`] for both kinds, no broadcast MACs, no B/IP endpoint,
/// no Number controls, no inner transport, no hooks.
pub struct TestTransportBuilder {
    transport: TestTransport,
}

impl TestTransportBuilder {
    /// Report `mac` as the local MAC, over an inner transport's.
    pub fn local_mac(mut self, mac: &[u8]) -> Self {
        self.transport.local_mac = Some(MacAddr::from_slice(mac));
        self
    }

    /// Add a MAC that `is_broadcast_mac` recognises (unless a hook overrides).
    pub fn broadcast_mac(mut self, mac: &[u8]) -> Self {
        self.transport.broadcast_macs.push(MacAddr::from_slice(mac));
        self
    }

    /// Add a MAC that reaches a group of nodes without being the link's
    /// broadcast, such as a B/IP multicast address: `is_group_destination`
    /// and `group_destinations` recognise it, `is_broadcast_mac` doesn't.
    pub fn group_mac(mut self, mac: &[u8]) -> Self {
        self.transport.group_macs.push(MacAddr::from_slice(mac));
        self
    }

    /// Recognise more group destinations with a hook, beside the broadcast
    /// and group MACs: `is_group_destination` asks it, `group_destinations`
    /// doesn't.
    pub fn on_is_group_destination(
        mut self,
        hook: impl Fn(&[u8]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.is_group_destination = Some(Arc::new(hook));
        self
    }

    /// Decide `is_broadcast_mac` with a hook instead of the MAC list.
    pub fn on_is_broadcast_mac(
        mut self,
        hook: impl Fn(&[u8]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.is_broadcast_mac = Some(Arc::new(hook));
        self
    }

    /// Answer `bip_broadcast_endpoint` with a hook (default: the inner
    /// transport's, else `None`).
    pub fn on_bip_broadcast_endpoint(
        mut self,
        hook: impl Fn() -> Option<SocketAddrV4> + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.bip_broadcast_endpoint = Some(Arc::new(hook));
        self
    }

    /// Choose how `start` behaves.
    pub fn start(mut self, mode: StartMode) -> Self {
        self.transport.start = mode;
        self
    }

    /// Opt in to the local Network Number controls, so a server spawns its
    /// Number worker on this link.
    pub fn number_controls(mut self) -> Self {
        self.transport.number_controls = true;
        self
    }

    /// Report `endpoint` as a NORMAL B/IP bind, so a server can register a
    /// Network Port on this link; the transport then holds the port's lease.
    pub fn normal_bip(self, endpoint: SocketAddrV4) -> Self {
        self.bip_port(BipPort::new(endpoint, BipPortMode::Normal))
    }

    /// Report `port`, a B/IP bind in any mode, for a server to register a
    /// Network Port on; the transport then holds the port's lease. A BBMD
    /// mode's tables are whatever view the test lends.
    pub fn bip_port(mut self, port: BipPort) -> Self {
        self.transport.bip_port = Some(port);
        self
    }

    /// Report `port`, a B/IPv6 bind in any mode, as
    /// [`bip_port`](Self::bip_port) does a B/IP one.
    pub fn bip6_port(mut self, port: Bip6Port) -> Self {
        self.transport.bip6_port = Some(port);
        self
    }

    /// Shorthand for `start(StartMode::Inbound(Some(receiver)))`.
    pub fn inbound(self, receiver: mpsc::Receiver<ReceivedNpdu>) -> Self {
        self.start(StartMode::Inbound(Some(receiver)))
    }

    /// Choose what a unicast send does.
    pub fn unicast(mut self, mode: SendMode) -> Self {
        self.transport.unicast = mode;
        self
    }

    /// Choose what a broadcast send does.
    pub fn broadcast(mut self, mode: SendMode) -> Self {
        self.transport.broadcast = mode;
        self
    }

    /// Report `capacity` as the local receive capacity (1476 at first; an
    /// inner transport's, when one is set and this is not).
    pub fn receive_capacity(self, capacity: u16) -> Self {
        self.transport
            .shared
            .receive_capacity
            .store(capacity, Ordering::SeqCst);
        self
    }

    /// Report `limit` as the egress APDU limit (1476 at first).
    pub fn egress_limit(self, limit: u16) -> Self {
        self.transport
            .shared
            .egress_limit
            .store(limit, Ordering::SeqCst);
        self
    }

    /// Report `limits` as the link's NPDU limits (standard at first; an
    /// inner transport's, when one is set and this is not).
    pub fn max_npdu(self, limits: NpduLimits) -> Self {
        TestTransportHandle {
            shared: Arc::clone(&self.transport.shared),
        }
        .set_max_npdu(limits);
        self
    }

    /// Answer `max_npdu` with a hook, so the limit follows state the test
    /// owns.
    pub fn on_max_npdu(mut self, hook: impl Fn() -> NpduLimits + Send + Sync + 'static) -> Self {
        self.transport.hooks.max_npdu = Some(Arc::new(hook));
        self
    }

    /// Publish the changes to the link's NPDU limits on `changes`, as
    /// `max_npdu_changes` does (default: none).
    pub fn max_npdu_changes(mut self, changes: watch::Receiver<NpduLimits>) -> Self {
        self.transport.max_npdu_changes = Some(changes);
        self
    }

    /// Stand in front of `inner`: it supplies the MAC, limits and group rules
    /// no setting here names, runs `start`, `stop` and `abort` (unless a
    /// [`StartMode::Inbound`] or an `on_stop` hook replaces them), receives
    /// `prepare_for_routing`, and takes the sends whose mode is
    /// [`SendMode::Forward`].
    pub fn inner(mut self, inner: impl Into<AnyTransport>) -> Self {
        self.transport.inner = Some(inner.into());
        self
    }

    /// Count `prepare_for_routing` but don't pass it on to the inner
    /// transport, as a wrapper that skips it would.
    pub fn skip_prepare_for_routing(mut self) -> Self {
        self.transport.prepare_inner = false;
        self
    }

    /// Run a callback at the top of every `start`, before the start mode.
    pub fn on_start(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.transport.hooks.on_start = Some(Arc::new(hook));
        self
    }

    /// Run a callback once a `start` has succeeded, with what the transport
    /// then reports.
    pub fn on_started(mut self, hook: impl Fn(Started<'_>) + Send + Sync + 'static) -> Self {
        self.transport.hooks.on_started = Some(Arc::new(hook));
        self
    }

    /// Run a hook on every recorded send, after it is logged and before the
    /// built-in controls. The hook's future is part of the send future, so
    /// dropping the send drops it; its error becomes the send's result.
    pub fn on_send<F>(mut self, hook: impl Fn(SentFrame) -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: SendHook =
            Arc::new(move |frame: SentFrame| -> HookFuture { Box::pin(hook(frame)) });
        self.transport.hooks.on_send = Some(hook);
        self
    }

    /// Await this hook's future in `start`, after `on_start`, and fail the
    /// start with its error: a start the test holds open, to drop it there.
    pub fn start_gate<F>(mut self, hook: impl Fn() -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: StopHook = Arc::new(move || -> HookFuture { Box::pin(hook()) });
        self.transport.hooks.start_gate = Some(hook);
        self
    }

    /// Replace `stop`'s `Ok(())` with this hook's result.
    pub fn on_stop<F>(mut self, hook: impl Fn() -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: StopHook = Arc::new(move || -> HookFuture { Box::pin(hook()) });
        self.transport.hooks.on_stop = Some(hook);
        self
    }

    /// Run a callback when `abort` is called.
    pub fn on_abort(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.transport.hooks.on_abort = Some(Arc::new(hook));
        self
    }

    /// Run a callback when the transport is dropped.
    pub fn on_drop(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.transport.hooks.on_drop = Some(Arc::new(hook));
        self
    }

    /// Attach test-owned state, read back with [`TestTransport::state`].
    pub fn state<T: Any + Send + Sync>(mut self, state: Arc<T>) -> Self {
        self.transport.state = Some(state);
        self
    }

    /// The send log the transport will record to.
    pub fn sent(&self) -> SendLog {
        self.transport.sent()
    }

    /// The handle the transport will answer to.
    pub fn handle(&self) -> TestTransportHandle {
        self.transport.handle()
    }

    /// Finish the transport.
    pub fn build(self) -> TestTransport {
        self.transport
    }
}

#[cfg(test)]
#[path = "test_transport_tests.rs"]
mod tests;

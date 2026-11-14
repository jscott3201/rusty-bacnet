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
//! A transport can stand in front of another: [`TestTransportBuilder::inner`]
//! takes any `TransportPort` (a real link, a double, another `TestTransport`)
//! behind an object-safe adapter, so the type stays concrete. The inner
//! transport answers whatever the test does not set, rule by rule, runs
//! `start`, `stop` and `abort`, and takes the unicasts and broadcasts sent
//! under [`SendMode::Forward`].
//!
//! Every send runs this pipeline:
//! 1. The [`SendMode`] for its kind (unicast or broadcast): `Ignore` returns
//!    `Ok(())` untouched, skipping every later step, `Panic` panics, and
//!    `Fail` records the frame and returns its error.
//! 2. The frame, with any data attributes, is appended to the [`SendLog`].
//! 3. The [`TestTransportBuilder::on_send`] hook, if any, runs to completion;
//!    its error ends the send.
//! 4. Built-in controls: a pending `fail_next_send` is taken; a held send
//!    (`block_next_send`) signals `wait_blocked` and waits for one
//!    `release_sends` permit; then the send fails if `fail_next_send` was
//!    taken.
//! 5. `Forward` only: the send goes on to the inner transport and returns its
//!    result. `ForwardFirst` instead forwards between steps 2 and 3.

use std::any::Any;
use std::future::Future;
use std::net::SocketAddrV4;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use bacnet_npdu::NpduLimits;
use bacnet_types::MacAddr;
use bacnet_types::data_link::DataLink;
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::{mpsc, watch};

use crate::port::{
    Bip6Port, BipPort, DataAttribute, GroupDestinations, ReceivedNpdu, TransportPort,
};

mod builder;
mod handle;
mod inner;
mod log;

pub use builder::TestTransportBuilder;
pub use handle::{InnerLimits, TestTransportHandle};
pub use log::{SendLog, SentFrame};

use handle::Shared;

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

/// How `start` behaves.
pub enum StartMode {
    /// Return a receiver whose sender is already gone: the link is closed.
    /// With an [`inner`](TestTransportBuilder::inner) transport, start it and
    /// return its receiver instead.
    Closed,
    /// Return this receiver, which the test feeds. `start` takes it, so a
    /// second start fails.
    Inbound(Option<mpsc::Receiver<ReceivedNpdu>>),
    /// Make a new channel of this capacity on every start. The handle keeps
    /// the sender ([`TestTransportHandle::inbound`]) until `stop` (after its
    /// hook and inner stop) or `abort` drops it, so the link closes when the stack stops it.
    Fresh(usize),
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
    /// Record the frame, hand it to the inner transport first, and only then
    /// run the hook and the built-in controls, returning their result: a
    /// link that has put the frame on the wire when a later step holds or
    /// fails the send. The inner transport's own error ends the send before
    /// the hook. Without an inner transport the send fails.
    ForwardFirst,
    /// Record the frame, then fail with an error carrying this message,
    /// skipping the hook and the built-in controls.
    Fail(&'static str),
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
    /// Reported until a start succeeds; see
    /// [`TestTransportBuilder::local_mac_until_started`].
    mac_until_started: Option<MacAddr>,
    started: bool,
    broadcast_macs: Vec<MacAddr>,
    /// Group destinations beside the broadcast MACs (#1493).
    group_macs: Vec<MacAddr>,
    start: StartMode,
    unicast: SendMode,
    broadcast: SendMode,
    hooks: Hooks,
    state: Option<Arc<dyn Any + Send + Sync>>,
    shared: Arc<Shared>,
    /// `None` takes the inner transport's answer, or `false`.
    number_controls: Option<bool>,
    /// See [`TestTransportBuilder::data_link`].
    data_link: Option<DataLink>,
    /// See [`TestTransportBuilder::bip_port`].
    bip_port: Option<BipPort>,
    /// See [`TestTransportBuilder::bip6_port`].
    bip6_port: Option<Bip6Port>,
    /// See [`TestTransportBuilder::max_npdu_changes`].
    max_npdu_changes: Option<watch::Receiver<NpduLimits>>,
    /// See [`TestTransportBuilder::inner`].
    inner: Option<Box<dyn inner::InnerPort>>,
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
                mac_until_started: None,
                started: false,
                broadcast_macs: Vec::new(),
                group_macs: Vec::new(),
                start: StartMode::Closed,
                unicast: SendMode::Record,
                broadcast: SendMode::Record,
                hooks: Hooks::default(),
                state: None,
                shared: Arc::default(),
                number_controls: None,
                bip_port: None,
                bip6_port: None,
                data_link: None,
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

    /// Whether any own broadcast or group rule is set.
    fn own_group_rule(&self) -> bool {
        !self.group_macs.is_empty() || self.hooks.is_group_destination.is_some()
    }

    fn own_broadcast_rule(&self) -> bool {
        !self.broadcast_macs.is_empty() || self.hooks.is_broadcast_mac.is_some()
    }

    /// Remember what the inner transport reports now, for the handle.
    fn refresh_inner_limits(&self) {
        if let Some(inner) = &self.inner {
            *self
                .shared
                .inner_limits
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(InnerLimits {
                receive_capacity: inner.receive_capacity(),
                egress_limit: inner.egress_limit(),
                max_npdu: inner.max_npdu(),
            });
        }
    }

    fn close_fresh_channel(&self) {
        self.shared
            .inbound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    async fn send_frame(&self, mode: SendMode, frame: SentFrame) -> Result<(), Error> {
        let (forward, first) = match mode {
            SendMode::Record => (false, false),
            SendMode::Forward => (true, false),
            SendMode::ForwardFirst => (true, true),
            SendMode::Fail(message) => {
                self.shared.sent.push(frame);
                return Err(Error::Encoding(message.into()));
            }
            SendMode::Ignore => return Ok(()),
            SendMode::Panic(message) => panic!("{message}"),
        };
        self.shared.sent.push(frame.clone());
        if forward && first {
            self.forward(&frame).await?;
        }
        if let Some(hook) = &self.hooks.on_send {
            hook(frame.clone()).await?;
        }
        self.shared.controls().await?;
        if forward && !first {
            return self.forward(&frame).await;
        }
        Ok(())
    }

    async fn forward(&self, frame: &SentFrame) -> Result<(), Error> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| Error::Encoding("test transport has no inner transport".into()))?;
        if frame.broadcast {
            inner.send_broadcast(&frame.npdu, &frame.attributes).await
        } else {
            inner
                .send_unicast(&frame.npdu, &frame.mac, &frame.attributes)
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
    fn data_link(&self) -> Option<DataLink> {
        self.data_link
            .or_else(|| self.inner.as_ref().and_then(|i| i.data_link()))
    }

    fn bip_transport(&self) -> Option<&crate::bip::BipTransport> {
        self.inner.as_ref().and_then(|i| i.bip_transport())
    }

    #[cfg(feature = "ipv6")]
    fn bip6_transport(&self) -> Option<&crate::bip6::Bip6Transport> {
        self.inner.as_ref().and_then(|i| i.bip6_transport())
    }

    fn supports_local_nonrouter_number_controls(&self) -> bool {
        match (self.number_controls, &self.inner) {
            (Some(own), _) => own,
            (None, Some(inner)) => inner.number_controls(),
            (None, None) => false,
        }
    }

    fn bip_port(&self) -> Option<BipPort> {
        self.bip_port
            .clone()
            .or_else(|| self.inner.as_ref().and_then(|i| i.bip_port()))
    }

    fn bip6_port(&self) -> Option<Bip6Port> {
        self.bip6_port
            .clone()
            .or_else(|| self.inner.as_ref().and_then(|i| i.bip6_port()))
    }

    fn retain_network_port_lease_internal(&mut self, lease: Arc<()>) -> Result<(), Error> {
        if self.bip_port.is_some() || self.bip6_port.is_some() {
            self.port_lease = Some(lease);
            return Ok(());
        }
        match &mut self.inner {
            Some(inner) => inner.retain_lease(lease),
            None => Err(Error::Encoding("not a B/IP test link".into())),
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
        self.refresh_inner_limits();
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
            StartMode::Fresh(capacity) => {
                let (tx, rx) = mpsc::channel(*capacity);
                *self
                    .shared
                    .inbound
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(tx);
                rx
            }
            StartMode::Panic(message) => panic!("{message}"),
        };
        self.started = true;
        self.refresh_inner_limits();
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
        let result = match (&self.hooks.on_stop, &mut self.inner) {
            (Some(hook), _) => hook().await,
            (None, Some(inner)) => inner.stop().await,
            (None, None) => Ok(()),
        };
        self.close_fresh_channel();
        self.refresh_inner_limits();
        result
    }

    fn abort(&mut self) {
        self.shared.aborts.fetch_add(1, Ordering::SeqCst);
        self.close_fresh_channel();
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
        if !self.started
            && let Some(mac) = &self.mac_until_started
        {
            return mac;
        }
        match (&self.local_mac, &self.inner) {
            (Some(mac), _) => mac,
            (None, Some(inner)) => inner.local_mac(),
            (None, None) => &[1],
        }
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.refresh_inner_limits();
        match (self.shared.settings().receive_capacity, &self.inner) {
            (Some(set), _) => set,
            (None, Some(inner)) => inner.receive_capacity(),
            (None, None) => 1476,
        }
    }

    fn egress_apdu_limit(&self) -> u16 {
        self.refresh_inner_limits();
        match (self.shared.settings().egress_limit, &self.inner) {
            (Some(set), _) => set,
            (None, Some(inner)) => inner.egress_limit(),
            (None, None) => 1476,
        }
    }

    fn max_npdu(&self) -> NpduLimits {
        if let Some(hook) = &self.hooks.max_npdu {
            return hook();
        }
        self.refresh_inner_limits();
        match (self.shared.settings().max_npdu, &self.inner) {
            (Some(own), _) => own,
            (None, Some(inner)) => inner.max_npdu(),
            (None, None) => NpduLimits::default(),
        }
    }

    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>> {
        self.max_npdu_changes
            .clone()
            .or_else(|| self.inner.as_ref().and_then(|i| i.max_npdu_changes()))
    }

    /// The hook, else the broadcast MAC list, else the inner transport's
    /// rule, else `false`.
    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        if let Some(hook) = &self.hooks.is_broadcast_mac {
            return hook(mac);
        }
        if !self.broadcast_macs.is_empty() {
            return self.broadcast_macs.iter().any(|b| b.as_slice() == mac);
        }
        self.inner.as_ref().is_some_and(|i| i.is_broadcast_mac(mac))
    }

    /// With an own group rule (a [`TestTransportBuilder::group_mac`] or an
    /// [`on_is_group_destination`](TestTransportBuilder::on_is_group_destination)
    /// hook): `is_broadcast_mac`, one of those MACs, or that hook. Without
    /// one: the inner transport's rule, unless the broadcast rule is the
    /// test's own, which then stands alone.
    fn is_group_destination(&self, mac: &[u8]) -> bool {
        if self.own_group_rule() {
            return self.is_broadcast_mac(mac)
                || self.group_macs.iter().any(|g| g.as_slice() == mac)
                || self
                    .hooks
                    .is_group_destination
                    .as_ref()
                    .is_some_and(|hook| hook(mac));
        }
        match &self.inner {
            Some(inner) if !self.own_broadcast_rule() => inner.is_group_destination(mac),
            _ => self.is_broadcast_mac(mac),
        }
    }

    /// The broadcast and group MAC lists, copied, so the rule never calls a
    /// hook (a hook-only rule gives an empty one); the inner transport's rule
    /// when the test set no broadcast or group rule at all, list or hook.
    fn group_destinations(&self) -> GroupDestinations {
        if let Some(inner) = &self.inner
            && !self.own_group_rule()
            && !self.own_broadcast_rule()
        {
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
        match &self.inner {
            Some(inner) if !self.own_group_rule() && !self.own_broadcast_rule() => {
                inner.local_broadcast_destinations()
            }
            _ => self.group_destinations(),
        }
    }
}

#[cfg(test)]
mod inner_tests;
#[cfg(test)]
mod tests;

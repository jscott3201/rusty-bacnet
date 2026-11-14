//! The builder for a [`TestTransport`].

use std::any::Any;
use std::future::Future;
use std::net::SocketAddrV4;
use std::sync::Arc;

use bacnet_npdu::NpduLimits;
use bacnet_types::MacAddr;
use bacnet_types::bip_port::BipPortMode;
use bacnet_types::error::Error;
use tokio::sync::{mpsc, watch};

use super::{
    HookFuture, SendHook, SendLog, SendMode, StartMode, Started, StopHook, TestTransport,
    TestTransportHandle,
};
use crate::port::{Bip6Port, BipPort, ReceivedNpdu, TransportPort};

/// Configures a [`TestTransport`]. Defaults: local MAC `[1]`, receive
/// capacity and egress limit 1476, standard NPDU limits, [`StartMode::Closed`],
/// [`SendMode::Record`] for both kinds, no broadcast MACs, no B/IP endpoint,
/// no Number controls, no inner transport, no hooks.
///
/// With an [`inner`](Self::inner) transport, every answer the test does not
/// set falls back to the inner transport's, rule by rule.
pub struct TestTransportBuilder {
    pub(super) transport: TestTransport,
}

impl TestTransportBuilder {
    /// Report `mac` as the local MAC, over an inner transport's.
    pub fn local_mac(mut self, mac: &[u8]) -> Self {
        self.transport.local_mac = Some(MacAddr::from_slice(mac));
        self
    }

    /// Report `mac` as the local MAC until a `start` has succeeded, as a link
    /// that learns its address at start does; after that the MAC set by
    /// [`local_mac`](Self::local_mac), or the inner transport's, or `[1]`.
    pub fn local_mac_until_started(mut self, mac: &[u8]) -> Self {
        self.transport.mac_until_started = Some(MacAddr::from_slice(mac));
        self
    }

    /// Add a MAC that `is_broadcast_mac` recognises (unless a hook overrides).
    /// Setting this or an `on_is_broadcast_mac` hook ends the inner
    /// transport's broadcast rule, and with it its group rules, unless the
    /// test also sets a group rule.
    pub fn broadcast_mac(mut self, mac: &[u8]) -> Self {
        self.transport.broadcast_macs.push(MacAddr::from_slice(mac));
        self
    }

    /// Add a MAC that reaches a group of nodes without being the link's
    /// broadcast, such as a B/IP multicast address: `is_group_destination`
    /// and `group_destinations` recognise it, `is_broadcast_mac` doesn't.
    /// With one set (or an `on_is_group_destination` hook), the inner
    /// transport's group rules no longer apply; its broadcast rule does for
    /// `is_broadcast_mac` and `is_group_destination`. The owned
    /// `group_destinations` copy cannot call the inner transport, so it then
    /// holds only the test's own broadcast and group MACs.
    pub fn group_mac(mut self, mac: &[u8]) -> Self {
        self.transport.group_macs.push(MacAddr::from_slice(mac));
        self
    }

    /// Recognise more group destinations with a hook, beside the broadcast
    /// and group MACs: `is_group_destination` asks it, `group_destinations`
    /// doesn't. With one set, the inner transport's group rule no longer
    /// applies.
    pub fn on_is_group_destination(
        mut self,
        hook: impl Fn(&[u8]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.is_group_destination = Some(Arc::new(hook));
        self
    }

    /// Decide `is_broadcast_mac` with a hook instead of the MAC list or the
    /// inner transport.
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

    /// Opt in (or out) of the local Network Number controls, so a server
    /// spawns its Number worker on this link. Without this the inner
    /// transport's answer applies, or `false`.
    pub fn number_controls(mut self) -> Self {
        self.transport.number_controls = Some(true);
        self
    }

    /// Report `endpoint` as a NORMAL B/IP bind, so a server can register a
    /// Network Port on this link; the transport then holds the port's lease
    /// until `stop` or the drop.
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

    /// Shorthand for `start(StartMode::Replace(Some(receiver)))`: start the
    /// inner transport, but take this receiver instead of its own.
    pub fn replace_inbound(self, receiver: mpsc::Receiver<ReceivedNpdu>) -> Self {
        self.start(StartMode::Replace(Some(receiver)))
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
        self.transport.shared.settings().receive_capacity = Some(capacity);
        self
    }

    /// Report `limit` as the egress APDU limit (1476 at first; an inner
    /// transport's, when one is set and this is not).
    pub fn egress_limit(self, limit: u16) -> Self {
        self.transport.shared.settings().egress_limit = Some(limit);
        self
    }

    /// Report `limits` as the link's NPDU limits (standard at first; an
    /// inner transport's, when one is set and this is not).
    pub fn max_npdu(self, limits: NpduLimits) -> Self {
        self.transport.shared.settings().max_npdu = Some(limits);
        self
    }

    /// Answer `max_npdu` with a hook, so the limit follows state the test
    /// owns.
    pub fn on_max_npdu(mut self, hook: impl Fn() -> NpduLimits + Send + Sync + 'static) -> Self {
        self.transport.hooks.max_npdu = Some(Arc::new(hook));
        self
    }

    /// Publish the changes to the link's NPDU limits on `changes`, as
    /// `max_npdu_changes` does (default: the inner transport's, else none).
    pub fn max_npdu_changes(mut self, changes: watch::Receiver<NpduLimits>) -> Self {
        self.transport.max_npdu_changes = Some(changes);
        self
    }

    /// Stand in front of `inner`: a real transport, a test double or another
    /// test transport. It supplies every answer no setting here names (MAC,
    /// limits, group rules, B/IP ports, Number controls), runs `start`,
    /// `stop` and `abort` (unless a [`StartMode::Inbound`] or
    /// [`StartMode::Fresh`], or an `on_stop` hook, replaces them), receives
    /// `prepare_for_routing`, and takes the sends whose mode is
    /// [`SendMode::Forward`] or [`SendMode::ForwardFirst`]. A Network Port
    /// lease goes to it when the port came from it.
    ///
    /// What replaces what: `Inbound` and `Fresh` replace only `start`; `stop`
    /// and `abort` still reach the inner transport. An `on_stop` hook
    /// replaces the inner `stop`, while `on_abort` is additive (the inner
    /// `abort` still runs).
    pub fn inner(mut self, inner: impl TransportPort + 'static) -> Self {
        self.transport.inner = Some(Box::new(inner));
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

    /// Await this hook's future at the end of a `start` that succeeded, after
    /// `on_started`: a start the test holds open once the link is up (its
    /// error fails the start, but the link was started).
    pub fn after_start<F>(mut self, hook: impl Fn() -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: StopHook = Arc::new(move || -> HookFuture { Box::pin(hook()) });
        self.transport.hooks.after_start = Some(hook);
        self
    }

    /// Filter the answer to `bip_port` with a hook: it gets the port set here
    /// or the inner transport's, and returns what the transport reports. A port
    /// that appears or goes as the link runs.
    pub fn on_bip_port(
        mut self,
        hook: impl Fn(Option<BipPort>) -> Option<BipPort> + Send + Sync + 'static,
    ) -> Self {
        self.transport.hooks.on_bip_port = Some(Arc::new(hook));
        self
    }

    /// Run a callback with each Network Port lease handed to the transport,
    /// before it is held or passed to the inner transport: a witness of the
    /// lease's lifetime.
    pub fn on_retain_lease(mut self, hook: impl Fn(&Arc<()>) + Send + Sync + 'static) -> Self {
        self.transport.hooks.on_retain_lease = Some(Arc::new(hook));
        self
    }

    /// Run a hook on every recorded send, after it is logged and before the
    /// built-in controls. The hook's future is part of the send future, so
    /// dropping the send drops it; its error becomes the send's result.
    pub fn on_send<F>(
        mut self,
        hook: impl Fn(super::SentFrame) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: SendHook =
            Arc::new(move |frame: super::SentFrame| -> HookFuture { Box::pin(hook(frame)) });
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

    /// Await this hook's future at the top of every `stop`, before the
    /// `on_stop` hook or the inner transport's stop; its error ends the stop
    /// with the inner transport untouched, a fresh inbound channel still
    /// open and any held Network Port lease still held. A stop the test
    /// holds open, to drop or release it there, with the rest of the stop
    /// still running.
    pub fn before_stop<F>(mut self, hook: impl Fn() -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: StopHook = Arc::new(move || -> HookFuture { Box::pin(hook()) });
        self.transport.hooks.before_stop = Some(hook);
        self
    }

    /// Await this hook's future after a `stop` that succeeded (the `on_stop`
    /// hook or the inner transport's stop), before the stop returns; its error
    /// becomes the stop's result. Not run when the stop failed.
    pub fn after_stop<F>(mut self, hook: impl Fn() -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let hook: StopHook = Arc::new(move || -> HookFuture { Box::pin(hook()) });
        self.transport.hooks.after_stop = Some(hook);
        self
    }

    /// Replace `stop`'s default (the inner transport's stop, or `Ok(())`)
    /// with this hook's result. The inner `stop` is not called; a
    /// [`StartMode::Fresh`] channel still closes afterwards.
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

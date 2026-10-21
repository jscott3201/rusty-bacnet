//! Following a port's NPDU limit while the router runs (#1696).
//!
//! A port whose transport offers `TransportPort::max_npdu_changes`, as
//! BACnet/SC does, gets a task that gives the engine each limit the
//! transport publishes. It changes the port's limit under the engine's
//! lock, as every other engine change is made, so every frame a dispatch
//! task routes after that goes by the new limit. Nothing waits under the
//! lock: the limit is copied out of the watch before the lock is taken,
//! and the lock is let go before the task waits again.

use std::sync::Arc;

use bacnet_npdu::NpduLimits;
use bacnet_router_core::PortId;
use tokio::sync::watch;
use tracing::{info, warn};

use super::dispatch::Shared;
use super::engine::Engine;

/// Give the engine's `port` each limit `changes` holds, from the one it
/// holds now, until the transport drops its sender.
///
/// The one it holds now comes first, so a change between the start's read
/// of `max_npdu` and this subscription isn't missed.
pub(super) async fn follow(
    shared: Arc<Shared>,
    port: PortId,
    mut changes: watch::Receiver<NpduLimits>,
) {
    loop {
        let reported = *changes.borrow_and_update();
        shared.set_port_limits(port, reported);
        if changes.changed().await.is_err() {
            return;
        }
    }
}

impl Shared {
    /// Give the engine's `port` the limits its transport reports now, as
    /// [`Engine::limits`] takes them, and log a change.
    pub(super) fn set_port_limits(&self, port: PortId, reported: NpduLimits) {
        let limits = Engine::limits(reported);
        let changed = {
            let mut router = self.engine.lock();
            match router.ports().get(usize::from(port.0)) {
                Some(config) if config.max_npdu == limits => Ok(false),
                _ => router.set_port_max_npdu(port, limits).map(|()| true),
            }
        };
        match changed {
            Ok(true) => info!(
                port = port.0,
                unicast = limits.unicast,
                broadcast = limits.broadcast,
                "Router port's NPDU limit changed"
            ),
            Ok(false) => {}
            // The port is the router's and the limits are within the
            // engine's bounds, so this is never expected.
            Err(error) => warn!(port = port.0, %error, "Router kept a port's NPDU limit"),
        }
    }
}

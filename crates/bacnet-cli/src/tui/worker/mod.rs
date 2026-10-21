//! The worker layer: the only part of the TUI that is generic over the
//! transport.
//!
//! It owns the `BACnetClient`, turns [`Command`](super::message::Command)s
//! into requests and forwards client notifications to the UI through a
//! bounded channel. When the UI falls behind, events are dropped and counted
//! rather than queued without limit, and the table is resynchronised from the
//! client's own discovery table afterwards.

pub(crate) mod browse;
pub(crate) mod coordinator;
pub(crate) mod discovery;
pub(crate) mod network;
pub(crate) mod passive;
#[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
pub(crate) mod sc;
pub(crate) mod session;
pub(crate) mod watch;
pub(crate) mod write;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_client::client::{DeviceCollisionEvent, DeviceEvent, DeviceEventKind};
use bacnet_client::discovery::DiscoveredDevice;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use super::message::{DeviceRow, WorkerEvent};
use crate::core::target::AddressStyle;

/// Capacity of the worker-to-UI channel.
pub(crate) const EVENT_CHANNEL_CAPACITY: usize = 1_024;

/// Capacity of the UI-to-worker channel; commands are user actions.
pub(crate) const COMMAND_CHANNEL_CAPACITY: usize = 16;

/// Result of offering an event to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Offer {
    /// Queued.
    Sent,
    /// The channel was full; the event was counted as dropped.
    Dropped,
    /// The UI has gone.
    Closed,
}

/// The sending side of the worker-to-UI channel, with the dropped counter.
#[derive(Clone)]
pub(crate) struct EventSink {
    tx: mpsc::Sender<WorkerEvent>,
    dropped: Arc<AtomicU64>,
    session: Option<(u64, tokio::sync::watch::Receiver<bool>)>,
}

impl EventSink {
    /// A sink and its receiver, holding at most `capacity` events.
    pub(crate) fn channel(capacity: usize) -> (Self, mpsc::Receiver<WorkerEvent>) {
        let (tx, rx) = mpsc::channel(capacity);
        let sink = Self {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
            session: None,
        };
        (sink, rx)
    }

    /// Scope every event to this session and make blocked sends cancellable.
    pub(crate) fn for_session(
        &self,
        generation: u64,
        cancelled: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        Self {
            tx: self.tx.clone(),
            dropped: Arc::clone(&self.dropped),
            session: Some((generation, cancelled)),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|(_, cancelled)| *cancelled.borrow())
    }

    pub(crate) async fn cancelled(&self) {
        let Some((_, receiver)) = &self.session else {
            return std::future::pending().await;
        };
        let mut receiver = receiver.clone();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }

    /// Wait until the UI has dropped its end of the channel.
    pub(crate) async fn closed(&self) {
        self.tx.closed().await;
    }

    fn scoped(&self, event: WorkerEvent) -> WorkerEvent {
        match &self.session {
            Some((generation, _)) => WorkerEvent::Session {
                generation: *generation,
                event: Box::new(event),
            },
            None => event,
        }
    }

    /// The shared dropped-events counter the status bar shows.
    pub(crate) fn dropped_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped)
    }

    /// Count `n` events lost upstream (a lagging broadcast receiver).
    pub(crate) fn add_dropped(&self, n: u64) {
        self.dropped.fetch_add(n, Ordering::Relaxed);
    }

    /// Queue a high-rate event without waiting; drop and count it when full.
    pub(crate) fn offer(&self, event: WorkerEvent) -> Offer {
        let offered = self.offer_uncounted(event);
        if offered == Offer::Dropped {
            self.add_dropped(1);
        }
        offered
    }

    /// [`offer`](Self::offer) for a stream that counts its own losses (the
    /// capture): a refusal leaves the status bar's counter alone.
    pub(crate) fn offer_uncounted(&self, event: WorkerEvent) -> Offer {
        if self.is_cancelled() {
            return Offer::Closed;
        }
        match self.tx.try_send(self.scoped(event)) {
            Ok(()) => Offer::Sent,
            Err(mpsc::error::TrySendError::Full(_)) => Offer::Dropped,
            Err(mpsc::error::TrySendError::Closed(_)) => Offer::Closed,
        }
    }

    /// Queue an event that must not be lost, without waiting: hand it back
    /// when the channel is full so the caller can offer it again later.
    pub(crate) fn offer_or_keep(&self, event: WorkerEvent) -> Option<WorkerEvent> {
        if self.is_cancelled() {
            return None;
        }
        match self.tx.try_send(self.scoped(event)) {
            Err(mpsc::error::TrySendError::Full(event)) => Some(match event {
                WorkerEvent::Session { event, .. } if self.session.is_some() => *event,
                event => event,
            }),
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => None,
        }
    }

    /// Queue a rare control event (connect, operation progress), waiting for
    /// room. Returns false if the UI has gone.
    pub(crate) async fn deliver(&self, event: WorkerEvent) -> bool {
        tokio::select! {
            biased;
            () = self.cancelled() => false,
            result = self.tx.send(self.scoped(event)) => result.is_ok(),
        }
    }
}

/// Folds the client's device and collision broadcasts into UI events.
pub(crate) struct DeviceFeed {
    style: AddressStyle,
    /// Something was lost, so the UI's table may be out of date.
    pub(crate) needs_resync: bool,
    /// `device_events()` is still open.
    pub(crate) devices_open: bool,
    /// `device_collision_events()` is still open.
    pub(crate) collisions_open: bool,
}

impl DeviceFeed {
    /// A feed formatting addresses with `style`.
    pub(crate) fn new(style: AddressStyle) -> Self {
        Self {
            style,
            needs_resync: false,
            devices_open: true,
            collisions_open: true,
        }
    }

    /// Convert a client row for the UI.
    pub(crate) fn row(&self, device: &DiscoveredDevice) -> DeviceRow {
        let routed = device
            .source_network
            .zip(device.source_address.as_ref())
            .map(|(network, station)| (network, station.as_slice()));
        let (address, via) = self.style.locate(device.mac_address.as_slice(), routed);
        DeviceRow {
            instance: device.object_identifier.instance_number(),
            address,
            via,
            network: device.source_network,
            vendor_id: device.vendor_id,
            max_apdu: device.max_apdu_length,
            segmentation: device.segmentation_supported,
            last_seen: device.last_seen,
        }
    }

    /// Handle one receive from `device_events()`.
    pub(crate) fn on_device(&mut self, sink: &EventSink, received: Result<DeviceEvent, RecvError>) {
        match received {
            Ok(event) => {
                let row = self.row(&event.device);
                let event = match event.kind {
                    DeviceEventKind::Discovered => WorkerEvent::Discovered(row),
                    DeviceEventKind::Updated => WorkerEvent::Updated(row),
                    DeviceEventKind::Lost => {
                        // The row leaves the table, so say so in the log pane.
                        tracing::info!("device {} lost ({})", row.instance, row.located());
                        WorkerEvent::Lost(row)
                    }
                };
                if sink.offer(event) == Offer::Dropped {
                    self.needs_resync = true;
                }
            }
            Err(RecvError::Lagged(missed)) => {
                sink.add_dropped(missed);
                self.needs_resync = true;
            }
            Err(RecvError::Closed) => self.devices_open = false,
        }
    }

    /// Handle one receive from `device_collision_events()`, returning the
    /// event to deliver. Collisions are rare and important, so the caller
    /// delivers them with [`EventSink::deliver`] rather than risk a drop.
    pub(crate) fn on_collision(
        &mut self,
        sink: &EventSink,
        received: Result<DeviceCollisionEvent, RecvError>,
    ) -> Option<WorkerEvent> {
        match received {
            Ok(event) => Some(WorkerEvent::Collision {
                retained: self.row(&event.retained),
                incoming: self.row(&event.incoming),
            }),
            Err(RecvError::Lagged(missed)) => {
                sink.add_dropped(missed);
                None
            }
            Err(RecvError::Closed) => {
                self.collisions_open = false;
                None
            }
        }
    }
}

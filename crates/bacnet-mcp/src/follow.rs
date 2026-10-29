//! What one subscriber follows ([`Follow`]): a listen stream's URIs, or a
//! legacy session's subscriptions, waited on without a lock (#1498).

use std::collections::HashMap;
use std::time::Duration;

use bacnet_client::client::{DeviceEvent, DeviceEventKind};
use bacnet_ops::engine::Engine;
use bacnet_ops::watch::Changes;
use bacnet_transport::port::TransportPort;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::time::{Instant, Interval};

use crate::listen::ALARMS_EVERY;
use crate::resources::uri::{self, Uri};

/// The fastest the device list is announced as changed.
const DEVICES_EVERY: Duration = Duration::from_secs(1);

/// What one subscriber follows: its points' change signals, the
/// discovery table's events and the alarm tick. A listen stream's set is
/// fixed; a legacy session's follows its subscribes and unsubscribes
/// ([`Follow::set`]). [`Follow::next`] waits on watch and broadcast
/// channels and timers only, never a lock (#1498), and is cancel safe.
pub(crate) struct Follow {
    points: Vec<(String, Changes)>,
    devices: Option<broadcast::Receiver<DeviceEvent>>,
    devices_due: Option<Instant>,
    devices_last: Option<Instant>,
    device_uris: Vec<String>,
    alarm_uris: Vec<String>,
    alarms: Interval,
}

/// Why [`Follow::next`] woke.
pub(crate) enum Wake {
    /// This resource changed.
    Updated(String),
    /// A device arrived or was lost: every device-list page followed
    /// ([`Follow::device_uris`]) changed, since each one counts `total`.
    Devices,
    /// The alarm resources are due for a poll.
    Alarms,
}

impl Follow {
    pub(crate) fn new() -> Self {
        let mut alarms = tokio::time::interval_at(Instant::now() + ALARMS_EVERY, ALARMS_EVERY);
        alarms.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self {
            points: Vec::new(),
            devices: None,
            devices_due: None,
            devices_last: None,
            device_uris: Vec::new(),
            alarm_uris: Vec::new(),
            alarms,
        }
    }

    /// Follow `uris`, each point URI with its change signal. A point
    /// already followed on the same watch keeps the signal it had, so
    /// nothing is announced twice.
    /// `engine` gives the discovery table's events, when the device list
    /// is followed.
    pub(crate) fn set<T: TransportPort + 'static>(
        &mut self,
        engine: Option<&Engine<T>>,
        uris: Vec<(String, Option<Changes>)>,
    ) {
        let mut kept: HashMap<String, Changes> =
            std::mem::take(&mut self.points).into_iter().collect();
        self.device_uris.clear();
        self.alarm_uris.clear();
        for (name, changes) in uris {
            if let Some(changes) = changes {
                // The old signal only while it is the same watch: a URI
                // unsubscribed and subscribed again between two calls has a
                // new one, and the old one never fires again.
                let changes = match kept.remove(&name) {
                    Some(old) if old.same(&changes) => old,
                    _ => changes,
                };
                self.points.push((name, changes));
            } else {
                match uri::parse(&name) {
                    Ok(Uri::Devices | Uri::DevicesAfter(_)) => self.device_uris.push(name),
                    Ok(Uri::Alarms | Uri::DeviceAlarms(_)) => self.alarm_uris.push(name),
                    _ => {}
                }
            }
        }
        if self.device_uris.is_empty() {
            self.devices = None;
            self.devices_due = None;
        } else if self.devices.is_none() {
            self.devices = engine.map(Engine::device_events);
        }
    }

    /// The signal followed for `name`, for tests.
    #[cfg(test)]
    pub(crate) fn signal(&self, name: &str) -> Option<&Changes> {
        self.points
            .iter()
            .find(|(followed, _)| followed == name)
            .map(|(_, changes)| changes)
    }

    /// Whether nothing followed can change.
    pub(crate) fn quiet(&self) -> bool {
        self.points.is_empty() && self.devices.is_none() && self.alarm_uris.is_empty()
    }

    /// The device-list pages followed, the first and any later ones.
    pub(crate) fn device_uris(&self) -> &[String] {
        &self.device_uris
    }

    /// The alarm resources followed.
    pub(crate) fn alarm_uris(&self) -> &[String] {
        &self.alarm_uris
    }

    /// The next change to announce, or the alarm tick; pending while
    /// nothing followed can change.
    pub(crate) async fn next(&mut self) -> Wake {
        loop {
            tokio::select! {
                biased;
                at = point_change(&mut self.points), if !self.points.is_empty() => {
                    return Wake::Updated(self.points[at].0.clone());
                }
                change = device_change(&mut self.devices), if self.devices.is_some() => {
                    match change {
                        None => self.devices = None,
                        Some(true) if self.devices_due.is_none() => {
                            let earliest = self
                                .devices_last
                                .map_or_else(Instant::now, |at| at + DEVICES_EVERY);
                            self.devices_due = Some(earliest.max(Instant::now()));
                        }
                        Some(_) => {}
                    }
                }
                () = tokio::time::sleep_until(self.devices_due.unwrap_or_else(Instant::now)),
                    if self.devices_due.is_some() => {
                    self.devices_due = None;
                    self.devices_last = Some(Instant::now());
                    return Wake::Devices;
                }
                _ = self.alarms.tick(), if !self.alarm_uris.is_empty() => return Wake::Alarms,
                else => std::future::pending::<()>().await,
            }
        }
    }
}

/// The index of the next point to change. Each signal's wait is cancel
/// safe, so the ones not first lose nothing.
async fn point_change(points: &mut [(String, Changes)]) -> usize {
    let mut waiting: FuturesUnordered<_> = points
        .iter_mut()
        .enumerate()
        .map(|(at, (_, changes))| async move {
            changes.changed().await;
            at
        })
        .collect();
    match waiting.next().await {
        Some(at) => at,
        None => std::future::pending().await,
    }
}

/// Whether the discovery table's next event is an arrival or a loss (a
/// lost receiver position counts, since one may have been missed); `None`
/// once the channel is closed.
async fn device_change(events: &mut Option<broadcast::Receiver<DeviceEvent>>) -> Option<bool> {
    match events.as_mut()?.recv().await {
        Ok(event) => Some(!matches!(event.kind, DeviceEventKind::Updated)),
        Err(RecvError::Lagged(_)) => Some(true),
        Err(RecvError::Closed) => None,
    }
}

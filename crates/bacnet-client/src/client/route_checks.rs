//! The client's own check of a routed path (#2823).
//!
//! The discovery table holds a routed I-Am that names a known address
//! through another station than the router its row names: the row keeps its
//! router, and the I-Am is a cue (`DeviceTable::held_route`). The purge task
//! takes the cues and checks each path, a network and the router its rows
//! name, with requests of its own:
//!
//! 1. A Who-Is to the cued station, sent through the router the row names.
//!    Any row on the network heard through that router since the check began
//!    (an I-Am it carried, or an answer to a request sent through it) keeps
//!    the path. The other station cannot send as the router, so it cannot
//!    fake that answer.
//! 2. Only if nothing came, a broadcast Who-Is-Router-To-Network for the
//!    network (Clause 6.6.3.2). The old router among the answers keeps the
//!    path: the device is quiet, not the router gone. One other router alone
//!    moves every row on the network that went through the old one to it
//!    (`DeviceTable::reroute`). No answer, several, or an answer lost to a
//!    lagging receiver keep the path.
//!
//! A router that never answers router-discovery queries, as some older ones
//! don't, keeps its rows for as long as its devices answer through it; when
//! it is really gone, its rows age out of the table and the devices' next
//! I-Ams through the new router make them again, or `add_routed_device` or
//! a [`Station::routed`] call names the new one at once.
//!
//! Each path is checked at once on its first cue, then at most once per
//! hold, which starts at a minute and doubles to the ten minutes a silent
//! row takes to age out, while cues keep coming. Cues for one path between
//! checks cost nothing more: a storm of I-Ams costs one check per hold per
//! path, never one per I-Am.

use super::device_events::TableNotices;
use super::i_have::collect_until;
use super::network_number::Discovery;
use super::*;
use crate::discovery::PathKey;
use bacnet_network::layer::{NetworkReport, ReceivedNetworkReport};
use bacnet_npdu::NetworkMessageType;
use std::collections::BTreeMap;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant as TokioInstant;

/// How long after a path's first check the next may run. Each check that
/// leaves the path as it was doubles the hold, up to [`LAST_HOLD`].
pub(super) const FIRST_HOLD: Duration = Duration::from_secs(60);

/// The longest hold: as long as a silent row takes to age out of the table.
pub(super) const LAST_HOLD: Duration = super::lifecycle::DEVICE_MAX_AGE;

/// The most paths paced at once. A cue for another path past that is
/// dropped, and the path stays as it is.
const MAX_PATHS: usize = 256;

/// One path's pacing.
#[derive(Debug)]
struct Paced {
    /// The station the latest cue named, which the next check asks.
    probe: MacAddr,
    /// A cue came since the last check.
    wanted: bool,
    /// The next check runs no sooner.
    due: TokioInstant,
    /// The hold after the next check.
    hold: Duration,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Found {
    /// A row on the network was heard through its router after the check
    /// began.
    Answered,
    /// The router still answers for the network, or no one other router
    /// alone does.
    Kept,
    /// Another router alone answered for the network, and the rows that
    /// went through the old one go through it now.
    Moved(MacAddr),
    /// No row on the network goes through the router any more.
    Gone,
    /// Nothing decided: the node may not initiate, or a send failed.
    Skipped,
}

/// The routers that answered a Who-Is-Router-To-Network for one network.
#[derive(Debug, Default)]
struct Routers {
    /// The router the rows name answered.
    old: bool,
    /// The others that answered, at most two: one decides, two refuse.
    others: Vec<MacAddr>,
}

/// The path checks the purge task runs, and their pacing, which lives in
/// that task alone.
pub(super) struct RouteChecks<T: TransportPort> {
    network: Arc<NetworkLayer<T>>,
    reports: broadcast::Sender<ReceivedNetworkReport>,
    /// How long each step waits for its answer: the client's APDU timeout.
    wait: Duration,
    paths: BTreeMap<PathKey, Paced>,
    /// A claim the check did not take: a second router to the network, a
    /// router that took over unconfirmed, or a forged announcement.
    kept: WarnThrottle,
}

impl<T: TransportPort + 'static> RouteChecks<T> {
    pub(super) fn new(
        network: Arc<NetworkLayer<T>>,
        reports: broadcast::Sender<ReceivedNetworkReport>,
        wait: Duration,
    ) -> Self {
        Self {
            network,
            reports,
            wait,
            paths: BTreeMap::new(),
            kept: WarnThrottle::default(),
        }
    }

    /// When the next check is due: the earliest path with a cue waiting.
    pub(super) fn next_due(&self) -> Option<TokioInstant> {
        self.paths
            .values()
            .filter(|paced| paced.wanted)
            .map(|paced| paced.due)
            .min()
    }

    /// Take the table's cues, then run every check that is due, one at a
    /// time. No lock is held across a send or a wait.
    pub(super) async fn run(&mut self, discovery: &Discovery, notices: TableNotices<'_>) {
        let cues = discovery.table.lock().take_route_cues();
        let now = TokioInstant::now();
        for (path, probe) in cues {
            if let Some(paced) = self.paths.get_mut(&path) {
                paced.probe = probe;
                paced.wanted = true;
            } else if self.paths.len() < MAX_PATHS {
                let paced = Paced {
                    probe,
                    wanted: true,
                    due: now,
                    hold: FIRST_HOLD,
                };
                self.paths.insert(path, paced);
            }
        }
        // A path left alone for a whole hold past its due is forgotten:
        // its next cue is checked at once again.
        self.paths.retain(|_, paced| {
            paced.wanted || paced.due.checked_add(LAST_HOLD).is_none_or(|end| now < end)
        });
        let due: Vec<(PathKey, MacAddr)> = self
            .paths
            .iter()
            .filter(|(_, paced)| paced.wanted && paced.due <= now)
            .map(|(path, paced)| (path.clone(), paced.probe.clone()))
            .collect();
        for (path, probe) in due {
            let found = self.check(discovery, &path, &probe, notices).await;
            if matches!(found, Found::Moved(_) | Found::Gone) {
                self.paths.remove(&path);
            } else if let Some(paced) = self.paths.get_mut(&path) {
                paced.wanted = false;
                paced.due = TokioInstant::now() + paced.hold;
                paced.hold = (paced.hold * 2).min(LAST_HOLD);
            }
        }
    }

    /// Check one path: ask through its router, then, only if nothing came,
    /// ask which router reaches its network.
    async fn check(
        &self,
        discovery: &Discovery,
        (network, router): &PathKey,
        probe: &MacAddr,
        (device_tx, _): TableNotices<'_>,
    ) -> Found {
        let network = *network;
        if !discovery.table.lock().routes_through(network, router) {
            return Found::Gone;
        }
        // A node that may not initiate sends no Who-Is (#2617): with nothing
        // asked, nothing moves.
        if discovery.asking.refusal().is_some() {
            return Found::Skipped;
        }
        let since = crate::discovery::now();
        // Subscribed before the send, so no answer comes before the
        // receiver.
        let mut events = device_tx.subscribe();
        if let Err(error) = self.probe(network, probe, router).await {
            debug!(%error, network, "a routed path's check sent no Who-Is; the path stays");
            return Found::Skipped;
        }
        let deadline = TokioInstant::now() + self.wait;
        heard_through(&mut events, deadline, network, router).await;
        if discovery
            .table
            .lock()
            .heard_through_since(network, router, since)
        {
            self.kept(network, router, "the device answered through the router");
            return Found::Answered;
        }
        // A network message, not a service: a node that restricts
        // initiation still sends it, one whose seat has closed doesn't.
        if discovery.asking.is_sealed() {
            return Found::Skipped;
        }
        let routers = match self.routers_to(network, router).await {
            Ok(Some(routers)) => routers,
            Ok(None) => {
                self.kept(
                    network,
                    router,
                    "router answers were lost to a lagging receiver",
                );
                return Found::Kept;
            }
            Err(error) => {
                debug!(%error, network, "a routed path's check sent no router query; the path stays");
                return Found::Skipped;
            }
        };
        let to = match routers.others.as_slice() {
            [to] if !routers.old => to.clone(),
            _ => {
                let why = if routers.old {
                    "the router still answers for the network"
                } else if routers.others.is_empty() {
                    "no router answered for the network"
                } else {
                    "several other routers answered for the network"
                };
                self.kept(network, router, why);
                return Found::Kept;
            }
        };
        let mut table = discovery.table.lock();
        // The old router carried something back meanwhile: it still works.
        if table.heard_through_since(network, router, since) {
            return Found::Answered;
        }
        let moved = table.reroute(network, router, &to);
        warn!(
            network,
            from = ?router,
            to = ?to,
            devices = moved.len(),
            "requests to network {network} now go through another router: the old one carried \
             no answer to this client's Who-Is and didn't answer its router query, which one \
             other router alone answered"
        );
        // Under the table's lock, as every device notice goes out.
        for device in moved {
            let _ = device_tx.send(DeviceEvent {
                kind: DeviceEventKind::Updated,
                device,
            });
        }
        Found::Moved(to)
    }

    /// A Who-Is to `station` on `network`, through `router`.
    async fn probe(&self, network: u16, station: &[u8], router: &[u8]) -> Result<(), Error> {
        let apdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        });
        let mut buf = BytesMut::with_capacity(2);
        encode_apdu(&mut buf, &apdu)?;
        self.network
            .send_apdu_routed(
                &buf,
                network,
                station,
                router,
                false,
                NetworkPriority::Normal,
            )
            .await
    }

    /// Broadcast a Who-Is-Router-To-Network for `network` and collect, for
    /// the wait, the routers on this link that answer for it: `None` when
    /// answers were lost to a lagging receiver, which leaves the question
    /// open.
    async fn routers_to(&self, network: u16, old: &MacAddr) -> Result<Option<Routers>, Error> {
        let mut rx = self.reports.subscribe();
        self.network
            .send_network_message(
                NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
                &network.to_be_bytes(),
                None,
            )
            .await?;
        let deadline = TokioInstant::now() + self.wait;
        let mut routers = Routers::default();
        let lost = collect_until(&mut rx, deadline, "I-Am-Router-To-Network", |received| {
            let NetworkReport::IAmRouterToNetwork(listed) = received.report else {
                return;
            };
            // A router on this link, answering for the network asked about.
            if received.source_network.is_some() || !listed.contains(&network) {
                return;
            }
            if received.source_mac == *old {
                routers.old = true;
            } else if routers.others.len() < 2 && !routers.others.contains(&received.source_mac) {
                routers.others.push(received.source_mac);
            }
        })
        .await;
        Ok((lost == 0).then_some(routers))
    }

    /// Another station claimed to reach devices on `network`, and the check
    /// kept the path through `router`, for the reason `why`.
    fn kept(&self, network: u16, router: &MacAddr, why: &'static str) {
        warn_throttled!(
            self.kept,
            network,
            router = ?router,
            why,
            "another station than their router announced devices on network {network}; \
             requests keep going through the router: a second router to the network, a \
             router that took over and must be confirmed, or a forged announcement"
        );
    }
}

/// Wait until `deadline` for a device notice of a row on `network` that
/// goes through `router`. The table's own record decides afterwards, so a
/// lost notice only makes the wait longer.
async fn heard_through(
    events: &mut broadcast::Receiver<DeviceEvent>,
    deadline: TokioInstant,
    network: u16,
    router: &MacAddr,
) {
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(event))
                if event.kind != DeviceEventKind::Lost
                    && event.device.source_network == Some(network)
                    && event.device.mac_address == *router =>
            {
                return;
            }
            Ok(Ok(_) | Err(RecvError::Lagged(_))) => {}
            Ok(Err(RecvError::Closed)) | Err(_) => return,
        }
    }
}

#[cfg(test)]
#[path = "route_checks_tests.rs"]
mod tests;

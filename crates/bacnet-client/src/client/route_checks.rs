//! The client's own check of a known routed path (#2823).
//!
//! The discovery table holds a routed I-Am that names a known address
//! through another station than the router its row names: the row keeps
//! its router, and the path, the network and that router, is marked for a
//! check (`DeviceTable::held_route`). This loop checks each marked path
//! with requests of its own:
//!
//! 1. A Who-Is through the path's router to the station on that network
//!    whose I-Am the table took through it last (then the one heard from
//!    last), the client's choice, never the one the claim named. Any frame
//!    that router carries back from the network after the check began
//!    keeps the path. The other station cannot send as the router, so it
//!    cannot fake that, with two exceptions that can only keep a path or
//!    pick the probe, never move one: an answer the client's transaction
//!    matching accepted, and on B/IP a forwarded frame whose origin a BBMD
//!    gives as the router's (#2829).
//! 2. Only if nothing came, the client broadcasts its router query
//!    (Who-Is-Router-To-Network) for that network (Clause 6.6.3.2). The old
//!    router among the answers keeps the path: the device is quiet, not the
//!    router gone. No answer, several, or an answer that may have been lost
//!    (a lagging report receiver, or a network control the layer's queue
//!    turned away during the wait) keep it too. A seated client can't see
//!    its node's control queue, so for it every answer may have been lost
//!    and this step never strikes. One other router alone is a strike.
//!
//! The first strike schedules a second check a minute later, however
//! far the path's hold had grown. Only when that one strikes too, with the
//! same router and nothing carried back through the old one since the first
//! began, does every row on the network that went through the old router
//! move to the new one (`DeviceTable::reroute`). Anything else clears the
//! strike, and the hold paces the path again. At the default 6 s APDU
//! timeout a real failover is taken 84 s after the first I-Am through the
//! new router at best: two checks of 12 s, a minute apart. A path checked
//! not long before waits out its hold first (up to ten minutes), other due
//! checks may run first, and a path whose first check comes after its rows
//! aged out is gone: the next I-Am through the new router is first sight.
//! Meanwhile requests go through the old router and end as on any silent
//! path, with `RemoteError::Abort` (`TSM_TIMEOUT`) after their retries.
//!
//! A router that never answers router-discovery queries, as some older
//! ones don't, keeps its rows for as long as its devices answer through
//! it; when it is really gone, its rows age out of the table and the
//! devices' next I-Ams through the new router make them again, or
//! `add_routed_device` or a [`Station::routed`] call names the new one at
//! once.
//!
//! The loop runs in the purge task beside the purge, polled with it, so a
//! check's waits never hold a purge back. Each path is checked at once on
//! its first mark, then at most once per hold, which starts at a minute and
//! doubles to ten while marks keep coming; a wake runs at most
//! [`CHECKS_PER_WAKE`] checks, the paths with the most and the longest
//! known rows first.

use super::device_events::TableNotices;
use super::i_have::collect_until;
use super::network_number::Discovery;
use super::*;
use crate::discovery::{Checked, PathKey, Strike};
use bacnet_network::layer::{NetworkReport, QueueAdmissionCounters, ReceivedNetworkReport};
use bacnet_npdu::NetworkMessageType;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant as TokioInstant;

/// The most checks one wake runs before the loop looks again for marks and
/// for the paths ranked highest.
pub(super) const CHECKS_PER_WAKE: usize = 4;

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Found {
    /// A row on the network was heard through its router after the check
    /// began.
    Answered,
    /// The router still answers for the network, or no one other router
    /// alone does, or an answer may have been lost.
    Kept,
    /// Another router alone answered for the network, the first time: the
    /// second check decides.
    Struck(Strike),
    /// The same other router alone answered the second check too, and the
    /// rows that went through the old one go through it now.
    Moved(MacAddr),
    /// No row on the network goes through the router any more.
    Gone,
    /// Nothing decided: the node may not initiate, no row there can be
    /// asked, or a send failed.
    Skipped,
}

/// The routers that answered one router query (Who-Is-Router-To-Network).
#[derive(Debug, Default)]
struct Routers {
    /// The router the rows name answered.
    old: bool,
    /// The others that answered, at most two: one decides, two refuse.
    others: Vec<MacAddr>,
}

/// The path checks and what they send with.
pub(super) struct RouteChecks<T: TransportPort> {
    network: Arc<NetworkLayer<T>>,
    reports: broadcast::Sender<ReceivedNetworkReport>,
    /// The network controls the layer's queue turned away: one during a
    /// router query's wait may have been an answer. `None` for a seated
    /// client, whose controls come through its node's queues: it can't
    /// know, so it takes every router query's answers as possibly lost.
    control_drops: Option<QueueAdmissionCounters>,
    /// How long each step waits for its answer: the client's APDU timeout.
    wait: Duration,
    /// A claim the check did not take: a second router to the network, a
    /// router that took over unconfirmed, or a forged announcement.
    kept: WarnThrottle,
}

impl<T: TransportPort + 'static> RouteChecks<T> {
    pub(super) fn new(
        network: Arc<NetworkLayer<T>>,
        reports: broadcast::Sender<ReceivedNetworkReport>,
        control_drops: Option<QueueAdmissionCounters>,
        wait: Duration,
    ) -> Self {
        Self {
            network,
            reports,
            control_drops,
            wait,
            kept: WarnThrottle::default(),
        }
    }

    /// Check the marked paths as they fall due, for as long as the client
    /// runs. No lock is held across a send or a wait, so the task can be
    /// aborted at any await.
    pub(super) async fn run(self, discovery: &Discovery, notices: TableNotices<'_>) {
        loop {
            let due = discovery.table.lock().next_path_due();
            tokio::select! {
                () = discovery.route_cue.notified() => {}
                () = sleep_until_due(due) => {}
            }
            let paths = discovery
                .table
                .lock()
                .due_paths(crate::discovery::now(), CHECKS_PER_WAKE);
            for path in paths {
                let found = self.check(discovery, &path, notices).await;
                let checked = match found {
                    Found::Moved(_) | Found::Gone => Checked::Ended,
                    Found::Struck(strike) => Checked::Struck(strike),
                    Found::Answered | Found::Kept | Found::Skipped => Checked::Kept,
                };
                discovery
                    .table
                    .lock()
                    .path_checked(&path, checked, crate::discovery::now());
            }
        }
    }

    /// Check one path: ask through its router, then, only if nothing came,
    /// ask which router reaches its network.
    async fn check(
        &self,
        discovery: &Discovery,
        path: &PathKey,
        (device_tx, _): TableNotices<'_>,
    ) -> Found {
        let (network, router) = (path.0, &path.1);
        let (probe, strike) = {
            let table = discovery.table.lock();
            if !table.routes_through(network, router) {
                return Found::Gone;
            }
            (table.probe_target(network, router), table.path_strike(path))
        };
        let Some(probe) = probe else {
            return Found::Skipped;
        };
        // A node that may not initiate sends no Who-Is (#2617): with nothing
        // asked, nothing moves.
        if discovery.asking.refusal().is_some() {
            return Found::Skipped;
        }
        let began = crate::discovery::now();
        // A second check counts what the old router carried since the first
        // began.
        let since = strike.as_ref().map_or(began, |strike| strike.since);
        // Subscribed before the send, so no answer comes before the
        // receiver.
        let mut events = device_tx.subscribe();
        if let Err(error) = self.probe(network, &probe, router).await {
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
                self.kept(network, router, "router answers may have been lost");
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
        match strike {
            None => {
                return Found::Struck(Strike { to, since: began });
            }
            Some(first) if first.to != to => {
                drop(table);
                self.kept(network, router, "another router answered alone each time");
                return Found::Kept;
            }
            Some(_) => {}
        }
        let moved = table.reroute(network, router, &to);
        warn!(
            network,
            from = ?router,
            to = ?to,
            devices = moved.len(),
            "requests to network {network} now go through another router: in two checks a \
             minute apart the old one carried no answer to this client's Who-Is or router \
             query, and the same other router alone answered"
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

    /// The controls the layer's queue has turned away so far: `None` when
    /// this client can't know.
    fn controls_dropped(&self) -> Option<u64> {
        self.control_drops.as_ref().map(|drops| {
            let counts = drops.snapshot();
            counts.full_drops.saturating_add(counts.closed_drops)
        })
    }

    /// Broadcast the router query (Who-Is-Router-To-Network) for `network`
    /// and collect, for the wait, the routers on this link that answer for
    /// it: `None` when an answer may have been lost, to a lagging report
    /// receiver or to the layer's control queue, or when this client can't
    /// know (a seat), which leaves the question open.
    async fn routers_to(&self, network: u16, old: &MacAddr) -> Result<Option<Routers>, Error> {
        let mut rx = self.reports.subscribe();
        let dropped = self.controls_dropped();
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
        let whole = lost == 0
            && matches!((dropped, self.controls_dropped()), (Some(before), Some(after)) if before == after);
        Ok(whole.then_some(routers))
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

/// Sleep until `due`, or forever with nothing due.
async fn sleep_until_due(due: Option<std::time::Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(TokioInstant::from_std(due)).await,
        None => std::future::pending().await,
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

#[cfg(test)]
#[path = "route_checks_attack_tests.rs"]
mod attack_tests;

#[cfg(test)]
#[path = "route_checks_strike_tests.rs"]
mod strike_tests;

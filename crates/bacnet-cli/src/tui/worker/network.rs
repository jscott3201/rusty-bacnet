//! The Network screen's worker (#966): polls the foreign-device
//! registration once a second and reads BBMD tables when asked.
//!
//! It runs beside the session loop under `join!`, so a survey waiting on a
//! silent BBMD holds up nothing else. Requests run one at a time, in order;
//! a request already queued is not queued twice. The table reads hold no
//! lock across an await: each is one BVLC exchange through the transport,
//! and the transport's own timer fails one that goes unanswered, so a read
//! this loop is not polling for a moment still ends on time. A survey reads
//! up to [`SURVEY_AT_ONCE`](bacnet_ops::bbmd::SURVEY_AT_ONCE) BDTs at once
//! (#1823).

use std::any::Any;
use std::collections::VecDeque;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_ops::bbmd::{Here, Probe, Unfollowed};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use tokio::sync::mpsc;

use super::EventSink;
use crate::tui::message::WorkerEvent;
use crate::tui::network::{
    BdtRow, FdtRow, NetworkCommand, NetworkEvent, address_of, mac, probe_error, read_error,
};

/// How often the registration is polled; the countdown shows whole seconds.
const STATUS_EVERY: Duration = Duration::from_secs(1);

// The survey's bounds: a larger set is drawn with the rest unread (a BDT
// of a hundred entries would not fit a terminal anyway).
#[cfg(test)]
pub(crate) use bacnet_ops::bbmd::{MAX_SURVEY, SURVEY_AT_ONCE};

/// Requests waiting behind the running one.
const MAX_QUEUED: usize = 8;

/// The B/IP transport, when the client has one.
fn as_bip<T: TransportPort + 'static>(transport: &T) -> Option<&BipTransport> {
    (transport as &dyn Any).downcast_ref()
}

/// The running request's event. Never ready while there is none.
async fn finished<F: Future<Output = NetworkEvent> + Unpin>(job: &mut Option<F>) -> NetworkEvent {
    match job {
        Some(job) => job.await,
        None => std::future::pending().await,
    }
}

/// Serve the Network screen until the session ends.
pub(crate) async fn run<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    sink: &EventSink,
    mut commands: mpsc::Receiver<NetworkCommand>,
) {
    let bip = as_bip(client.transport());
    let local =
        address_of(client.local_mac()).unwrap_or(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
    let mut status = tokio::time::interval(STATUS_EVERY);
    status.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut queue: VecDeque<NetworkCommand> = VecDeque::new();
    let mut running: Option<NetworkCommand> = None;
    let mut job = None;
    let mut registered = false;
    loop {
        if job.is_none()
            && let (Some(bip), Some(next)) = (bip, queue.pop_front())
        {
            running = Some(next);
            job = Some(Box::pin(execute(bip, next, local)));
        }
        tokio::select! {
            biased;
            () = sink.cancelled() => break,
            command = commands.recv() => match command {
                None => break,
                Some(command) if bip.is_none() => {
                    let reason = "the Network screen needs BACnet/IP".to_string();
                    sink.deliver(WorkerEvent::Network(NetworkEvent::Refused { command, reason })).await;
                }
                Some(command) if running == Some(command) || queue.contains(&command) => {}
                Some(command) if queue.len() >= MAX_QUEUED => {
                    let reason = "the network worker is busy; try again".to_string();
                    sink.deliver(WorkerEvent::Network(NetworkEvent::Refused { command, reason })).await;
                }
                Some(command) => queue.push_back(command),
            },
            event = finished(&mut job), if job.is_some() => {
                job = None;
                running = None;
                sink.deliver(WorkerEvent::Network(event)).await;
            }
            _ = status.tick(), if bip.is_some() => {
                let Some(bip) = bip else { continue };
                let snapshot = bip.bvlc_client_snapshot().foreign_registration;
                if let Some(bbmd) = snapshot.last_bbmd {
                    // Newly registered: read the FDT to see the entry the
                    // BBMD recorded for us.
                    let tables = NetworkCommand::Tables { bbmd };
                    if snapshot.registered && !registered && running != Some(tables) && !queue.contains(&tables) {
                        queue.push_back(tables);
                    }
                    registered = snapshot.registered;
                    sink.offer(WorkerEvent::Network(NetworkEvent::Registration(snapshot)));
                }
            }
        }
    }
}

/// Carry out one request.
async fn execute(bip: &BipTransport, command: NetworkCommand, local: SocketAddrV4) -> NetworkEvent {
    match command {
        NetworkCommand::Tables { bbmd } => {
            let target = mac(bbmd);
            let bdt = bip
                .read_bdt(&target)
                .await
                .map(|rows| rows.iter().map(BdtRow::from).collect())
                .map_err(|error| read_error(&error));
            let fdt = bip
                .read_fdt(&target)
                .await
                .map(|rows| rows.iter().map(FdtRow::from).collect())
                .map_err(|error| read_error(&error));
            NetworkEvent::Tables {
                bbmd,
                local,
                bdt,
                fdt,
            }
        }
        NetworkCommand::Survey { seed } => {
            let here = Here {
                own: local,
                broadcast: bip.broadcast_address(),
            };
            let (probes, not_followed) = survey(bip, seed, here).await;
            NetworkEvent::Survey {
                seed,
                probes,
                not_followed,
            }
        }
    }
}

/// Read the BDT of `seed`, then of every BBMD a table read lists, breadth
/// first, up to [`MAX_SURVEY`](bacnet_ops::bbmd::MAX_SURVEY) reads; the rest
/// come back skipped. `here` is where this client is: entries no read may
/// go to are not followed ([`bacnet_ops::bbmd::NotFollowed`]), and come
/// back beside the probes.
async fn survey(bip: &BipTransport, seed: SocketAddrV4, here: Here) -> Surveyed {
    survey_with(seed, here, |bbmd| async move {
        match bip.read_bdt(&mac(bbmd)).await {
            Ok(rows) => Probe::Table(rows.iter().map(BdtRow::from).collect()),
            Err(error) => probe_error(&error),
        }
    })
    .await
}

/// [`survey`] with `read` giving each BBMD's probe: the shared survey
/// ([`bacnet_ops::bbmd::survey`]) from one seed. The BBMDs past the
/// survey's limit come back skipped, and the entries not followed beside
/// the probes, for the matrix view to list with their reasons (#2130).
async fn survey_with<F, R>(seed: SocketAddrV4, here: Here, read: F) -> Surveyed
where
    F: Fn(SocketAddrV4) -> R,
    R: Future<Output = Probe>,
{
    let read = &read;
    let found =
        bacnet_ops::bbmd::survey(&[seed], here, |bbmd| async move { (read(bbmd).await, ()) }).await;
    let read = found.read.into_iter().map(|s| (s.bbmd, s.probe));
    let unread = found.unread.into_iter().map(|bbmd| (bbmd, Probe::Skipped));
    (read.chain(unread).collect(), found.not_followed)
}

/// A survey's probes, in the order it found the BBMDs, and the entries it
/// did not follow.
type Surveyed = (Vec<(SocketAddrV4, Probe)>, Vec<Unfollowed>);

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;

//! One bounded serial owner; raw Reject/APDU dispatch never awaits Number egress.
//!
//! The client asks for its network's number on first need, never at start
//! (#2572): before its first Who-Is, whose answers are about to fill the
//! discovery table, and when it first stores a routed row while the number
//! is unknown, the row that can turn out to be a local device's twin. A
//! client that only names devices by address never asks. After that first
//! ask, each return of the link while the number is still unknown asks once
//! more. Clause 6.4.19 lets any node ask with a local broadcast and has it
//! keep the answer rather than repeat the question, so a known number is
//! never asked for again.
use super::device_events::{self, TableNotices};
use super::*;
use bacnet_network::network_number::{NumberControl, number_is_reply};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::network_number::NetworkNumber;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, watch};

pub(super) const CAPACITY: usize = 256;

/// When the client asks for its network's number: shared by the request
/// methods, which ask before a Who-Is, the dispatch task, which wants an
/// ask when it stores a routed row, and the Number owner, which makes that
/// ask and the ones a link return calls for.
#[derive(Debug, Default)]
pub(super) struct Asking {
    /// The link takes the local Number controls: without them nothing is
    /// asked, and no owner runs.
    numbers: bool,
    /// A question went out. A send that failed doesn't count, so the next
    /// need asks again.
    asked: AtomicBool,
    /// Held across a send, so two needs at once ask once.
    sending: tokio::sync::Mutex<()>,
    /// Wakes the owner to ask for the dispatch task, which never waits on
    /// Number egress.
    wanted: Notify,
}

impl Asking {
    pub(super) fn new(numbers: bool) -> Self {
        Self {
            numbers,
            ..Self::default()
        }
    }

    /// Ask now, unless the link takes no Number controls, the number is
    /// known, or the question went out already. Awaited before a Who-Is,
    /// so the question goes ahead of it; a send that fails is logged and
    /// doesn't hold the Who-Is back.
    pub(super) async fn first_need<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) {
        let needed = || {
            !self.asked.load(Ordering::Acquire) && network.local_network_number().get().is_none()
        };
        if !self.numbers || !needed() {
            return;
        }
        let _one = self.sending.lock().await;
        if needed() && ask(network).await {
            self.asked.store(true, Ordering::Release);
        }
    }

    /// The dispatch task stored a routed row while the number is unknown:
    /// the owner asks, once, if nothing has. Never waits.
    pub(super) fn want(&self) {
        if self.numbers && !self.asked.load(Ordering::Acquire) {
            self.wanted.notify_one();
        }
    }

    /// A caller's own What-Is-Network-Number went out
    /// ([`BACnetClient::what_is_network_number`]): it counts as the
    /// question, so the next need doesn't ask again.
    pub(super) fn asked_by_caller(&self) {
        self.asked.store(true, Ordering::Release);
    }

    /// The owner's ask for a routed row stored while the number is unknown.
    /// It never waits on the send lock, so the owner's controls keep
    /// moving: a busy lock means a question is going out already, and one
    /// that fails is asked again at the next need.
    async fn owner_need<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) {
        let needed = || {
            !self.asked.load(Ordering::Acquire) && network.local_network_number().get().is_none()
        };
        let Ok(_one) = self.sending.try_lock() else {
            return;
        };
        if needed() && ask(network).await {
            self.asked.store(true, Ordering::Release);
        }
    }

    /// The link is back: ask again if a question went out before and the
    /// number is still unknown. A link return never starts the asking, and
    /// the owner doesn't wait for a question already going out.
    async fn again<T: TransportPort + 'static>(&self, network: &NetworkLayer<T>) {
        let needed =
            || self.asked.load(Ordering::Acquire) && network.local_network_number().get().is_none();
        if !needed() {
            return;
        }
        let Ok(_one) = self.sending.try_lock() else {
            return;
        };
        if needed() {
            ask(network).await;
        }
    }
}

/// What the owner keeps beside the layer: the discovery table its learning
/// folds, with that table's subscribers, the asking it shares with the
/// client, and the link's returns, when the transport reports them.
pub(super) struct Owner {
    pub(super) table: Arc<SyncMutex<DeviceTable>>,
    pub(super) device_tx: broadcast::Sender<DeviceEvent>,
    pub(super) collision_tx: broadcast::Sender<DeviceCollisionEvent>,
    pub(super) asking: Arc<Asking>,
    pub(super) link_returns: Option<watch::Receiver<u64>>,
}

/// Ask this network's number with a local broadcast What-Is-Network-Number,
/// which carries no DNET and is never routed (Clause 6.4.19). Whether it
/// went: a send that fails is logged and left for the next need.
async fn ask<T: TransportPort + 'static>(network: &NetworkLayer<T>) -> bool {
    let ask = network.send_network_message(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[], None);
    match ask.await {
        Ok(()) => true,
        Err(error) => {
            debug!(%error, "What-Is-Network-Number broadcast failed");
            false
        }
    }
}

/// Wait for the link to come back; forever once the transport has no more
/// returns to report, or never had any.
async fn link_returned(returns: &mut Option<watch::Receiver<u64>>) {
    let back = match returns.as_mut() {
        Some(returns) => returns.changed().await.is_ok(),
        None => false,
    };
    if !back {
        *returns = None;
        std::future::pending::<()>().await;
    }
}

pub(super) fn spawn<T: TransportPort + 'static>(
    network: &Arc<NetworkLayer<T>>,
    owner: Owner,
) -> (mpsc::Sender<NumberControl>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(CAPACITY);
    let network = Arc::clone(network);
    let task = tokio::spawn(async move {
        let Owner {
            table,
            device_tx,
            collision_tx,
            asking,
            mut link_returns,
        } = owner;
        let notices: TableNotices<'_> = (&device_tx, &collision_tx);
        let mut state = NetworkNumber::default();
        // Two routers that disagree flip the number on every announcement:
        // the table follows each flip, the log once an interval.
        let changes = WarnThrottle::default();
        loop {
            // No branch holds a lock, and each arm finishes its work, the
            // send included, before the next turn polls any of them.
            let control = tokio::select! {
                control = rx.recv() => match control {
                    Some(control) => control,
                    None => break,
                },
                () = asking.wanted.notified() => {
                    asking.owner_need(&network).await;
                    continue;
                }
                () = link_returned(&mut link_returns) => {
                    asking.again(&network).await;
                    continue;
                }
            };
            match control {
                NumberControl::NumberIs { number, flag } => {
                    let before = state.snapshot().0;
                    state.observe(number, flag);
                    let after = state.snapshot().0;
                    if after != before {
                        if before != 0 {
                            warn_throttled!(
                                changes,
                                previous = before,
                                number = after,
                                "this network's number changed; rows made local under the \
                                 previous number are dropped"
                            );
                        }
                        let this_network = network.local_network_number();
                        device_events::learned_number(&table, this_network, state, notices);
                    }
                }
                NumberControl::WhatIs => {
                    if let Some(npdu) = number_is_reply(state)
                        && let Err(error) = network.transport().send_broadcast(&npdu).await
                    {
                        debug!(%error, "Network-Number-Is broadcast failed");
                    }
                }
            }
        }
    });
    (tx, task)
}

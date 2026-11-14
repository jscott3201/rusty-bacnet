//! One bounded serial owner; raw Reject/APDU dispatch never awaits Number egress.
//!
//! The client asks for its network's number once at start (#2572), before
//! this owner runs, and the owner asks again each time the link comes back
//! while the number is still unknown. Clause 6.4.19 lets any node ask with
//! a local broadcast and has it keep the answer rather than repeat the
//! question, so a known number is never asked for again.
use super::device_events::{self, TableNotices};
use super::*;
use bacnet_network::network_number::{NumberControl, number_is_reply};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::network_number::NetworkNumber;
use tokio::sync::watch;

pub(super) const CAPACITY: usize = 256;

/// What the owner keeps beside the layer: the discovery table its learning
/// folds, with that table's subscribers, and the link's returns, when the
/// transport reports them.
pub(super) struct Owner {
    pub(super) table: Arc<SyncMutex<DeviceTable>>,
    pub(super) device_tx: broadcast::Sender<DeviceEvent>,
    pub(super) collision_tx: broadcast::Sender<DeviceCollisionEvent>,
    pub(super) link_returns: Option<watch::Receiver<u64>>,
}

/// Ask this network's number with a local broadcast What-Is-Network-Number,
/// which carries no DNET and is never routed (Clause 6.4.19). A send that
/// fails is logged and left: the answer, or the next trigger, may yet come.
pub(super) async fn ask<T: TransportPort + 'static>(network: &NetworkLayer<T>) {
    let ask = network.send_network_message(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[], None);
    if let Err(error) = ask.await {
        debug!(%error, "What-Is-Network-Number broadcast failed");
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
            mut link_returns,
        } = owner;
        let notices: TableNotices<'_> = (&device_tx, &collision_tx);
        let mut state = NetworkNumber::default();
        loop {
            // Neither branch holds a lock, and each arm finishes its work,
            // the send included, before the next turn polls either.
            let control = tokio::select! {
                control = rx.recv() => match control {
                    Some(control) => control,
                    None => break,
                },
                () = link_returned(&mut link_returns) => {
                    // One ask per return; a known number is kept.
                    if state.snapshot().0 == 0 {
                        ask(&network).await;
                    }
                    continue;
                }
            };
            match control {
                NumberControl::NumberIs { number, flag } => {
                    let before = state.snapshot().0;
                    state.observe(number, flag);
                    if state.snapshot().0 != before {
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

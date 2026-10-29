//! One bounded serial owner; raw Reject/APDU dispatch never awaits Number egress.
use super::*;
use bacnet_network::network_number::{NumberControl, number_is_reply};
use bacnet_types::network_number::NetworkNumber;

pub(super) const CAPACITY: usize = 256;
pub(super) fn spawn<T: TransportPort + 'static>(
    network: &Arc<NetworkLayer<T>>,
) -> (mpsc::Sender<NumberControl>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel(CAPACITY);
    let network = Arc::clone(network);
    let task = tokio::spawn(async move {
        let mut state = NetworkNumber::default();
        while let Some(control) = rx.recv().await {
            match control {
                NumberControl::NumberIs { number, flag } => {
                    state.observe(number, flag);
                    network.local_network_number().publish(state);
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

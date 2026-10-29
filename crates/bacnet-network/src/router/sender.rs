//! Each port's sender owns its transport and carries out queued sends.
//!
//! A typed pre-send length refusal returns reason 4 using the retained
//! origin record. The transport's current advertised limit cannot explain
//! a past refusal: it may have changed again, or exclude frame attributes
//! and direct-peer bounds. Other failures are only logged.

use std::sync::{Arc, Weak};

use bacnet_router_core::PortId;
use bacnet_router_core::router::{RejectOrigin, SendRefusal};
use bacnet_transport::port::{DataAttribute, TransportPort};
use bacnet_types::error::Error;

use super::SendRequest;
use super::actions::{drain, report_backlog};
use super::dispatch::{Outputs, Shared};
use super::send_queue;
use super::send_warning::SendWarnings;

/// Carry out every send until the queue closes. Weak feedback ownership
/// lets dispatch closure release the queues and local consumers normally.
#[cfg(test)]
pub(super) async fn run<T: TransportPort>(
    transport: T,
    port: PortId,
    queue: send_queue::Receiver,
    shared: Arc<Shared>,
    outputs: Weak<Outputs>,
) {
    run_loop(&transport, port, queue, shared, outputs).await;
}

async fn run_loop<T: TransportPort>(
    transport: &T,
    port: PortId,
    mut queue: send_queue::Receiver,
    shared: Arc<Shared>,
    outputs: Weak<Outputs>,
) {
    let warnings = SendWarnings::default();
    while let Some(request) = queue.recv().await {
        // The slot is free before the send starts: a congested port
        // reports its backlog as it drains (#1697).
        report_backlog(&shared, &outputs, port, &queue);
        match request {
            SendRequest::Unicast {
                npdu,
                mac,
                data_attributes,
                reject_origin,
                response,
            } => {
                let done = match response {
                    Some(response) => match response.start() {
                        Some(done) => Some(done),
                        None => continue,
                    },
                    None => None,
                };
                let result = transport
                    .send_unicast_with_data_attributes(&npdu, &mac, &data_attributes)
                    .await;
                if let Err(error) = &result {
                    refused(&shared, &outputs, error, reject_origin, &data_attributes);
                    warnings.unicast_failed(error);
                }
                if let Some(done) = done {
                    let _ = done.send(result);
                }
            }
            SendRequest::Broadcast {
                npdu,
                data_attributes,
                reject_origin,
            } => {
                if let Err(error) = transport
                    .send_broadcast_with_data_attributes(&npdu, &data_attributes)
                    .await
                {
                    refused(&shared, &outputs, &error, reject_origin, &data_attributes);
                    warnings.broadcast_failed(&error);
                }
            }
        }
    }
}

fn refused(
    shared: &Shared,
    outputs: &Weak<Outputs>,
    error: &Error,
    origin: Option<RejectOrigin>,
    attributes: &[DataAttribute],
) {
    if !matches!(error, Error::SendTooLong { .. }) {
        return;
    }
    let outputs = outputs.upgrade();
    let queues = outputs
        .as_ref()
        .map_or(&[][..], |outputs| outputs.send_txs.as_slice());
    let mut router = shared.engine.lock();
    router.refused_later(SendRefusal::TooLong, origin);
    // The engine lock and a feedback owner live only for this synchronous
    // drain. Replies use the same bounded queues and priority/FIFO rules.
    drain(&mut router, attributes, queues, shared.engine.now());
}

#[cfg(test)]
#[path = "sender_ready_tests.rs"]
mod tests;

/// The transport outlives cancellation or panic of its borrowed send loop.
pub(super) fn owned<T: TransportPort>(
    transport: T,
    port: PortId,
    queue: send_queue::Receiver,
    shared: Arc<Shared>,
    outputs: Weak<Outputs>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    network_number: u16,
) -> impl std::future::Future<Output = Vec<super::RouterStopFailure>> + Send {
    use super::shutdown::{RouterStopFailure, TransportOwner, observe};
    let mut owner = TransportOwner {
        transport,
        stopped: false,
    };
    // Construct the fallback owner before spawning, even if the task is never polled.
    async move {
        let mut failures = Vec::new();
        let send_result = tokio::select! {
            biased;
            _ = async {
                if !*stop.borrow_and_update() { let _ = stop.changed().await; }
            } => Ok(()),
            result = observe(run_loop(&owner.transport, port, queue, shared, outputs)) => result,
        };
        if let Err(detail) = send_result {
            failures.push(RouterStopFailure::Worker {
                network_number: Some(network_number),
                worker: "sender",
                detail,
            });
        }
        match observe(owner.transport.stop()).await {
            Ok(Ok(())) => owner.stopped = true,
            Ok(Err(error)) => failures.push(RouterStopFailure::Transport {
                network_number,
                error,
            }),
            Err(detail) => failures.push(RouterStopFailure::Worker {
                network_number: Some(network_number),
                worker: "transport stop",
                detail,
            }),
        }
        // Keep the guard captured as one owner, then run its fallback only
        // after the cleanup outcome has been recorded.
        drop(owner);
        failures
    }
}

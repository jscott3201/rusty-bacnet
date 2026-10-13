//! Carrying out what the engine asks to send (#1679).
//!
//! After every call into the engine, [`drain`] polls it until it is idle
//! and puts each send on its port's queue with `try_send`, under the
//! engine's lock: nothing here waits. A send an arrival prompted carries
//! that arrival's data attributes (RB-03); the startup announcements carry
//! none. A full or closed queue drops the send, logs the warning its kind
//! logged before the engine, and tells the engine, which ends a chunked
//! Initialize-Routing-Table-Ack answer there.

use bacnet_router_core::message::MessageType;
use bacnet_router_core::router::{Action, Cause, Dest, Kind, SendRefusal};
use bacnet_transport::port::DataAttribute;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tracing::warn;

use super::engine::HostedRouter;
use super::SendRequest;

/// Send what `router` asks for on the port queues `send_txs`, by port
/// index, until it is idle. `attributes` are the data attributes of the
/// arrival that prompted the sends, if any.
pub(super) fn drain(
    router: &mut HostedRouter,
    attributes: &[DataAttribute],
    send_txs: &[mpsc::Sender<SendRequest>],
) {
    // A fan-out polls the same NPDU once per port: share its buffer.
    let mut last: Option<Bytes> = None;
    loop {
        let Action::Send(send) = router.poll_action() else {
            return;
        };
        let npdu = match last.take() {
            Some(previous) if previous.as_ref() == send.npdu => previous,
            _ => Bytes::copy_from_slice(send.npdu),
        };
        last = Some(npdu.clone());
        let data_attributes = match send.cause {
            Cause::Ingress => attributes.to_vec(),
            Cause::Local => Vec::new(),
        };
        let request = match send.to {
            Dest::Unicast(mac) => SendRequest::Unicast {
                npdu,
                mac: MacAddr::from_slice(mac),
                data_attributes,
            },
            Dest::Broadcast => SendRequest::Broadcast {
                npdu,
                data_attributes,
            },
        };
        let (port, kind) = (usize::from(send.port.0), send.kind);
        let result = match send_txs.get(port) {
            Some(tx) => tx.try_send(request),
            None => Err(TrySendError::Closed(request)),
        };
        if let Err(error) = result {
            refused(kind, &error);
            router.refused(SendRefusal::Full);
        }
    }
}

/// Log a send its queue refused, as the router logged it before the engine.
/// Relays and searches were dropped without a word, and still are.
fn refused(kind: Kind, error: &TrySendError<SendRequest>) {
    match kind {
        Kind::Forward => warn!(%error, "Router dropped message: output channel full"),
        Kind::Global => warn!(%error, "Router dropped broadcast: output channel full"),
        Kind::Reject(_) => warn!(%error, "Router dropped reject message: output channel full"),
        Kind::Answer(MessageType::I_AM_ROUTER_TO_NETWORK) => {
            warn!(%error, "Router dropped I-Am-Router response: output channel full")
        }
        Kind::Answer(MessageType::INITIALIZE_ROUTING_TABLE_ACK) => {
            warn!(%error, "Router dropped Init-Routing-Table-ACK: output channel full")
        }
        Kind::Answer(_) => warn!(%error, "Router dropped Network-Number-Is: output channel full"),
        Kind::Announce(MessageType::NETWORK_NUMBER_IS) => {
            warn!(%error, "Router dropped Network-Number-Is announcement: output channel full")
        }
        Kind::Announce(_) => {
            warn!(%error, "Router dropped I-Am-Router announcement: output channel full")
        }
        Kind::Relay(_) | Kind::Solicit => {}
        // A kind a later engine adds.
        _ => warn!(%error, "Router dropped a message: output channel full"),
    }
}

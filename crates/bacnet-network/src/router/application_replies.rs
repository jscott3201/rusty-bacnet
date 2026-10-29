//! Answers from the router's application port to the confirmed requests it
//! received, sent back through each request's saved reply plan (#1808).
//!
//! The router hands its application a [`RouterResponse`] with every
//! Confirmed-Request it delivers: the endpoint the request was addressed to
//! and the path it came by. A stack on the
//! [application port](super::RouterApplicationPort) answers by writing a
//! routed NPDU to the requester, which on its own would be originated
//! afresh, from the egress port's address and by today's route. So the port
//! keeps each request's plan for a while, keyed by the requester (SNET and
//! SADR as the stack sees them) and the invoke ID, and sends the answer
//! through it: from the endpoint that was asked, back along the path the
//! request took, as the server does for its own replies.
//!
//! A ConfirmedEventNotification goes to both stacks on
//! [split ports](super::RouterApplicationPorts), and the server's answer is
//! the one sent: the client's port keeps a silenced plan for it, which
//! swallows the client's answer. When the server's port can't take the
//! notification, the client's port keeps the real plan instead.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use bacnet_npdu::{Address, NetworkPriority};
use bacnet_transport::port::DirectResponseScope;
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::time::{Duration, Instant};

use super::RouterResponse;

/// The most requests whose plans are kept; a new one past it pushes out
/// the oldest.
pub(super) const CAPACITY: usize = 64;

/// How long a plan is kept: far longer than a stack takes to answer.
pub(super) const LIFETIME: Duration = Duration::from_secs(30);

// `RouterApplicationPort`'s docs and docs/rust-api.md name both numbers:
// change them together.
const _: () = assert!(CAPACITY == 64 && LIFETIME.as_secs() == 30);

/// One request's saved plan.
struct Saved {
    network: u16,
    mac: MacAddr,
    invoke_id: u8,
    /// `None` when another stack answers it.
    response: Option<RouterResponse>,
    saved_at: Instant,
}

/// What becomes of an answer to a request the port received.
#[derive(Debug)]
pub(super) enum Plan {
    /// Send it through the request's saved plan.
    Send(RouterResponse),
    /// Drop it: the other stack answers the request.
    Silenced,
}

/// The saved plans of the requests the port received, and the owner of the
/// answers sent through them: dropping the port retires those not yet sent.
#[derive(Default)]
pub(super) struct Replies {
    scope: DirectResponseScope,
    saved: Mutex<VecDeque<Saved>>,
}

impl Replies {
    fn saved(&self) -> std::sync::MutexGuard<'_, VecDeque<Saved>> {
        self.saved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keep `response`, the plan of the Confirmed-Request `apdu` from
    /// `network` and `mac`. A repeat of the request replaces its plan.
    pub(super) fn save(&self, network: u16, mac: &[u8], apdu: &[u8], response: RouterResponse) {
        self.keep(network, mac, apdu, Some(response));
    }

    /// Keep a silenced plan for the Confirmed-Request `apdu` from `network`
    /// and `mac`, which the other stack answers: this port's answer to it
    /// is dropped.
    pub(super) fn silence(&self, network: u16, mac: &[u8], apdu: &[u8]) {
        self.keep(network, mac, apdu, None);
    }

    fn keep(&self, network: u16, mac: &[u8], apdu: &[u8], response: Option<RouterResponse>) {
        // Confirmed-Request: the invoke ID is its third octet.
        let &[first, _, invoke_id, ..] = apdu else {
            return;
        };
        if first >> 4 != 0 {
            return;
        }
        let now = Instant::now();
        let mut saved = self.saved();
        saved.retain(|entry| {
            now.duration_since(entry.saved_at) < LIFETIME
                && !(entry.network == network
                    && entry.mac.as_slice() == mac
                    && entry.invoke_id == invoke_id)
        });
        if saved.len() == CAPACITY {
            saved.pop_front();
        }
        saved.push_back(Saved {
            network,
            mac: MacAddr::from_slice(mac),
            invoke_id,
            response,
            saved_at: now,
        });
    }

    /// The plan for `apdu`, when it answers a request the port received
    /// from `destination`: a SimpleAck, ComplexAck, Error or Reject, or a
    /// Segment-ACK or Abort sent as the server. An answer that ends the
    /// transaction takes the plan with it; a Segment-ACK or a segment with
    /// more to follow leaves it for the rest.
    pub(super) fn plan_for(&self, destination: Address<'_>, apdu: &[u8]) -> Option<Plan> {
        let (&first, &invoke_id) = (apdu.first()?, apdu.get(1)?);
        let as_server = first & 0x01 != 0;
        let ends = match first >> 4 {
            0x2 | 0x5 | 0x6 => true,
            0x3 => first & 0x04 == 0,
            0x4 if as_server => false,
            0x7 if as_server => true,
            _ => return None,
        };
        let now = Instant::now();
        let mut saved = self.saved();
        saved.retain(|entry| now.duration_since(entry.saved_at) < LIFETIME);
        let index = saved.iter().position(|entry| {
            entry.network == destination.network
                && entry.mac.as_slice() == destination.mac
                && entry.invoke_id == invoke_id
        })?;
        let response = if ends {
            saved.remove(index)?.response
        } else {
            // Still going: its lifetime counts from this answer, so a long
            // segmented exchange keeps its plan.
            let entry = &mut saved[index];
            entry.saved_at = now;
            entry.response.clone()
        };
        Some(response.map_or(Plan::Silenced, Plan::Send))
    }

    /// Send `apdu` through `response`'s plan, and wait for the router to
    /// send it.
    pub(super) async fn send(
        &self,
        response: &RouterResponse,
        apdu: &[u8],
        expecting_reply: bool,
        priority: NetworkPriority,
    ) -> Result<(), Error> {
        let npdu = response.plan().npdu(apdu, expecting_reply, priority);
        let mut encoded = BytesMut::zeroed(npdu.encoded_len());
        npdu.encode_into(&mut encoded)
            .map_err(|error| Error::Encoding(error.to_string()))?;
        response
            .send(encoded.freeze(), priority, self.scope.guard(), || {})
            .await
    }

    /// How many plans are kept now.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.saved().len()
    }
}

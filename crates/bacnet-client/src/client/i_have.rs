//! I-Have delivery (Clause 16.9, #960): every decoded I-Have goes to the
//! subscribers of [`BACnetClient::i_have_events`], and
//! [`BACnetClient::who_has_replies`] collects the answers to one Who-Has.

use super::*;
use bacnet_services::who_has::{IHaveRequest, WhoHasObject};
use bacnet_services::who_is::DeviceInstanceRange;
use tokio::sync::broadcast::error::RecvError;

/// I-Have broadcast channel capacity. A subscriber this far behind loses
/// the oldest and is told how many with `RecvError::Lagged`; the dispatch
/// task never waits for a subscriber.
pub const I_HAVE_CHANNEL_CAPACITY: usize = 256;

/// An I-Have this client received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedIHave {
    /// The device, the object it has, and the object's name.
    pub i_have: IHaveRequest,
    /// The link-layer peer it came from: the device, or the router that
    /// passed it on.
    pub source_mac: MacAddr,
    /// SNET/SADR, when a router passed it on from another network.
    pub source_network: Option<NpduAddress>,
}

impl ReceivedIHave {
    /// Whether it answers a Who-Has for `object` in `range`: the same
    /// identifier, or the same name, compared exactly, from a device in
    /// the range (Clause 16.9.2).
    pub fn answers(&self, object: &WhoHasObject, range: Option<DeviceInstanceRange>) -> bool {
        let in_range = range
            .is_none_or(|range| range.contains(self.i_have.device_identifier.instance_number()));
        in_range
            && match object {
                WhoHasObject::Identifier(id) => self.i_have.object_identifier == *id,
                WhoHasObject::Name(name) => self.i_have.object_name == *name,
            }
    }
}

/// Decode an I-Have and hand it to the subscribers. A malformed one is
/// logged through `decode_warning` and dropped; with no subscriber nothing
/// is decoded.
pub(super) fn deliver(
    i_have_tx: &broadcast::Sender<ReceivedIHave>,
    source_mac: &[u8],
    source_network: &Option<NpduAddress>,
    service_request: &[u8],
    decode_warning: &WarnThrottle,
) {
    if i_have_tx.receiver_count() == 0 {
        return;
    }
    match IHaveRequest::decode(service_request) {
        Ok(i_have) => {
            let _ = i_have_tx.send(ReceivedIHave {
                i_have,
                source_mac: MacAddr::from_slice(source_mac),
                source_network: source_network.clone(),
            });
        }
        Err(e) => {
            warn_throttled!(decode_warning, error = %e, "Failed to decode IHave");
        }
    }
}

/// Receive from `rx` until `deadline`, handing each item to `each`. A lag
/// is logged with the count lost, and collection goes on.
pub(super) async fn collect_until<T: Clone>(
    rx: &mut broadcast::Receiver<T>,
    deadline: tokio::time::Instant,
    what: &'static str,
    mut each: impl FnMut(T),
) {
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(item)) => each(item),
            Ok(Err(RecvError::Lagged(missed))) => {
                warn!(missed, what, "collector fell behind; replies were lost");
            }
            Ok(Err(RecvError::Closed)) | Err(_) => return,
        }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get a receiver for every I-Have this client receives, an answer to
    /// its own Who-Has or not (a device may send one at any time). Each call
    /// returns a new independent receiver. A receiver that falls
    /// [`I_HAVE_CHANNEL_CAPACITY`] behind loses the oldest.
    pub fn i_have_events(&self) -> broadcast::Receiver<ReceivedIHave> {
        self.i_have_tx.subscribe()
    }

    /// Send a Who-Has as [`Self::who_has`] does, then collect the I-Haves
    /// that answer it ([`ReceivedIHave::answers`]) for `wait`, one per
    /// device and object, in the order they arrived. No answer is an empty
    /// list, not an error.
    pub async fn who_has_replies(
        &self,
        object: WhoHasObject,
        range: Option<DeviceInstanceRange>,
        wait: Duration,
    ) -> Result<Vec<ReceivedIHave>, Error> {
        // Subscribe first, so no answer can arrive before the receiver.
        let mut rx = self.i_have_events();
        self.who_has(object.clone(), range).await?;
        let deadline = tokio::time::Instant::now() + wait;
        let mut replies: Vec<ReceivedIHave> = Vec::new();
        collect_until(&mut rx, deadline, "I-Have", |reply| {
            let seen = replies.iter().any(|kept| {
                kept.i_have.device_identifier == reply.i_have.device_identifier
                    && kept.i_have.object_identifier == reply.i_have.object_identifier
            });
            if !seen && reply.answers(&object, range) {
                replies.push(reply);
            }
        })
        .await;
        Ok(replies)
    }
}

#[cfg(test)]
#[path = "i_have_tests.rs"]
mod tests;

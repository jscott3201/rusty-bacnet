//! I-Have delivery (Clause 16.9, #960): every decoded I-Have goes to the
//! subscribers of [`BACnetClient::i_have_events`], and
//! [`BACnetClient::who_has_replies`] collects the answers to one Who-Has
//! ([`collect_answers`]).

use std::collections::HashSet;

use super::*;
use bacnet_services::who_has::{IHaveRequest, WhoHasObject};
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::broadcast::error::RecvError;

/// I-Have broadcast channel capacity. A subscriber this far behind loses
/// the oldest and is told how many with `RecvError::Lagged`; the dispatch
/// task never waits for a subscriber.
pub const I_HAVE_CHANNEL_CAPACITY: usize = 256;

/// The most answers one Who-Has collects ([`collect_answers`]). Collection
/// stops there, before its wait is over, and logs a warning, so a list of
/// exactly this many may have been cut short. Each answer holds an object name, and a Who-Has
/// to every network of a large site can draw one from every device.
pub const MAX_I_HAVE_REPLIES: usize = 4096;

/// What makes two answers the same one: the device, the object, and the
/// source (the link peer, and SNET/SADR when a router passed it on).
type AnswerKey = (
    ObjectIdentifier,
    ObjectIdentifier,
    MacAddr,
    Option<NpduAddress>,
);

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
/// is logged and collection goes on. Return the number of reports lost.
pub(super) async fn collect_until<T: Clone>(
    rx: &mut broadcast::Receiver<T>,
    deadline: tokio::time::Instant,
    what: &'static str,
    mut each: impl FnMut(T),
) -> u64 {
    let mut lost = 0u64;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(item)) => each(item),
            Ok(Err(RecvError::Lagged(missed))) => {
                lost = lost.saturating_add(missed);
                warn!(missed, what, "collector fell behind; replies were lost");
            }
            Ok(Err(RecvError::Closed)) | Err(_) => return lost,
        }
    }
}

/// Receive from `rx` until `deadline` the I-Haves that answer a Who-Has
/// for `object` in `range` ([`ReceivedIHave::answers`]) and that `keep`
/// takes: one per device, object and source, so two devices sharing an
/// instance both come back, in the order they came, and at most
/// [`MAX_I_HAVE_REPLIES`]; one `keep` refuses doesn't count toward it.
/// `keep` sees each answer not kept already. A receiver that falls behind
/// loses the oldest, which is logged, and collection goes on.
pub async fn collect_answers(
    rx: &mut broadcast::Receiver<ReceivedIHave>,
    deadline: tokio::time::Instant,
    object: &WhoHasObject,
    range: Option<DeviceInstanceRange>,
    mut keep: impl FnMut(&ReceivedIHave) -> bool,
) -> Vec<ReceivedIHave> {
    let mut seen: HashSet<AnswerKey> = HashSet::new();
    let mut replies: Vec<ReceivedIHave> = Vec::new();
    while replies.len() < MAX_I_HAVE_REPLIES {
        let reply = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(RecvError::Lagged(missed))) => {
                warn!(missed, "I-Have collector fell behind; answers were lost");
                continue;
            }
            Ok(Err(RecvError::Closed)) | Err(_) => return replies,
        };
        let key = (
            reply.i_have.device_identifier,
            reply.i_have.object_identifier,
            reply.source_mac.clone(),
            reply.source_network.clone(),
        );
        if reply.answers(object, range) && !seen.contains(&key) && keep(&reply) {
            seen.insert(key);
            replies.push(reply);
        }
    }
    warn!(
        cap = MAX_I_HAVE_REPLIES,
        "Who-Has answers reached the cap; the rest were not collected"
    );
    replies
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Get a receiver for every I-Have this client receives, an answer to
    /// its own Who-Has or not (a device may send one at any time). Each call
    /// returns a new independent receiver. A receiver that falls
    /// [`I_HAVE_CHANNEL_CAPACITY`] behind loses the oldest.
    pub fn i_have_events(&self) -> broadcast::Receiver<ReceivedIHave> {
        self.i_have_tx.subscribe()
    }

    /// Send a Who-Has to `scope` as [`Self::who_has`] does, then collect
    /// the I-Haves that answer it from that scope for `wait`
    /// ([`collect_answers`], [`Broadcast::answered_from`]): one per device,
    /// object and source address, in the order they arrived, at most
    /// [`MAX_I_HAVE_REPLIES`]. An I-Have from outside the scope, such as
    /// one a router relays from another network after a local Who-Has, is
    /// no answer. No answer is an empty list, not an error.
    pub async fn who_has_replies(
        &self,
        scope: Broadcast,
        object: WhoHasObject,
        range: Option<DeviceInstanceRange>,
        wait: Duration,
    ) -> Result<Vec<ReceivedIHave>, Error> {
        // Subscribe first, so no answer can arrive before the receiver.
        let mut rx = self.i_have_events();
        self.who_has(scope, object.clone(), range).await?;
        let deadline = tokio::time::Instant::now() + wait;
        let from_scope = |reply: &ReceivedIHave| {
            let network = reply.source_network.as_ref().map(|from| from.network);
            scope.answered_from(network, &self.local_networks())
        };
        Ok(collect_answers(&mut rx, deadline, &object, range, from_scope).await)
    }
}

#[cfg(test)]
#[path = "i_have_tests.rs"]
mod tests;

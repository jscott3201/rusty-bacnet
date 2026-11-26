//! Bounded next-hop learning, separate from confirmed transaction ownership.

use std::collections::HashMap;

use bacnet_types::MacAddr;

/// A full cache leaves additional networks using routed local broadcast.
const MAX_LEARNED_ROUTERS: usize = 64;

/// Router MACs learned from admitted terminal responses for remote networks.
/// The notification owner decides admission before this cache is updated.
///
/// An answer teaches a network's router only while none is known (#2831).
/// A request sent through the known router is completed only by an answer
/// through it, so a later answer can teach something else only when it
/// answered a retry sent with the link's broadcast address, and a retry is
/// sent only after the known router stayed silent, which forgets it
/// ([`Self::forget_router`]). So a station that answers for a device
/// cannot take the place of a router that still carries its answers;
/// Clause 6.5.3 leaves which of its ways of learning a router a node uses
/// to the node.
pub(super) struct LearnedRouterCache {
    routers: HashMap<u16, MacAddr>,
}

impl LearnedRouterCache {
    pub(super) fn new() -> Self {
        Self {
            routers: HashMap::new(),
        }
    }

    /// Whether an admitted answer may teach its network's router: the one
    /// rule for learning from answers (#2831). Only an answer to an attempt
    /// sent with the link's broadcast address may, since no station was
    /// chosen for that one; an answer to an attempt sent through a station
    /// only shows that station carried it, and that station may be one
    /// that had the server send it a routed request itself.
    pub(super) fn answer_teaches(by_broadcast: bool) -> bool {
        by_broadcast
    }

    /// Learn `router` for `network`, while the cache knows none for it and
    /// has room for another network.
    pub(super) fn learn_router(&mut self, network: u16, router: &MacAddr) {
        if router.is_empty()
            || self.routers.len() >= MAX_LEARNED_ROUTERS
            || self.routers.contains_key(&network)
        {
            return;
        }
        self.routers.insert(network, router.clone());
    }

    /// Forget `network`'s router, if it is still `silent`: a request sent
    /// through it drew no answer, so the next one goes by local broadcast
    /// and its answer teaches the router again.
    pub(super) fn forget_router(&mut self, network: u16, silent: &MacAddr) {
        if self.routers.get(&network) == Some(silent) {
            self.routers.remove(&network);
        }
    }

    pub(super) fn cached_router(&self, network: u16) -> Option<MacAddr> {
        self.routers.get(&network).cloned()
    }
}

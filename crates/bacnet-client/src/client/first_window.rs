//! The first window of a segmented request (#2742).
//!
//! The client sends segment zero of a segmented request alone, carrying the
//! window it proposes, and takes the window for the rest from the answer to
//! it (Clause 5.4.4.1). Some receivers never answer segment zero by itself
//! when the proposed window is above one: they wait for a whole window. Such
//! a peer would hold up every request by a segment timeout. So the first
//! time a peer leaves segment zero, sent alone, unanswered for a whole
//! segment timeout, the client remembers it, and from then on sends it the
//! whole first window at once: what such a peer waits for, and what a
//! conforming peer takes too (it discards what it already has).
//!
//! The request that taught it goes on as before: it sends segment zero
//! again, and goes on if the peer answers that; a peer that drops the
//! repeat as well fails that request, and its next one works.
//!
//! The table is bounded ([`PEERS`], the oldest out first) and a peer leaves
//! it only that way, or when the client ends. It sits beside the discovery
//! table, which every copy of the client shares. Only a segmented request
//! proposing a window above one reads or writes it, under its own lock,
//! which no await holds.

use super::*;
use std::collections::VecDeque;

/// Peers remembered at once.
pub(crate) const PEERS: usize = 256;

/// Peers that get a segmented request's whole first window at once, keyed
/// as their transactions are (a routed peer apart from its router).
#[derive(Debug, Default)]
pub(crate) struct WholeFirstWindow {
    peers: VecDeque<MacAddr>,
}

impl WholeFirstWindow {
    pub(crate) fn contains(&self, peer: &[u8]) -> bool {
        self.peers.iter().any(|known| known.as_slice() == peer)
    }

    /// Remember `peer`; past [`PEERS`] the oldest goes.
    pub(crate) fn remember(&mut self, peer: &[u8]) {
        if self.contains(peer) {
            return;
        }
        if self.peers.len() == PEERS {
            self.peers.pop_front();
        }
        self.peers.push_back(MacAddr::from_slice(peer));
    }

    /// Peers remembered.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.peers.len()
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// The first window of a segmented request to the peer keyed `tsm_mac`,
    /// and whether it is segment zero alone, sent to learn the window. A
    /// window of one reads nothing.
    pub(super) fn first_window(&self, tsm_mac: &[u8]) -> (usize, bool) {
        let proposed = usize::from(self.config.proposed_window_size.max(1));
        if proposed == 1 {
            return (1, false);
        }
        match &*self.device_table.whole_first_window.lock() {
            Some(peers) if peers.contains(tsm_mac) => (proposed, false),
            _ => (1, true),
        }
    }

    /// A segment timeout ran out on the first window; when that was
    /// segment zero sent alone (`alone`), remember the peer.
    pub(super) fn segment_zero_unanswered(&self, alone: bool, tsm_mac: &[u8]) {
        if !alone {
            return;
        }
        self.device_table
            .whole_first_window
            .lock()
            .get_or_insert_with(Box::default)
            .remember(tsm_mac);
        debug!("A peer that waits for a whole first window: sending it one from now on");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_peer_goes_first_past_the_cap() {
        let mut peers = WholeFirstWindow::default();
        for peer in 0..PEERS as u16 {
            peers.remember(&peer.to_be_bytes());
        }
        assert_eq!(peers.len(), PEERS);
        peers.remember(&0u16.to_be_bytes());
        assert_eq!(peers.len(), PEERS, "a peer known already is kept once");
        peers.remember(&(PEERS as u16).to_be_bytes());
        assert_eq!(peers.len(), PEERS);
        assert!(!peers.contains(&0u16.to_be_bytes()), "the oldest went");
        assert!(peers.contains(&1u16.to_be_bytes()));
        assert!(peers.contains(&(PEERS as u16).to_be_bytes()));
    }
}

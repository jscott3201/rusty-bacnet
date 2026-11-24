//! The first window of a segmented request (#2742).
//!
//! The client sends segment zero of a segmented request alone, carrying the
//! window it proposes, and takes the window for the rest from the answer to
//! it (Clause 5.4.4.1), never more than it proposed and at least one. Some
//! receivers never answer segment zero by itself when the proposed window is
//! above one: they wait for a whole window. Such a peer would hold up every
//! request by a segment timeout. So the first time a peer leaves segment
//! zero, sent alone, unanswered for a whole segment timeout, the client
//! remembers it, and from then on sends it the whole first window at once:
//! what such a peer waits for, and what a conforming peer takes too.
//!
//! The request that taught it goes on as before: it sends segment zero
//! again, and goes on if the peer answers that; a peer that drops the
//! repeat as well fails that request, and its next one works.
//!
//! A conforming peer can be remembered too: one that lost a segment, was
//! offline, or answered slower than the segment timeout. Sent a whole first
//! window, it acknowledges segment zero before the segment timeout runs
//! out, which a peer of the other kind never does (it acknowledges the
//! window's end, or answers a repeat negatively), and that acknowledgement
//! forgets it, so its next request sends segment zero alone again. One
//! that comes only after the timeout forgets nothing: a peer that always
//! answers that late would otherwise be forgotten and learned again on
//! every other request.
//!
//! The table is bounded ([`PEERS`], the oldest out first); a peer leaves it
//! that way, by that acknowledgement, or when the client ends. It sits
//! beside the discovery table, which every copy of the client shares. Only a
//! segmented request proposing a window above one reads or writes it, under
//! its own lock, which no await holds; the acknowledgement takes the lock
//! only for a peer sent a whole first window.

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

    /// Forget `peer`.
    pub(crate) fn forget(&mut self, peer: &[u8]) {
        self.peers.retain(|known| known.as_slice() != peer);
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

/// How a segmented request's first window goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FirstWindow {
    /// A window of one: segment zero alone, as always.
    One,
    /// Segment zero alone, to learn the window from its answer.
    Alone,
    /// The whole first window, to a peer remembered for it.
    Whole,
    /// The whole first window, whose segment timeout ran out before any
    /// answer.
    Late,
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// The first window of a segmented request to the peer keyed `tsm_mac`,
    /// and how it goes. A window of one reads nothing.
    pub(super) fn first_window(&self, tsm_mac: &[u8]) -> (usize, FirstWindow) {
        let proposed = usize::from(self.config.proposed_window_size.max(1));
        if proposed == 1 {
            return (1, FirstWindow::One);
        }
        match &*self.device_table.whole_first_window.lock() {
            Some(peers) if peers.contains(tsm_mac) => (proposed, FirstWindow::Whole),
            _ => (1, FirstWindow::Alone),
        }
    }

    /// A segment timeout ran out on the window starting at `window_start`;
    /// when that was segment zero sent alone, remember the peer. Returns how
    /// the first window now stands: a whole one goes late.
    pub(super) fn segment_zero_unanswered(
        &self,
        first: FirstWindow,
        window_start: usize,
        tsm_mac: &[u8],
    ) -> FirstWindow {
        if window_start != 0 {
            return first;
        }
        match first {
            FirstWindow::Alone => {}
            FirstWindow::Whole => return FirstWindow::Late,
            FirstWindow::One | FirstWindow::Late => return first,
        }
        self.device_table
            .whole_first_window
            .lock()
            .get_or_insert_with(Box::default)
            .remember(tsm_mac);
        debug!("A peer that waits for a whole first window: sending it one from now on");
        first
    }

    /// The window `ack` grants, from one to the window proposed. A positive
    /// acknowledgement of segment zero from a peer sent a whole first window
    /// shows it answers segment zero at once: it is forgotten.
    pub(super) fn granted_window(
        &self,
        first: FirstWindow,
        window_start: usize,
        ack: &SegmentAckPdu,
        tsm_mac: &[u8],
    ) -> usize {
        if first == FirstWindow::Whole && window_start == 0 {
            self.forget_on(ack, tsm_mac);
        }
        let proposed = usize::from(self.config.proposed_window_size.max(1));
        usize::from(ack.actual_window_size).clamp(1, proposed)
    }

    /// The transfer is over. A peer sent a whole first window may have
    /// answered the request before its acknowledgement of segment zero was
    /// read: read what is left, and forget the peer for that one too.
    pub(super) fn segment_zero_answered(
        &self,
        first: FirstWindow,
        acks: &mut tokio::sync::mpsc::Receiver<SegmentAckPdu>,
        tsm_mac: &[u8],
    ) {
        if first != FirstWindow::Whole {
            return;
        }
        while let Ok(ack) = acks.try_recv() {
            self.forget_on(&ack, tsm_mac);
        }
    }

    fn forget_on(&self, ack: &SegmentAckPdu, tsm_mac: &[u8]) {
        if !ack.negative_ack
            && ack.sequence_number == 0
            && let Some(peers) = self.device_table.whole_first_window.lock().as_mut()
        {
            peers.forget(tsm_mac);
        }
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
        peers.forget(&1u16.to_be_bytes());
        assert!(!peers.contains(&1u16.to_be_bytes()));
        assert_eq!(peers.len(), PEERS - 1);
    }
}

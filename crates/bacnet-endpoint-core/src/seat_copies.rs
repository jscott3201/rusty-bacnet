//! The line from a node's server to its client for a request that comes in
//! segments (#2743), and the octets its copies may hold on the client seat.

use super::*;

/// The octets the client seat may hold at once in copies of requests the
/// server reassembled (#2743): 4 MiB, the reassembly budget
/// ([`MAX_SAVED_BYTES`](crate::segmented_request::MAX_SAVED_BYTES)), which
/// the server gives back as each request completes. A copy holds its share
/// from the moment room is found for it until the client's dispatch has
/// decoded it and let go of its octets; what the client publishes to its
/// subscribers is decoded, and holds none. A copy past the bound is dropped
/// and counted with the seat's other drops; the server's answer doesn't
/// depend on it. Copies of requests that come whole are not counted here:
/// the seat's slots bound them, and one is never longer than an APDU.
pub const MAX_REASSEMBLED_COPY_OCTETS: usize = crate::segmented_request::MAX_SAVED_BYTES;

/// The line from a node's server to its client for a request that comes in
/// segments (#2743). Ingress hands the client a copy of a
/// ConfirmedEventNotification that comes whole, but it can't hand over one
/// that comes in segments: the server alone reassembles it. Once the server
/// has it whole, it hands it on through this, and the client hears it as it
/// hears one that came whole. It holds the node's seats weakly: once the
/// node is gone it hands nothing on.
#[doc(hidden)]
#[derive(Clone)]
pub struct ClientCopies(pub(super) Weak<SeatSlot>);

impl ClientCopies {
    /// Offer the node's client a copy of `request`, which the server
    /// reassembled from segments, the last of them in `last`. The client
    /// gets it when it would get a copy of the request whole (a
    /// ConfirmedEventNotification, beside the server role), as one APDU
    /// with no reply authority, in that copy's tier of its queue and within
    /// [`MAX_REASSEMBLED_COPY_OCTETS`]. It never waits: a seat with no room
    /// drops and counts it, as it does any copy. A request of any other
    /// service returns at once, without taking the seats' lock.
    ///
    /// Called only from the server's branch for a completed segmented
    /// request, which is cold: `#[cold]` keeps it out of the plain path's
    /// layout, `#[inline]` lets that branch take it where it is compiled.
    #[cold]
    #[inline]
    pub fn reassembled(
        &self,
        last: &ReceivedApdu,
        request: &bacnet_encoding::apdu::ConfirmedRequest,
    ) {
        if !crate::endpoint_ingress::copied_when_reassembled(request) {
            return;
        }
        if let Some(seats) = self.0.upgrade() {
            crate::endpoint_ingress::share_reassembled(&seats, last, request);
        }
    }
}

impl std::fmt::Debug for ClientCopies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientCopies").finish_non_exhaustive()
    }
}

/// One reassembled request's copy's share of
/// [`MAX_REASSEMBLED_COPY_OCTETS`], given back when it drops: with the
/// copy's octets, once the client has let go of every view of them.
pub(crate) struct CopyBudget {
    held: Arc<AtomicUsize>,
    octets: usize,
}

impl Drop for CopyBudget {
    fn drop(&mut self) {
        self.held.fetch_sub(self.octets, Ordering::Relaxed);
    }
}

/// Room found on one client seat for a reassembled request's copy: that
/// seat's queue, and the copy's share of [`MAX_REASSEMBLED_COPY_OCTETS`].
pub(crate) struct ReservedCopy {
    /// The queue of the seat the room was found on, and the only one the
    /// copy may go to.
    pub(crate) seat: mpsc::Sender<ReceivedApdu>,
    pub(crate) budget: CopyBudget,
}

impl SeatSlot {
    /// Room on the client seat for a reassembled request's copy of
    /// `octets`: a slot in a confirmed request's tier, as a copy of one
    /// that came whole takes, and `octets` within
    /// [`MAX_REASSEMBLED_COPY_OCTETS`] of the copies it holds. Taken before
    /// the copy is made; without room nothing is made, and the drop is
    /// counted here.
    pub(crate) fn reserve_reassembled(
        &self,
        client: &Attached,
        octets: usize,
    ) -> Option<ReservedCopy> {
        if !self.has_room(client, Some(0)) {
            return None;
        }
        let held = &self.reassembled_octets;
        let reserved = held.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(octets)
                .filter(|total| *total <= MAX_REASSEMBLED_COPY_OCTETS)
        });
        if reserved.is_err() {
            add(&self.counts[client.role as usize].full_drops);
            return None;
        }
        Some(ReservedCopy {
            seat: client.apdus.clone(),
            budget: CopyBudget {
                held: Arc::clone(held),
                octets,
            },
        })
    }

    /// Hand `copy` to `seat`, the client seat its room was found on, and to
    /// no other: a client made since on a new seat gets nothing its own
    /// checks didn't admit. No lock is taken. A seat that filled in between,
    /// or whose client is gone, drops it, and the drop is counted.
    pub(crate) fn offer_reassembled(&self, seat: &mpsc::Sender<ReceivedApdu>, copy: ReceivedApdu) {
        let counts = &self.counts[Role::Client as usize];
        match seat.try_send(copy) {
            Ok(()) => add(&counts.apdus),
            Err(_) => add(&counts.full_drops),
        }
    }

    /// The octets reassembled requests' copies hold on the client seat now.
    #[cfg(test)]
    pub(crate) fn reassembled_octets(&self) -> usize {
        self.reassembled_octets.load(Ordering::Relaxed)
    }
}

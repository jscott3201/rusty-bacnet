//! The station a routed lease's request went through, and the check that a
//! routed answer comes back through it (#2831).
//!
//! A routed answer's SNET and SADR are written by whoever sends the frame;
//! the link address it arrives from is the part a station on most links
//! can't choose. So for a lease whose owner records where each attempt went
//! ([`OutboundTransactionCoordinator::route_through`]), an answer naming the
//! lease's peer counts only from that station.

use super::*;

/// Where a routed lease's current attempt went.
#[derive(Clone, Debug)]
pub(super) struct NextHop {
    /// The lease it was recorded for: a later lease of the same invoke ID
    /// has a generation of its own, so this entry no longer applies to it.
    generation: u64,
    /// The router's link address, or `None` for an attempt sent with the
    /// link's broadcast address, which any router may carry back.
    station: Option<MacAddr>,
}

impl OutboundTransactionCoordinator {
    /// Record where the current attempt of `token`'s request goes, before
    /// it is sent (#2831): `Some(station)` for a request sent through that
    /// router, which only an answer arriving from it then completes
    /// ([`Self::admit_from_source`]); `None` for one sent with the link's
    /// broadcast address, which Clause 6.5.3 allows while the router is
    /// unknown and whose answer any router may carry. Each attempt records
    /// its own, replacing the one before.
    ///
    /// A lease that records nothing has its routed answers matched by peer
    /// alone: an owner that checks the path itself, as the client does,
    /// records none. A token that is no longer active records nothing.
    pub fn route_through(
        &self,
        token: LeaseToken,
        station: Option<&[u8]>,
    ) -> Result<(), CoordinatorError> {
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::StatePoisoned)?;
        let CoordinatorState {
            slots, next_hops, ..
        } = &mut *state;
        let active = slots[usize::from(token.invoke_id)]
            .as_ref()
            .is_some_and(|active| active.generation == token.generation);
        if active {
            // Records of leases since ended go here, not on release, which
            // every client request takes: at most one per invoke ID stays.
            next_hops.retain(|invoke_id, hop| {
                slots[usize::from(*invoke_id)]
                    .as_ref()
                    .is_some_and(|lease| lease.generation == hop.generation)
            });
            next_hops.insert(
                token.invoke_id,
                NextHop {
                    generation: token.generation,
                    station: station.map(MacAddr::from_slice),
                },
            );
        }
        Ok(())
    }

    /// Routed answers refused since the coordinator was made because they
    /// came through another station than their lease's attempt went to
    /// ([`Self::route_through`], #2831). Saturates.
    pub fn off_path_answers(&self) -> Result<u64, CoordinatorError> {
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        self.state
            .lock()
            .map(|state| state.off_path_answers)
            .map_err(|_| CoordinatorError::StatePoisoned)
    }

    /// [`Self::admit`] for a routed answer that arrived from link station
    /// `from`: after the peer, the station the lease's current attempt went
    /// through is checked, under the same lock, before anything is claimed.
    /// Kept apart from [`Self::admit`], which every client answer takes, so
    /// that path is not changed by this check.
    pub(super) fn admit_through(
        &self,
        peer: &CanonicalPeer,
        from: &[u8],
        apdu: &Apdu,
    ) -> Result<AdmissionOutcome, CoordinatorError> {
        let Some(invoke_id) = response_invoke_id(apdu) else {
            return Ok(AdmissionOutcome::NotOutbound);
        };
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::StatePoisoned)?;
        let CoordinatorState {
            slots,
            next_hops,
            off_path_answers,
            ..
        } = &mut *state;
        let Some(active) = slots[usize::from(invoke_id)].as_mut() else {
            return Ok(AdmissionOutcome::UnknownInvokeId);
        };
        if active.metadata.peer != *peer {
            return Ok(AdmissionOutcome::PeerMismatch);
        }
        let off_path = next_hops.get(&invoke_id).is_some_and(|hop| {
            hop.generation == active.generation
                && hop
                    .station
                    .as_ref()
                    .is_some_and(|station| station.as_slice() != from)
        });
        if off_path {
            *off_path_answers = off_path_answers.saturating_add(1);
            return Ok(AdmissionOutcome::PeerMismatch);
        }
        Ok(claim(active, invoke_id, apdu))
    }

    /// Whether a routed answer arriving from link station `from` with
    /// `invoke_id` keeps to the path the lease of that invoke ID recorded
    /// for its current attempt ([`Self::route_through`]): false only for a
    /// lease whose attempt went through another station, which
    /// [`Self::admit_from_source`] would refuse. Claims and counts nothing:
    /// for an observer that must see only what admission could take
    /// (#2831).
    #[doc(hidden)]
    pub fn on_path(&self, invoke_id: u8, from: &[u8]) -> bool {
        // lock-poison: fail-closed (a half-done lease could repeat an invoke ID)
        self.state.lock().is_ok_and(|state| {
            let Some(active) = state.slots[usize::from(invoke_id)].as_ref() else {
                return true;
            };
            state.next_hops.get(&invoke_id).is_none_or(|hop| {
                hop.generation != active.generation
                    || hop
                        .station
                        .as_ref()
                        .is_none_or(|station| station.as_slice() == from)
            })
        })
    }

    #[cfg(test)]
    fn recorded_hops(&self) -> usize {
        self.state.lock().unwrap().next_hops.len()
    }
}

#[cfg(test)]
#[path = "coordinator_paths_tests.rs"]
mod tests;

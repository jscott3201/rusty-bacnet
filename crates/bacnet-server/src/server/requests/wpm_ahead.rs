//! Deciding a WritePropertyMultiple's durable attempts ahead of its handler
//! (#1321).
//!
//! An attempt on an object that saves its state first is staged before the
//! handler runs, so its save can run with the database guard dropped
//! (`durable_writes`). Staging a save for an attempt policy then denies would
//! put a state in storage that no client was told about. So with a policy
//! that decides mutations, the request asks it about each such attempt first,
//! in wire order among those attempts and after the checks the handler makes
//! on that attempt, and stages only those before the first it denies, or the
//! first whose value doesn't decode, since the request stops there.
//!
//! The handler, reaching one of those attempts under the guard, takes the
//! decision made here instead of asking again. The policy is asked once
//! per attempt, and the decision counters count an attempt only when the
//! handler reaches its gate, as before. The price: such an attempt is asked
//! about ahead of earlier attempts the objects don't save first, and one the
//! request never reaches, because an earlier attempt failed, was still put
//! to the policy. A request with no such attempt asks nothing ahead and
//! takes no read guard.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_objects::database::ObjectDatabase;
use bacnet_services::wpm::WritePropertyAttempt;
use bacnet_types::enums::PropertyIdentifier;
use tokio::sync::RwLock;

use super::Request;
use crate::handlers;
use crate::mutation::{MutationDecision, MutationPolicy, MutationTarget};
use crate::server::durable_writes::DurableTarget;

/// The decisions made ahead of the handler, in wire order, and how many of
/// them it has taken.
#[derive(Default)]
pub(super) struct DecidedAhead {
    decided: Vec<(WritePropertyAttempt, MutationDecision)>,
    taken: AtomicUsize,
}

impl DecidedAhead {
    /// The attempts of `request` to stage. Without a policy that decides
    /// mutations, the mode answers alone: a permissive one stages every attempt, as
    /// the handler will allow each, and `DenyAll` stages nothing. With one,
    /// each attempt is checked and decided in turn under a read guard, which
    /// is dropped before anything is staged.
    pub(super) async fn targets(
        &mut self,
        request: &Request<'_>,
        db: &Arc<RwLock<ObjectDatabase>>,
    ) -> Vec<DurableTarget> {
        if request.config.mutation_policy == MutationPolicy::DenyAll {
            return Vec::new();
        }
        let attempts = DurableTarget::write_property_multiple(&request.req.service_request);
        if attempts.is_empty() {
            return Vec::new();
        }
        if request.config.mutation_decider().is_none() {
            return DurableTarget::in_object_order(attempts, |_| true);
        }
        let db = db.read().await;
        DurableTarget::in_object_order(attempts, |attempt| self.decide(request, &db, attempt))
    }

    /// Whether `request` may stage `attempt`, deciding it now and keeping
    /// the decision for the handler. An attempt that fails the handler's
    /// checks ends the request there, as policy only judges a checked
    /// attempt; it is neither decided nor staged.
    fn decide(
        &mut self,
        request: &Request<'_>,
        db: &ObjectDatabase,
        attempt: &WritePropertyAttempt,
    ) -> bool {
        let reference = &attempt.reference;
        let checked = db.get(&reference.object_identifier).is_some_and(|object| {
            handlers::gate_and_decode_write(
                object,
                PropertyIdentifier::from_raw(reference.property_identifier),
                reference.property_array_index,
                &attempt.value,
            )
            .is_ok()
        });
        if !checked {
            return false;
        }
        let decision = request.decide(MutationTarget::WritePropertyMultiple(attempt.clone()));
        self.decided.push((attempt.clone(), decision));
        matches!(decision, MutationDecision::Allow)
    }

    /// The decision made ahead for `attempt`, which the handler has
    /// reached, if it is the next one kept. The handler reaches attempts in
    /// wire order and stops at the first that fails, so the decisions are
    /// taken in the order they were made. An attempt with nothing kept is
    /// one that objects don't save first; the handler asks for it as usual.
    pub(super) fn take(&self, attempt: &WritePropertyAttempt) -> Option<MutationDecision> {
        let next = self.taken.load(Ordering::Relaxed);
        let (decided, decision) = self.decided.get(next)?;
        if decided != attempt {
            return None;
        }
        self.taken.store(next + 1, Ordering::Relaxed);
        Some(*decision)
    }
}

//! Deciding a WritePropertyMultiple's attempts before its handler takes the
//! database write lock (#1321, #2032).
//!
//! An installed [`ServerPolicy`](crate::ServerPolicy) is never asked with a
//! database guard held. A policy that blocks (a Python callable waiting on
//! another thread, say) then delays only its own request: other clients'
//! reads and writes and the server's stop keep going.
//!
//! So, with a policy that decides mutations, the request first walks its
//! attempts in wire order under a short read guard and keeps those that pass
//! the checks the handler makes before it asks (the object exists, and the
//! index and value suit the property), up to the first that fails them, since
//! the handler stops there. The guard is dropped, and the policy is asked
//! about each kept attempt in wire order, up to the first it denies. Only
//! then are the attempts objects save first staged (`durable_writes`), those
//! the policy allowed, and the handler takes the write lock.
//!
//! The handler, reaching an attempt, takes the decision made here instead of
//! asking. The policy is asked once per attempt, and the decision counters
//! count an attempt only when the handler reaches its gate, as before. The
//! price: the policy can be asked about an attempt the request never reaches,
//! because an earlier attempt failed later on (its object refused the value,
//! or the new Object_Name is taken).
//!
//! The database can change between the two passes. The handler checks every
//! attempt again under the write lock, so an object deleted in between fails
//! there with its usual error. An attempt the handler reaches with no decision
//! made here (an object created in between, say) is denied: the policy was
//! never asked about it, and asking now would mean asking under the lock.
//! The decision counters count that denial as a policy Deny.
//!
//! The first pass checks each attempt against the database as it was before
//! the request. An indexed write whose validity depends on an earlier attempt
//! of the same request (an index that an earlier attempt brings into range,
//! as with a Staging object's Stage_Names and other state-dependent
//! metadata) fails that check, so the policy isn't asked about it or what
//! follows; if the handler, after the earlier writes, finds it valid, it is
//! denied rather than asked about.
//!
//! With no policy that decides mutations, nothing is asked ahead and no read
//! guard is taken: the mode alone answers each attempt in the handler.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bacnet_objects::database::ObjectDatabase;
use bacnet_services::wpm::{
    WritePropertyAttempt, WritePropertyMultipleCursor, WritePropertyMultipleEvent,
};
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
    /// Whether the policy was asked ahead. Only then does an attempt with
    /// no decision kept fail closed in the handler.
    asked: bool,
    decided: Vec<(WritePropertyAttempt, MutationDecision)>,
    taken: AtomicUsize,
}

impl DecidedAhead {
    /// Decide `request`'s attempts ahead, as the module explains, and return
    /// the attempts to stage. Without a policy that decides mutations, the
    /// mode answers alone: a permissive one stages every attempt objects save
    /// first, as the handler will allow each, and `DenyAll` stages nothing.
    pub(super) async fn targets(
        &mut self,
        request: &Request<'_>,
        db: &Arc<RwLock<ObjectDatabase>>,
    ) -> Vec<DurableTarget> {
        if request.config.mutation_policy == MutationPolicy::DenyAll {
            return Vec::new();
        }
        let service_data = &request.req.service_request;
        if request.config.mutation_decider().is_none() {
            let attempts = DurableTarget::write_property_multiple(service_data);
            return DurableTarget::in_object_order(attempts, |_| true);
        }
        self.asked = true;
        // The guard lives for this statement only: it is gone before the
        // policy is asked.
        let checked = checked_attempts(&*db.read().await, service_data);
        if !checked.is_empty() {
            let targets = checked
                .iter()
                .cloned()
                .map(MutationTarget::WritePropertyMultiple)
                .collect();
            // One trip to the blocking pool for the whole request.
            let decisions = request.decide(targets).await;
            self.decided = checked.into_iter().zip(decisions).collect();
        }
        let attempts = DurableTarget::write_property_multiple(service_data);
        if attempts.is_empty() {
            return Vec::new();
        }
        // Both lists are in wire order, so one pass over the decisions finds
        // each attempt that may save. One the policy never decided ends the
        // staging, as the request stops before it.
        let mut decided = self.decided.iter();
        DurableTarget::in_object_order(attempts, |attempt| {
            decided
                .by_ref()
                .find(|(kept, _)| kept == attempt)
                .is_some_and(|(_, decision)| *decision == MutationDecision::Allow)
        })
    }

    /// The decision for `attempt`, which the handler has reached. The handler
    /// reaches attempts in wire order and stops at the first that fails, so
    /// the decisions are taken in the order they were made. `None` when
    /// nothing was asked ahead: the caller lets the mode decide, asking no
    /// policy. An attempt with no decision kept although the policy was asked
    /// ahead is one the database changed under between the passes, and is
    /// denied.
    pub(super) fn take(&self, attempt: &WritePropertyAttempt) -> Option<MutationDecision> {
        if !self.asked {
            return None;
        }
        let next = self.taken.load(Ordering::Relaxed);
        match self.decided.get(next) {
            Some((decided, decision)) if decided == attempt => {
                self.taken.store(next + 1, Ordering::Relaxed);
                Some(*decision)
            }
            _ => Some(MutationDecision::Deny),
        }
    }
}

/// The attempts of a WritePropertyMultiple request that pass the checks its
/// handler makes before it asks the policy, in wire order, up to the first
/// that fails them or the first undecodable part of the request. Object_Name
/// uniqueness is left to the handler: an earlier attempt in the same request
/// may free the name.
fn checked_attempts(db: &ObjectDatabase, service_data: &[u8]) -> Vec<WritePropertyAttempt> {
    let mut cursor = WritePropertyMultipleCursor::new(service_data);
    let mut checked = Vec::new();
    while let Ok(Some(event)) = cursor.next_event() {
        let WritePropertyMultipleEvent::WriteAttempt(attempt) = event else {
            continue;
        };
        let reference = &attempt.reference;
        let passes = db.get(&reference.object_identifier).is_some_and(|object| {
            handlers::gate_and_decode_write(
                object,
                PropertyIdentifier::from_raw(reference.property_identifier),
                reference.property_array_index,
                &attempt.value,
            )
            .is_ok()
        });
        if !passes {
            break;
        }
        checked.push(attempt);
    }
    checked
}

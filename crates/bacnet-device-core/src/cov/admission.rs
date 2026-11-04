//! Admission: ordinary and Single subscriptions, Multiple references with
//! their shared context, and the caps every new subscription meets.
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::cov::{CovRecipient, MultipleContextKey, SubscriberEndpoint};
use bacnet_time::Instant;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::{CovSubscription, CovSubscriptionKey, CovSubscriptionSnapshot, CovTable};

/// A SubscribeCOVPropertyMultiple admission that did not take every reference
/// (Clause 13.16.2). Processing runs in request order and stops at the first
/// reference that fails; the references before it stay subscribed and are
/// owed their initial notifications, and those after it are never looked at.
#[derive(Debug)]
pub struct MultipleRefusal {
    /// The failure to report.
    pub error: Error,
    /// Position, counted from 0 in request order, of the reference that
    /// failed; `None` when the request failed as a whole, before any
    /// reference was processed.
    pub refused: Option<usize>,
    /// Snapshots of the references accepted before the failure (final
    /// duplicates only, in request order); empty when nothing was kept.
    pub committed: Vec<CovSubscriptionSnapshot>,
}

impl From<Error> for MultipleRefusal {
    /// A failure of the whole request, which keeps nothing.
    fn from(error: Error) -> Self {
        Self {
            error,
            refused: None,
            committed: Vec::new(),
        }
    }
}

/// The cap a new subscription would go past.
#[derive(Clone, Copy)]
enum Cap {
    /// The recipient's own quota.
    Peer,
    /// The table's global or unreserved capacity.
    Table,
}

impl CovTable {
    /// Accept an ordinary or Single insertion or renewal after quota and
    /// generation preflight. Multiple references are rejected before any table
    /// effect: they enter only through [`subscribe_multiple`](Self::subscribe_multiple),
    /// which owns their shared context lifetime and maximum notification delay.
    pub fn subscribe(
        &mut self,
        sub: CovSubscription,
        now: Instant,
    ) -> Result<CovSubscriptionSnapshot, Error> {
        let key = sub.key()?;
        if key.multiple_context().is_some() {
            return Err(Error::Encoding(
                "Multiple references must be admitted through subscribe_multiple".into(),
            ));
        }
        self.expire(now);
        let existing = self.subs.get(&key).cloned();
        self.check_admission(
            &sub.recipient(),
            sub.expires_at.is_none(),
            existing.as_deref(),
            now,
        )?;
        let generation = self.reserve_generations(1)?;
        Ok(self.publish(key, sub, generation, None, None, None))
    }

    /// Accept Multiple references in request order and refresh their exact
    /// context. All identities/options are validated before quota/generation
    /// reservation or refresh. Each proposal that would add a subscription is
    /// then checked against the recipient's quota and the table's capacity in
    /// turn (a renewal, or a repeat of an earlier proposal, adds none), and
    /// the first that does not fit ends the request (Clause 13.16.2, #1059).
    /// The proposals before it are kept as if they had been the whole request
    /// and come back in a [`MultipleRefusal`] naming its position; when it is
    /// the first proposal, or the request fails as a whole (identity or route
    /// mismatch, generation exhaustion), nothing changes (#1058). Final
    /// duplicates among the kept proposals win, once each.
    /// The request's expiry, maximum notification delay and the maximum APDU
    /// its subscriber advertised become the whole context's (last write wins;
    /// a `None` maximum APDU, unknown, keeps the one advertised before). The
    /// delay bounds how long the context's
    /// timestamped changes may stay queued after a notification failed or was
    /// held back; notifications fit the smaller of the subscriber's maximum
    /// APDU and the local one.
    /// The admitted route also replaces the route of every retained reference,
    /// including empty renewals. A changed route fences old snapshots while
    /// preserving unreplaced observations; same-route refresh retains authority,
    /// unless it lists references while the context's confirmed report is
    /// outstanding or holding off, which fences it the same way (#896). A fence
    /// while that report is outstanding, holding off or owed also clears the
    /// observations of the kept untimestamped references it carried, since the
    /// fenced report may have reached the subscriber (#923).
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe_multiple(
        &mut self,
        context: &MultipleContextKey,
        route: &SubscriberEndpoint,
        expires_at: Instant,
        max_notification_delay: u32,
        subscriber_max_apdu: Option<u16>,
        mut subscriptions: Vec<CovSubscription>,
        now: Instant,
    ) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
        context.recipient.validate()?;
        let recipient = CovRecipient::from_endpoint(&route.mac, route.network.as_ref());
        recipient.validate()?;
        if recipient != context.recipient {
            return Err(
                Error::Encoding("Multiple route does not match its recipient".into()).into(),
            );
        }
        for sub in &subscriptions {
            if sub.key()?.multiple_context() != Some(context)
                || sub.expires_at != Some(expires_at)
                || sub.endpoint() != *route
            {
                return Err(Error::Encoding(
                    "Multiple subscription does not match its context/route/expiry".into(),
                )
                .into());
            }
        }
        self.expire(now);
        let overflow = self
            .first_overflow(&recipient, &subscriptions)
            .map(|(position, cap)| (position, self.refuse_new(cap)));
        match overflow {
            Some((0, error)) => {
                return Err(MultipleRefusal {
                    error,
                    refused: Some(0),
                    committed: Vec::new(),
                });
            }
            Some((position, _)) => subscriptions.truncate(position),
            None => {}
        }
        // Final options win exactly once, retaining the final-occurrence request order.
        subscriptions.reverse();
        let mut keys = BTreeSet::new();
        subscriptions.retain(|sub| keys.insert(sub.key().expect("validated identity")));
        subscriptions.reverse();
        let first_generation = self.reserve_generations(subscriptions.len())?;
        // No fallible step follows this point. Unreplaced context references retain generations.
        let (flight, replaced) =
            self.context_flight(context, route, !subscriptions.is_empty(), now);
        // A request whose header is unknown (the raw-data handler) keeps the
        // maximum APDU the context's subscriber advertised before.
        let subscriber_max_apdu = subscriber_max_apdu.or_else(|| {
            self.multiple_context_references(context)
                .next()
                .and_then(|entry| entry.subscriber_max_apdu)
        });
        // The context's references, renewed or published below, expire then.
        self.due_by(Some(expires_at));
        let mut previously_indefinite = 0;
        for entry in self.subs.values_mut() {
            if entry.key.multiple_context() == Some(context) {
                previously_indefinite += usize::from(entry.expires_at.is_none());
                entry.subscription.expires_at = Some(expires_at);
                entry.max_notification_delay = Some(max_notification_delay);
                entry.subscriber_max_apdu = subscriber_max_apdu;
                entry.subscription.subscriber_mac = route.mac.clone();
                entry.subscription.subscriber_network = route.network.clone();
                entry.flight = flight;
            }
        }
        if let Some(count) = self.peer_indefinite_counts.get_mut(&recipient) {
            *count -= previously_indefinite;
            if *count == 0 {
                self.peer_indefinite_counts.remove(&recipient);
            }
        }
        let accepted = subscriptions
            .into_iter()
            .enumerate()
            .map(|(offset, sub)| {
                self.publish(
                    sub.key().expect("validated identity"),
                    sub,
                    first_generation + offset as u64,
                    Some(max_notification_delay),
                    subscriber_max_apdu,
                    Some(flight),
                )
            })
            .collect();
        // The host's timestamped histories follow the context's delay and
        // sizing.
        self.fenced.contexts.push(context.clone());
        if let Some(replaced) = replaced {
            self.fence_context_flight(context, replaced, &keys, now);
        }
        match overflow {
            None => Ok(accepted),
            Some((position, error)) => Err(MultipleRefusal {
                error,
                refused: Some(position),
                committed: accepted,
            }),
        }
    }

    /// The first proposal that would add a subscription past one of
    /// `recipient`'s caps, and the cap it meets. Renewals of live entries and
    /// repeats of an earlier proposal add none.
    fn first_overflow(
        &self,
        recipient: &CovRecipient,
        proposals: &[CovSubscription],
    ) -> Option<(usize, Cap)> {
        let (peer_room, table_room) = self.room(recipient);
        let room = peer_room.min(table_room);
        let mut added = BTreeSet::new();
        for (position, sub) in proposals.iter().enumerate() {
            let key = sub.key().expect("validated identity");
            if self.subs.contains(&key) || added.contains(&key) {
                continue;
            }
            if added.len() == room {
                let cap = self
                    .cap_passed(recipient, room + 1)
                    .expect("one past the room passes a cap");
                return Some((position, cap));
            }
            added.insert(key);
        }
        None
    }

    fn reserve_generations(&mut self, count: usize) -> Result<u64, Error> {
        if count == 0 {
            return Ok(self.generation);
        }
        let last = u64::try_from(count)
            .ok()
            .and_then(|count| self.generation.checked_add(count));
        let Some(last) = last else {
            self.counts.subscriptions_rejected_capacity += 1;
            return Err(Error::protocol(
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
            ));
        };
        let first = self.generation + 1;
        self.generation = last;
        Ok(first)
    }

    fn publish(
        &mut self,
        key: CovSubscriptionKey,
        sub: CovSubscription,
        generation: u64,
        max_notification_delay: Option<u32>,
        subscriber_max_apdu: Option<u16>,
        context_flight: Option<u64>,
    ) -> CovSubscriptionSnapshot {
        let flight = context_flight.unwrap_or_else(|| self.new_flight());
        let snapshot = CovSubscriptionSnapshot {
            key,
            generation,
            table: self.id,
            last_successful_ticket: 0,
            flight,
            subscription: sub.clone(),
            max_notification_delay,
            subscriber_max_apdu,
        };
        let peer = sub.recipient();
        let new_indefinite = sub.expires_at.is_none();
        self.due_by(sub.expires_at);
        match self.subs.insert(snapshot.clone()) {
            Some(old) => {
                if old.flight != snapshot.flight {
                    // The replaced incarnation's outstanding report stops retrying;
                    // the replacement reports for itself (#896).
                    self.fence_flight(old.flight);
                }
                let old_indefinite = old.expires_at.is_none();
                if old_indefinite != new_indefinite {
                    if new_indefinite {
                        *self.peer_indefinite_counts.or_default(peer) += 1;
                    } else if let Some(count) = self.peer_indefinite_counts.get_mut(&peer) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            self.peer_indefinite_counts.remove(&peer);
                        }
                    }
                }
            }
            _ => {
                *self.peer_counts.or_default(peer.clone()) += 1;
                if new_indefinite {
                    *self.peer_indefinite_counts.or_default(peer) += 1;
                }
                self.counts.subscriptions_created += 1;
            }
        }
        snapshot
    }

    /// Check admission for a single subscription request against policy quotas.
    fn check_admission(
        &mut self,
        peer: &CovRecipient,
        is_indefinite: bool,
        existing: Option<&CovSubscription>,
        now: Instant,
    ) -> Result<(), Error> {
        self.expire(now);

        // Indefinite subscriptions are optional (Clause 13.14.1.1.4). A device
        // that takes none answers that the lifetime is outside the range it
        // supports, the code Clause 13.14.1.3.1 gives (#2317).
        if is_indefinite && !self.policy.allow_indefinite_subscriptions {
            self.counts.subscriptions_rejected_indefinite += 1;
            return Err(Error::protocol(
                ErrorClass::SERVICES,
                ErrorCode::VALUE_OUT_OF_RANGE,
            ));
        }

        let (new_count, new_indefinite) = match existing {
            Some(sub) => {
                let was_indefinite = sub.expires_at.is_none();
                let add_indefinite = if is_indefinite && !was_indefinite {
                    1
                } else {
                    0
                };
                (0, add_indefinite)
            }
            None => {
                let add_indefinite = if is_indefinite { 1 } else { 0 };
                (1, add_indefinite)
            }
        };

        self.check_caps(peer, new_count, new_indefinite, now)
    }

    /// Check the per-peer indefinite cap and the subscription caps for one
    /// admission adding `new_count` subscriptions, `new_indefinite` of them
    /// indefinite. [`check_admission`](Self::check_admission), its only
    /// caller, has refused an indefinite one the policy disallows.
    fn check_caps(
        &mut self,
        peer: &CovRecipient,
        new_count: usize,
        new_indefinite: usize,
        now: Instant,
    ) -> Result<(), Error> {
        self.expire(now);

        if new_indefinite > 0 {
            debug_assert!(self.policy.allow_indefinite_subscriptions);
            let current_indefinite = self.peer_indefinite_counts.get(peer).copied().unwrap_or(0);
            if current_indefinite + new_indefinite > self.policy.max_indefinite_per_peer {
                self.counts.subscriptions_rejected_indefinite += 1;
                return Err(Error::protocol(
                    ErrorClass::RESOURCES,
                    ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
                ));
            }
        }

        match self.cap_passed(peer, new_count) {
            Some(cap) => Err(self.refuse_new(cap)),
            None => Ok(()),
        }
    }

    /// How many more subscriptions `peer` may add under its own quota, and
    /// under the table's capacity (the global cap and, for a recipient
    /// without a reservation, the unreserved share).
    fn room(&self, peer: &CovRecipient) -> (usize, usize) {
        let current_peer = self.peer_counts.get(peer).copied().unwrap_or(0);
        let peer_room = self
            .policy
            .max_subscriptions_per_peer
            .saturating_sub(current_peer);
        let mut capacity = self.policy.max_subscriptions_global;
        if !self.policy.is_peer_reserved(peer) {
            capacity = capacity.min(self.policy.effective_unreserved_capacity());
        }
        (peer_room, capacity.saturating_sub(self.subs.len()))
    }

    /// The cap `new_count` more subscriptions of `peer` would pass, its own
    /// quota checked first.
    fn cap_passed(&self, peer: &CovRecipient, new_count: usize) -> Option<Cap> {
        let (peer_room, table_room) = self.room(peer);
        if new_count > peer_room {
            Some(Cap::Peer)
        } else if new_count > table_room {
            Some(Cap::Table)
        } else {
            None
        }
    }

    /// Count a refusal at `cap` and build its error.
    fn refuse_new(&mut self, cap: Cap) -> Error {
        match cap {
            Cap::Peer => self.counts.subscriptions_rejected_quota += 1,
            Cap::Table => self.counts.subscriptions_rejected_capacity += 1,
        }
        Error::protocol(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
        )
    }
}

#[cfg(test)]
mod tests;

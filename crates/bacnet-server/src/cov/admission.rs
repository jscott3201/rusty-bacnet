use super::*;

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

impl CovSubscriptionTable {
    /// Accept an ordinary or Single insertion or renewal after quota and
    /// generation preflight. Multiple references are rejected before any table
    /// effect: they enter only through [`subscribe_multiple`](Self::subscribe_multiple),
    /// which owns their shared context lifetime and maximum notification delay.
    pub fn subscribe(&mut self, sub: CovSubscription) -> Result<CovSubscriptionSnapshot, Error> {
        let key = sub.key()?;
        if key.multiple_context().is_some() {
            return Err(Error::Encoding(
                "Multiple references must be admitted through subscribe_multiple".into(),
            ));
        }
        self.purge_expired();
        let existing = self.subs.get(&key).cloned();
        self.check_admission(
            &sub.recipient(),
            sub.expires_at.is_none(),
            existing.as_deref(),
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
    pub fn subscribe_multiple(
        &mut self,
        context: &MultipleContextKey,
        route: &SubscriberEndpoint,
        expires_at: Instant,
        max_notification_delay: u32,
        subscriber_max_apdu: Option<u16>,
        mut subscriptions: Vec<CovSubscription>,
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
        self.purge_expired();
        let overflow = self
            .first_overflow(&recipient, &subscriptions)
            .map(|(position, cap)| (position, self.refuse_new(cap)));
        match overflow {
            Some((0, error)) => {
                return Err(MultipleRefusal {
                    error,
                    refused: Some(0),
                    committed: Vec::new(),
                })
            }
            Some((position, _)) => subscriptions.truncate(position),
            None => {}
        }
        // Final options win exactly once, retaining the final-occurrence request order.
        subscriptions.reverse();
        let mut keys = std::collections::HashSet::new();
        subscriptions.retain(|sub| keys.insert(sub.key().expect("validated identity")));
        subscriptions.reverse();
        let first_generation = self.reserve_generations(subscriptions.len())?;
        // No fallible step follows this point. Unreplaced context references retain generations.
        let (flight, replaced) = self.context_flight(context, route, !subscriptions.is_empty());
        // A request whose header is unknown (the raw-data handler) keeps the
        // maximum APDU the context's subscriber advertised before.
        let subscriber_max_apdu = subscriber_max_apdu.or_else(|| {
            self.subs
                .values()
                .find(|entry| entry.key.multiple_context() == Some(context))
                .and_then(|entry| entry.subscriber_max_apdu)
        });
        let mut previously_indefinite = 0;
        for entry in self.subs.values_mut() {
            if entry.key.multiple_context() == Some(context) {
                previously_indefinite += usize::from(entry.expires_at.is_none());
                entry.subscription.expires_at = Some(expires_at);
                entry.max_notification_delay = Some(max_notification_delay);
                entry.subscriber_max_apdu = subscriber_max_apdu;
                entry.subscription.subscriber_mac = route.mac.clone();
                entry.subscription.subscriber_network = route.network.clone();
                entry.flight = flight.clone();
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
                    Some(flight.clone()),
                )
            })
            .collect();
        {
            let mut timed = self.timed.lock();
            timed.set_delay(context, max_notification_delay);
            // Lifetime left now, rounded up as notifications report it; an
            // expired one, which no notification reports, sizes as the longest.
            let time_remaining = CovTimeRemaining::at(Some(expires_at), runtime_clock::now())
                .wire_seconds()
                .unwrap_or(u32::MAX);
            timed.set_sizing(context, subscriber_max_apdu, time_remaining);
        }
        if let Some(replaced) = replaced {
            self.fence_context_flight(context, &replaced, &keys);
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
        let mut added = std::collections::HashSet::new();
        for (position, sub) in proposals.iter().enumerate() {
            let key = sub.key().expect("validated identity");
            if self.subs.contains_key(&key) || added.contains(&key) {
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

    /// Test fixture: admit one proposal through its family's production
    /// owner. A Multiple reference is a one-reference `subscribe_multiple`
    /// request (finite expiry required) that refreshes its exact context with
    /// the supplied maximum notification delay.
    #[cfg(test)]
    pub(crate) fn admit_for_test(
        &mut self,
        sub: CovSubscription,
        max_notification_delay: u32,
    ) -> Result<CovSubscriptionSnapshot, Error> {
        let Some(context) = sub.key()?.multiple_context().cloned() else {
            return self.subscribe(sub);
        };
        let expires_at = sub
            .expires_at
            .expect("Multiple contexts always have a finite lifetime");
        let mut accepted = self
            .subscribe_multiple(
                &context,
                &sub.endpoint(),
                expires_at,
                max_notification_delay,
                None,
                vec![sub],
            )
            .map_err(|refusal| refusal.error)?;
        Ok(accepted.remove(0))
    }

    fn reserve_generations(&mut self, count: usize) -> Result<u64, Error> {
        if count == 0 {
            return Ok(self.generation);
        }
        let last = u64::try_from(count)
            .ok()
            .and_then(|count| self.generation.checked_add(count));
        let Some(last) = last else {
            self.counters
                .subscriptions_rejected_capacity
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Protocol {
                class: ErrorClass::RESOURCES.to_raw() as u32,
                code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
            });
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
        context_flight: Option<super::confirmed::FlightMarker>,
    ) -> CovSubscriptionSnapshot {
        let snapshot = CovSubscriptionSnapshot {
            key,
            generation,
            owner: Arc::clone(&self.owner),
            last_successful_ticket: 0,
            flight: context_flight.unwrap_or_default(),
            subscription: sub.clone(),
            max_notification_delay,
            subscriber_max_apdu,
        };
        {
            let mut timed = self.timed.lock();
            let delay = max_notification_delay.unwrap_or_default();
            match (snapshot.key.multiple_context(), sub.timestamped) {
                (Some(_), true) => timed.reset(&snapshot.key, generation, delay),
                (Some(_), false) => timed.reset_untimed(&snapshot.key, generation, delay),
                (None, _) => timed.remove(&snapshot.key),
            }
        }
        let peer = sub.recipient();
        let new_indefinite = sub.expires_at.is_none();
        if let Some(old) = self.subs.insert(snapshot.key.clone(), snapshot.clone()) {
            if !old.flight.same(&snapshot.flight) {
                // The replaced incarnation's outstanding report stops retrying;
                // the replacement reports for itself (#896).
                old.flight.fence();
            }
            let old_indefinite = old.expires_at.is_none();
            if old_indefinite != new_indefinite {
                if new_indefinite {
                    *self.peer_indefinite_counts.entry(peer).or_default() += 1;
                } else if let Some(count) = self.peer_indefinite_counts.get_mut(&peer) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.peer_indefinite_counts.remove(&peer);
                    }
                }
            }
        } else {
            *self.peer_counts.entry(peer.clone()).or_default() += 1;
            if new_indefinite {
                *self.peer_indefinite_counts.entry(peer).or_default() += 1;
            }
            self.counters
                .subscriptions_created
                .fetch_add(1, Ordering::Relaxed);
        }
        self.counters
            .subscriptions_active
            .store(self.subs.len() as u64, Ordering::Relaxed);
        snapshot
    }

    /// Check admission for a single subscription request against policy quotas.
    pub(super) fn check_admission(
        &mut self,
        peer: &CovRecipient,
        is_indefinite: bool,
        existing: Option<&CovSubscription>,
    ) -> Result<(), Error> {
        self.purge_expired();

        if is_indefinite && !self.policy.allow_indefinite_subscriptions {
            self.counters
                .subscriptions_rejected_indefinite
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
            });
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

        self.check_caps(peer, new_count, new_indefinite)
    }

    /// Check the indefinite policy and the subscription caps for one
    /// admission adding `new_count` subscriptions, `new_indefinite` of them
    /// indefinite.
    fn check_caps(
        &mut self,
        peer: &CovRecipient,
        new_count: usize,
        new_indefinite: usize,
    ) -> Result<(), Error> {
        self.purge_expired();

        if new_indefinite > 0 {
            if !self.policy.allow_indefinite_subscriptions {
                self.counters
                    .subscriptions_rejected_indefinite
                    .fetch_add(1, Ordering::Relaxed);
                return Err(Error::Protocol {
                    class: ErrorClass::SERVICES.to_raw() as u32,
                    code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
                });
            }
            let current_indefinite = self.peer_indefinite_counts.get(peer).copied().unwrap_or(0);
            if current_indefinite + new_indefinite > self.policy.max_indefinite_per_peer {
                self.counters
                    .subscriptions_rejected_indefinite
                    .fetch_add(1, Ordering::Relaxed);
                return Err(Error::Protocol {
                    class: ErrorClass::RESOURCES.to_raw() as u32,
                    code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
                });
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
    fn refuse_new(&self, cap: Cap) -> Error {
        let counter = match cap {
            Cap::Peer => &self.counters.subscriptions_rejected_quota,
            Cap::Table => &self.counters.subscriptions_rejected_capacity,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
        }
    }
}

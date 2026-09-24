use super::*;

impl CovSubscriptionTable {
    /// Accept an insertion or renewal after quota and generation preflight.
    pub fn subscribe(&mut self, sub: CovSubscription) -> Result<CovSubscriptionSnapshot, Error> {
        let key = sub.key()?;
        self.purge_expired();
        let existing = self.subs.get(&key).cloned();
        self.check_admission(
            &sub.peer_key(),
            sub.expires_at.is_none(),
            existing.as_deref(),
        )?;
        let generation = self.reserve_generations(1)?;
        Ok(self.publish(key, sub, generation))
    }

    /// Atomically accept final unique Multiple references and refresh their exact context.
    /// All identities/options are validated before quota/generation reservation or refresh.
    pub fn subscribe_multiple(
        &mut self,
        context: &MultipleContextKey,
        expires_at: Instant,
        mut subscriptions: Vec<CovSubscription>,
    ) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
        for sub in &subscriptions {
            if sub.key()?.multiple_context() != Some(context) || sub.expires_at != Some(expires_at)
            {
                return Err(Error::Encoding(
                    "Multiple subscription does not match its context/expiry".into(),
                ));
            }
        }
        // Final options win exactly once, retaining the final-occurrence request order.
        subscriptions.reverse();
        let mut keys = std::collections::HashSet::new();
        subscriptions.retain(|sub| keys.insert(sub.key().expect("validated identity")));
        subscriptions.reverse();
        self.purge_expired();
        let new_count = keys
            .iter()
            .filter(|key| !self.subs.contains_key(key))
            .count();
        let peer =
            CovPeerKey::from_endpoint(&context.endpoint.mac, context.endpoint.network.as_ref());
        self.check_admission_multiple(&peer, new_count, 0)?;
        let first_generation = self.reserve_generations(subscriptions.len())?;
        // No fallible step follows this point. Unreplaced context references retain generations.
        let mut previously_indefinite = 0;
        for entry in self.subs.values_mut() {
            if entry.key.multiple_context() == Some(context) {
                previously_indefinite += usize::from(entry.expires_at.is_none());
                entry.subscription.expires_at = Some(expires_at);
            }
        }
        if let Some(count) = self.peer_indefinite_counts.get_mut(&peer) {
            *count -= previously_indefinite;
            if *count == 0 {
                self.peer_indefinite_counts.remove(&peer);
            }
        }
        Ok(subscriptions
            .into_iter()
            .enumerate()
            .map(|(offset, sub)| {
                self.publish(
                    sub.key().expect("validated identity"),
                    sub,
                    first_generation + offset as u64,
                )
            })
            .collect())
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
    ) -> CovSubscriptionSnapshot {
        let snapshot = CovSubscriptionSnapshot {
            key,
            generation,
            owner: Arc::clone(&self.owner),
            subscription: sub.clone(),
        };
        let peer = sub.peer_key();
        let new_indefinite = sub.expires_at.is_none();
        if let Some(old) = self.subs.insert(snapshot.key.clone(), snapshot.clone()) {
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
        peer: &CovPeerKey,
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

        self.check_admission_multiple(peer, new_count, new_indefinite)
    }

    /// Check admission for a batch of subscriptions against policy quotas.
    fn check_admission_multiple(
        &mut self,
        peer: &CovPeerKey,
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

        if new_count == 0 {
            return Ok(());
        }

        let current_peer = self.peer_counts.get(peer).copied().unwrap_or(0);
        if current_peer + new_count > self.policy.max_subscriptions_per_peer {
            self.counters
                .subscriptions_rejected_quota
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Protocol {
                class: ErrorClass::RESOURCES.to_raw() as u32,
                code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
            });
        }

        if self.subs.len() + new_count > self.policy.max_subscriptions_global {
            self.counters
                .subscriptions_rejected_capacity
                .fetch_add(1, Ordering::Relaxed);
            return Err(Error::Protocol {
                class: ErrorClass::RESOURCES.to_raw() as u32,
                code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
            });
        }

        if !self.policy.is_peer_reserved(peer) {
            let unreserved_capacity = self.policy.effective_unreserved_capacity();
            if self.subs.len() + new_count > unreserved_capacity {
                self.counters
                    .subscriptions_rejected_capacity
                    .fetch_add(1, Ordering::Relaxed);
                return Err(Error::Protocol {
                    class: ErrorClass::RESOURCES.to_raw() as u32,
                    code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
                });
            }
        }

        Ok(())
    }
}

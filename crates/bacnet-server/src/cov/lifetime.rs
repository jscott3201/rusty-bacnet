use super::*;
use std::num::NonZeroU32;

/// Monotonic remaining lifetime before a new COV notification is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovTimeRemaining {
    /// No expiry; represented by zero in an owned ordinary COV notification.
    Indefinite,
    /// A live finite lifetime, rounded upward and bounded to the wire range.
    Finite(NonZeroU32),
    /// The deadline has been reached; it cannot authorize a new notification.
    Expired,
}

impl CovTimeRemaining {
    /// Project one supplied monotonic instant. Positive fractions round upward;
    /// values beyond the wire range saturate. This is a local representation policy.
    pub fn at(expires_at: Option<Instant>, now: Instant) -> Self {
        let Some(expires_at) = expires_at else {
            return Self::Indefinite;
        };
        let Some(remaining) = expires_at.checked_duration_since(now) else {
            return Self::Expired;
        };
        if remaining.is_zero() {
            return Self::Expired;
        }
        let seconds = remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() != 0));
        Self::Finite(NonZeroU32::new(u32::try_from(seconds).unwrap_or(u32::MAX)).unwrap())
    }

    /// Wire seconds for an eligible owned subscription; expiry is never wire zero.
    pub fn wire_seconds(self) -> Option<u32> {
        match self {
            Self::Indefinite => Some(0),
            Self::Finite(seconds) => Some(seconds.get()),
            Self::Expired => None,
        }
    }
}

impl CovSubscriptionTable {
    /// Resolve a captured owner/key/generation against the live table expiry.
    /// An expiry-only Multiple context refresh does not replace the generation.
    pub fn remaining_lifetime(
        &self,
        snapshot: &CovSubscriptionSnapshot,
        now: Instant,
    ) -> Option<CovTimeRemaining> {
        if !Arc::ptr_eq(&self.owner, &snapshot.owner) {
            return None;
        }
        self.subs.get(snapshot.key()).and_then(|entry| {
            (entry.generation == snapshot.generation)
                .then(|| CovTimeRemaining::at(entry.expires_at, now))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cov_lifetime_projection_distinguishes_finite_indefinite_and_expired() {
        let now = Instant::now();
        assert_eq!(
            CovTimeRemaining::at(None, now),
            CovTimeRemaining::Indefinite
        );
        assert_eq!(CovTimeRemaining::at(None, now).wire_seconds(), Some(0));
        for (duration, expected) in [
            (Duration::from_nanos(1), 1),
            (Duration::from_millis(999), 1),
            (Duration::from_secs(1), 1),
            (Duration::from_millis(1001), 2),
            (Duration::from_secs(3), 3),
            (Duration::from_secs(u64::from(u32::MAX)), u32::MAX),
            (Duration::from_secs(u64::from(u32::MAX) + 1), u32::MAX),
        ] {
            let projected = CovTimeRemaining::at(Some(now + duration), now);
            assert_eq!(
                projected,
                CovTimeRemaining::Finite(NonZeroU32::new(expected).unwrap())
            );
            assert_eq!(projected.wire_seconds(), Some(expected));
        }
        for expiry in [now, now - Duration::from_nanos(1)] {
            assert_eq!(
                CovTimeRemaining::at(Some(expiry), now),
                CovTimeRemaining::Expired
            );
            assert_eq!(CovTimeRemaining::at(Some(expiry), now).wire_seconds(), None);
        }
    }

    #[test]
    fn cov_lifetime_live_query_follows_context_refresh_and_rejects_stale_owners() {
        let now = Instant::now();
        let proposal = CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[1]),
            subscriber_network: None,
            subscriber_process_identifier: 1,
            monitored_object_identifier: ObjectIdentifier::new(
                bacnet_types::enums::ObjectType::ANALOG_VALUE,
                1,
            )
            .unwrap(),
            issue_confirmed_notifications: false,
            expires_at: Some(now + Duration::from_secs(1)),
            last_notified_observation: None,
            monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Multiple,
            timestamped: false,
        };
        let mut table = CovSubscriptionTable::new();
        let snapshot = table.admit_for_test(proposal.clone(), 0).unwrap();
        let context = snapshot.key().multiple_context().unwrap().clone();
        table
            .subscribe_multiple(&context, now + Duration::from_secs(10), 0, vec![])
            .unwrap();
        assert_eq!(
            table
                .remaining_lifetime(&snapshot, now + Duration::from_secs(2))
                .unwrap()
                .wire_seconds(),
            Some(8)
        );
        assert_eq!(
            table.remaining_lifetime(&snapshot, now + Duration::from_secs(10)),
            Some(CovTimeRemaining::Expired)
        );
        assert_eq!(snapshot.expires_at, proposal.expires_at);
        let foreign = CovSubscriptionTable::new()
            .admit_for_test(proposal.clone(), 0)
            .unwrap();
        assert_eq!(table.remaining_lifetime(&foreign, now), None);
        table.admit_for_test(proposal, 0).unwrap();
        assert_eq!(table.remaining_lifetime(&snapshot, now), None);
        table.unsubscribe(snapshot.key());
        assert_eq!(table.remaining_lifetime(&snapshot, now), None);
    }
}

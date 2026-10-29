use bacnet_objects::event::EventTransition;
use bacnet_objects::notification_class::{MAX_RECIPIENT_LIST_DESTINATIONS, RecipientLookupOutcome};
use bacnet_types::constructed::BACnetRecipient;
use tracing::{debug, warn};

use super::super::event_suppression::{EventSuppression, EventSuppressions};

/// Log a bounded lookup diagnostic, count an outcome that fails closed, and
/// expose the selection. `None` means the lookup failed closed (the
/// Notification Class is missing, or its Recipient_List can't be read, is
/// invalid or is past the cap) and the transition is refused whole. A class
/// that reads fine but selects nobody gives an empty selection.
pub(super) fn matched_recipients_or_log(
    outcome: RecipientLookupOutcome,
    notification_class: u32,
    transition: EventTransition,
    suppressions: &EventSuppressions,
) -> Option<Vec<(BACnetRecipient, u32, bool)>> {
    if let Some(suppression) = EventSuppression::from_lookup(&outcome) {
        suppressions.record(suppression);
    }
    match outcome {
        RecipientLookupOutcome::NotificationClassMissing => {
            warn!(
                notification_class,
                ?transition,
                "Missing Notification Class; delivery suppressed"
            );
            None
        }
        RecipientLookupOutcome::RecipientListUnavailable => {
            warn!(
                notification_class,
                ?transition,
                "Recipient list unavailable; delivery suppressed"
            );
            None
        }
        RecipientLookupOutcome::RecipientListInvalid => {
            warn!(
                notification_class,
                ?transition,
                "Recipient list invalid; delivery suppressed"
            );
            None
        }
        RecipientLookupOutcome::RecipientListTooLong => {
            // Only a custom Notification Class can serve a list past the cap.
            // The whole transition is refused rather than sent to part of the
            // list (#1124).
            warn!(
                notification_class,
                ?transition,
                cap = MAX_RECIPIENT_LIST_DESTINATIONS,
                "Recipient list longer than the cap; delivery suppressed"
            );
            None
        }
        RecipientLookupOutcome::NoConfiguredDestinations => {
            debug!(
                notification_class,
                ?transition,
                "Recipient list empty; no delivery"
            );
            Some(Vec::new())
        }
        RecipientLookupOutcome::NoMatchingDestinations => {
            debug!(
                notification_class,
                ?transition,
                "No eligible recipient; no delivery"
            );
            Some(Vec::new())
        }
        RecipientLookupOutcome::Matched(recipients) => Some(recipients),
    }
}

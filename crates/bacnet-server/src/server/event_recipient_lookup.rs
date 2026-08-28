use bacnet_objects::event::EventTransition;
use bacnet_objects::notification_class::RecipientLookupOutcome;
use bacnet_types::constructed::BACnetRecipient;
use tracing::{debug, warn};

/// Log a bounded lookup diagnostic and expose only successful selections.
pub(super) fn matched_recipients_or_log(
    outcome: RecipientLookupOutcome,
    notification_class: u32,
    transition: EventTransition,
) -> Option<Vec<(BACnetRecipient, u32, bool)>> {
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
        RecipientLookupOutcome::NoConfiguredDestinations => {
            debug!(
                notification_class,
                ?transition,
                "Recipient list empty; no delivery"
            );
            None
        }
        RecipientLookupOutcome::NoMatchingDestinations => {
            debug!(
                notification_class,
                ?transition,
                "No eligible recipient; no delivery"
            );
            None
        }
        RecipientLookupOutcome::Matched(recipients) => Some(recipients),
    }
}

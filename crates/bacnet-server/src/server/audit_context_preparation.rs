//! Fallible context suitability checks before canonical configuration publication.
use super::event_recipient_route::ConfirmedRecipientRoute;
use super::*;
use bacnet_objects::audit::{AuditReporterConfiguration, AuditReporterStatus};

pub(super) fn validate_summary(
    status: &Arc<AuditReporterStatus>,
    configuration: &AuditReporterConfiguration,
    device: ObjectIdentifier,
    route: Option<&Arc<ConfirmedRecipientRoute>>,
    max_apdu: u32,
) -> Result<(), Error> {
    if !configuration.enabled()
        || !configuration
            .auditable_operations
            .contains(bacnet_types::enums::AuditOperation::AUDITING_FAILURE)
    {
        return Ok(());
    }
    let Some(route) = route else {
        return Ok(());
    }; // Unresolved Device configuration remains an explicit startup state.
    let context = super::notification_transactions::AuditFailureContext {
        status: Arc::clone(status),
        epoch: status.configuration_epoch(),
        device,
        confirmed: configuration.confirmed,
        peer: route.canonical_peer.clone(),
        route: Arc::clone(route),
        max_apdu,
    };
    let timestamp = bacnet_types::primitives::BACnetTimeStamp::DateTime {
        date: bacnet_types::primitives::Date {
            year: 124,
            month: 12,
            day: 31,
            day_of_week: 2,
        },
        time: bacnet_types::primitives::Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
    };
    super::audit_reporter::encode_notification(
        &context.notification(u64::MAX, timestamp),
        configuration.confirmed,
        max_apdu,
        255,
    )
    .ok_or_else(super::audit_recipient::denied)
    .map(|_| ())
}

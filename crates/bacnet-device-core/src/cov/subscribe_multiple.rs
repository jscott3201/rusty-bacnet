//! SubscribeCOVPropertyMultiple (Clause 13.16): check the request, then
//! cancel, renew or add each reference it names, keep the timestamped
//! histories in step with the table, and capture the initial report of
//! every timestamped reference it accepted.
use alloc::vec::Vec;

use bacnet_objects::database::ObjectDatabase;
use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
use bacnet_time::{Duration, Instant};
use bacnet_types::constructed::{BACnetObjectPropertyReference, PropertyReference};
use bacnet_types::enums::{ErrorClass, ErrorCode, RejectReason};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;

use super::timed::{TimedHistories, follow_table};
use super::timed_capture::TimedCapture;
use super::{
    CovNotificationKind, CovRecipient, CovSubscription, CovSubscriptionKey,
    CovSubscriptionSnapshot, CovTable, MultipleContextKey, MultipleRefusal, SubscriberEndpoint,
    validate_cov_property,
};

/// A refusal of one COV reference of a SubscribeCOVPropertyMultiple request,
/// which names the monitored object and the reference's property and array
/// index: the first-failed-subscription form of its error (Clause 13.16.2).
fn subscription_error(
    error: Error,
    monitored: ObjectIdentifier,
    reference: &PropertyReference,
) -> Error {
    match error {
        Error::Remote(RemoteError::Error { class, code })
        | Error::Structured { class, code, .. } => Error::with_detail(
            class,
            code,
            Some(ErrorDetail::FirstFailedSubscription(
                BACnetObjectPropertyReference {
                    object_identifier: monitored,
                    property_identifier: reference.property_identifier.to_raw(),
                    property_array_index: reference.property_array_index,
                },
            )),
        ),
        other => other,
    }
}

/// SubscribeCOVPropertyMultiple from `peer`, at `now`, against `db`: the
/// decoded `request` cancels a context or some of its references, or
/// creates an individual COV subscription for each property of each object
/// it names, in request order. The accepted references come back: they are
/// owed an initial notification.
///
/// `subscriber_max_apdu` is the max-APDU-length-accepted of the request's
/// header, when the caller has it; the context's notifications fit it.
///
/// A failure before any COV reference is processed (inconsistent or out of
/// range timing) keeps nothing and carries the bare class and code. Otherwise
/// the references are processed in request order until one fails, whether
/// its object, its property, its timestamping or the subscription caps refuse
/// it (#1059, #1102): the error names that reference, and the
/// [`MultipleRefusal`] carries the references kept before it, which are owed
/// their initial notifications like an accepted request's (#1058).
///
/// `histories` follow the table through the call
/// ([`follow_table`]): a cancelled reference loses its history, an accepted
/// one is bound to its new generation, and each accepted timestamped
/// reference has its current value captured as its initial report, stamped
/// with the Device time of admission. Call it with nothing fenced waiting
/// in `table`; what this request fenced is left there for the host's own
/// part ([`CovTable::take_fenced`]), with the histories' part done.
pub fn subscribe_cov_property_multiple(
    table: &mut CovTable,
    histories: &mut TimedHistories,
    db: &ObjectDatabase,
    peer: &SubscriberEndpoint,
    subscriber_max_apdu: Option<u16>,
    request: SubscribeCOVPropertyMultipleRequest,
    now: Instant,
) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
    let confirmed = request.issue_confirmed_notifications;
    let context = MultipleContextKey {
        recipient: CovRecipient::from_endpoint(&peer.mac, peer.network.as_ref()),
        process_id: request.subscriber_process_identifier,
        confirmed,
    };
    let cancellation = match (request.lifetime, request.max_notification_delay) {
        (None, None) => true,
        (Some(_), Some(_)) => false,
        _ => {
            return Err(Error::reject(RejectReason::INCONSISTENT_PARAMETERS).into());
        }
    };

    if cancellation {
        if request.list_of_cov_subscription_specifications.is_empty() {
            table.unsubscribe_cov_multiple_context(&context);
        } else {
            for spec in &request.list_of_cov_subscription_specifications {
                for cov_ref in &spec.list_of_cov_references {
                    table.unsubscribe(&CovSubscriptionKey::Multiple {
                        context: context.clone(),
                        object: spec.monitored_object_identifier,
                        property: cov_ref.monitored_property.property_identifier,
                        index: cov_ref.monitored_property.property_array_index,
                    });
                }
            }
        }
        follow_table(histories, table, table.fenced(), &[], now);
        return Ok(Vec::new());
    }

    let timestamped = request
        .list_of_cov_subscription_specifications
        .iter()
        .flat_map(|spec| &spec.list_of_cov_references)
        .any(|cov_ref| cov_ref.timestamped);
    // One clock sample both admits timestamped references and stamps their
    // initial report, so a clock turning invalid in between cannot cost it.
    let admission_clock = timestamped
        .then(|| db.clock_frame())
        .flatten()
        .filter(|frame| frame.is_valid_actual_datetime());

    let lifetime = request.lifetime.expect("validated COV-multiple lifetime");
    let max_notification_delay = request
        .max_notification_delay
        .expect("validated COV-multiple max notification delay");
    if lifetime == 0
        || max_notification_delay > 3600
        || u64::from(max_notification_delay) >= u64::from(lifetime)
    {
        return Err(Error::protocol(ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE).into());
    }
    let expires_at = now + Duration::from_secs(u64::from(lifetime));
    // The references that pass validation, in request order, up to the first
    // that fails; `names` holds each one's object and property reference.
    let mut subscriptions = Vec::new();
    let mut names = Vec::new();
    let mut failure = None;

    'specs: for spec in &request.list_of_cov_subscription_specifications {
        let monitored = spec.monitored_object_identifier;
        let object = match db.get(&monitored) {
            Some(object) if object.cov().is_some() => object,
            found => {
                let code = match found {
                    None => ErrorCode::UNKNOWN_OBJECT,
                    Some(_) => ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                };
                let error = Error::protocol(ErrorClass::OBJECT, code);
                // A refusal of the object is a refusal of its first COV
                // reference, the first one that could not be processed.
                failure = Some(match spec.list_of_cov_references.first() {
                    Some(first) => subscription_error(error, monitored, &first.monitored_property),
                    None => error,
                });
                break;
            }
        };

        for cov_ref in &spec.list_of_cov_references {
            let property_identifier = cov_ref.monitored_property.property_identifier;
            let property_array_index = cov_ref.monitored_property.property_array_index;

            // Timestamped is an option of each reference, so a missing clock
            // fails the reference that asks for timestamps, after its object
            // and property, rather than the request (Clause 13.16.2, #1102).
            // Neither error table has a row for it; SERVICES /
            // OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED is Clause 18.5's code for
            // a parameter needing functionality the device lacks, the pair
            // DeviceCommunicationControl gives a clockless device (16.1.1.3).
            let refused = validate_cov_property(object, property_identifier, property_array_index)
                .and_then(|()| {
                    if cov_ref.timestamped && admission_clock.is_none() {
                        Err(Error::protocol(
                            ErrorClass::SERVICES,
                            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                        ))
                    } else {
                        Ok(())
                    }
                });
            if let Err(error) = refused {
                failure = Some(subscription_error(
                    error,
                    monitored,
                    &cov_ref.monitored_property,
                ));
                break 'specs;
            }

            subscriptions.push(CovSubscription {
                subscriber_mac: peer.mac.clone(),
                subscriber_network: peer.network.clone(),
                subscriber_process_identifier: request.subscriber_process_identifier,
                monitored_object_identifier: monitored,
                issue_confirmed_notifications: confirmed,
                expires_at: Some(expires_at),
                last_notified_observation: None,
                monitored_property: Some(property_identifier),
                monitored_property_array_index: property_array_index,
                cov_increment: cov_ref.cov_increment,
                notification_kind: CovNotificationKind::Multiple,
                timestamped: cov_ref.timestamped,
            });
            names.push((monitored, &cov_ref.monitored_property));
        }
    }

    // A failed first reference leaves nothing processed, so nothing changes;
    // not even an existing context's lifetime is renewed.
    let validated = subscriptions.len();
    if validated == 0
        && let Some(error) = failure
    {
        return Err(MultipleRefusal {
            error,
            refused: Some(0),
            committed: Vec::new(),
        });
    }
    let admitted = table.subscribe_multiple(
        &context,
        peer,
        expires_at,
        max_notification_delay,
        subscriber_max_apdu,
        subscriptions,
        now,
    );
    // The histories take the admission before the initial capture reads
    // them: whatever it kept, accepted in full or up to a refusal.
    let published = match &admitted {
        Ok(accepted) => accepted.as_slice(),
        Err(refusal) => refusal.committed.as_slice(),
    };
    follow_table(histories, table, table.fenced(), published, now);
    let (accepted, refusal) = match admitted {
        Ok(accepted) => (accepted, failure.map(|error| (validated, error))),
        // A subscription cap stopped admission before the validated
        // references ran out. A context has no storage or limit of its own,
        // so a new one whose first reference finds no room is refused here
        // too, and the general context error is never sent (#1102).
        Err(MultipleRefusal {
            error,
            refused: Some(position),
            committed,
        }) => {
            let (monitored, reference) = names[position];
            let error = subscription_error(error, monitored, reference);
            (committed, Some((position, error)))
        }
        Err(whole) => return Err(whole),
    };
    if let Some(frame) = admission_clock {
        TimedCapture::initial(table, &accepted, frame, now).run(db, histories, now);
    }
    match refusal {
        None => Ok(accepted),
        Some((position, error)) => Err(MultipleRefusal {
            error,
            refused: Some(position),
            committed: accepted,
        }),
    }
}

#[cfg(test)]
#[path = "subscribe_multiple_tests.rs"]
mod tests;

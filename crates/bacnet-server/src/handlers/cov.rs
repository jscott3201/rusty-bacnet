use super::*;
use crate::runtime_clock;
use bacnet_device_core::cov::{MultipleRefusal, validate_cov_property};

/// A refusal of one COV reference of a SubscribeCOVPropertyMultiple request,
/// which names the monitored object and the reference's property and array
/// index: the first-failed-subscription form of its error (Clause 13.16.2).
fn subscription_error(
    error: Error,
    monitored: ObjectIdentifier,
    reference: &bacnet_types::constructed::PropertyReference,
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

/// Handle a SubscribeCOV request.
///
/// Absent optional fields indicate a cancellation. Otherwise creates or updates
/// a subscription. Returns an error if the monitored object does not exist.
pub fn handle_subscribe_cov(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<(), Error> {
    handle_subscribe_cov_with_initial(table, db, source_mac, service_data).map(|_| ())
}

pub(crate) fn handle_subscribe_cov_with_initial(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
    handle_subscribe_cov_with_initial_endpoint(table, db, source_mac, None, service_data)
}

pub(crate) fn handle_subscribe_cov_with_initial_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    service_data: &[u8],
) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
    let peer = SubscriberEndpoint::new(source_mac, source_network);
    Ok(table
        .subscribe_cov(db, &peer, service_data)?
        .into_iter()
        .collect())
}

/// Handle a SubscribeCOVProperty request.
///
/// Like SubscribeCOV but subscribes to changes on a specific property.
pub fn handle_subscribe_cov_property(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<(), Error> {
    handle_subscribe_cov_property_with_initial(table, db, source_mac, service_data).map(|_| ())
}

pub(crate) fn handle_subscribe_cov_property_with_initial(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
    handle_subscribe_cov_property_with_initial_endpoint(table, db, source_mac, None, service_data)
}

pub(crate) fn handle_subscribe_cov_property_with_initial_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    service_data: &[u8],
) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
    let peer = SubscriberEndpoint::new(source_mac, source_network);
    Ok(table
        .subscribe_cov_property(db, &peer, service_data)?
        .into_iter()
        .collect())
}
/// Handle a SubscribeCOVPropertyMultiple request.
///
/// Creates individual COV subscriptions for each property in each object
/// referenced by the request, in request order. A failed reference ends the
/// request with an error naming it; the references before it stay subscribed
/// (Clause 13.16.2).
pub fn handle_subscribe_cov_property_multiple(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<(), Error> {
    handle_subscribe_cov_property_multiple_with_initial(table, db, source_mac, service_data)
        .map(|_| ())
        .map_err(|refusal| refusal.error)
}

pub(crate) fn handle_subscribe_cov_property_multiple_with_initial(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
    handle_subscribe_cov_property_multiple_with_initial_endpoint(
        table,
        db,
        source_mac,
        None,
        service_data,
    )
}

pub(crate) fn handle_subscribe_cov_property_multiple_with_initial_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    service_data: &[u8],
) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
    use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;

    let request = SubscribeCOVPropertyMultipleRequest::decode(service_data)
        .map_err(Error::into_request_reject)?;
    handle_subscribe_cov_property_multiple_request_endpoint(
        table,
        db,
        source_mac,
        source_network,
        None,
        request,
    )
}

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
pub(crate) fn handle_subscribe_cov_property_multiple_request_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    subscriber_max_apdu: Option<u16>,
    request: bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest,
) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
    let confirmed = request.issue_confirmed_notifications;
    let route = SubscriberEndpoint::new(source_mac, source_network);
    let context = MultipleContextKey {
        recipient: bacnet_device_core::cov::CovRecipient::from_endpoint(source_mac, source_network),
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
    let expires_at =
        runtime_clock::core_now() + bacnet_time::Duration::from_secs(u64::from(lifetime));
    let subscriber_mac = MacAddr::from_slice(source_mac);
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
                subscriber_mac: subscriber_mac.clone(),
                subscriber_network: source_network.cloned(),
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
    let (accepted, refusal) = match table.subscribe_multiple(
        &context,
        &route,
        expires_at,
        max_notification_delay,
        subscriber_max_apdu,
        subscriptions,
    ) {
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
        table.initial_timed_capture(&accepted, frame).run(db);
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

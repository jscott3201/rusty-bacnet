use super::*;

fn cov_property_error(code: ErrorCode) -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}

fn validate_cov_property(
    object: &dyn bacnet_objects::traits::BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<(), Error> {
    let zone_tracking_deferred = object.object_identifier().object_type()
        == ObjectType::LIFE_SAFETY_ZONE
        && property == PropertyIdentifier::TRACKING_VALUE;
    let value = object.read_property(property, array_index).map_err(|_| {
        cov_property_error(if zone_tracking_deferred {
            ErrorCode::NOT_COV_PROPERTY
        } else {
            ErrorCode::UNKNOWN_PROPERTY
        })
    })?;
    if !object.supports_cov_property(property) {
        return Err(cov_property_error(ErrorCode::NOT_COV_PROPERTY));
    }
    crate::cov::prepare::validate_sample(object, property, array_index, &value)?;
    Ok(())
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
    let request = SubscribeCOVRequest::decode(service_data)?;

    // Service consistency is distinct from structural parsing. Validate before
    // lookup, expiry purge or subscription replacement/initial notification.
    if request.lifetime.is_some() && request.issue_confirmed_notifications.is_none() {
        return Err(Error::Reject {
            reason: RejectReason::INCONSISTENT_PARAMETERS.to_raw(),
        });
    }

    if request.is_cancellation() {
        table.unsubscribe(&CovSubscriptionKey::Object {
            endpoint: SubscriberEndpoint::new(source_mac, source_network),
            process_id: request.subscriber_process_identifier,
            object: request.monitored_object_identifier,
        });
        return Ok(Vec::new());
    }

    match db.get(&request.monitored_object_identifier) {
        None => {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
            });
        }
        Some(obj) if !obj.supports_cov() => {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
            });
        }
        _ => {}
    }

    let expires_at = request.lifetime.and_then(|secs| {
        if secs == 0 {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(secs as u64))
        }
    });

    let subscription = CovSubscription {
        subscriber_mac: MacAddr::from_slice(source_mac),
        subscriber_network: source_network.cloned(),
        subscriber_process_identifier: request.subscriber_process_identifier,
        monitored_object_identifier: request.monitored_object_identifier,
        issue_confirmed_notifications: request.issue_confirmed_notifications.unwrap_or(false),
        expires_at,
        last_notified_observation: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    };
    Ok(vec![table.subscribe(subscription)?])
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
    use bacnet_services::cov::SubscribeCOVPropertyRequest;

    let request = SubscribeCOVPropertyRequest::decode(service_data)?;

    // Keep service validity separate from structural decoding so malformed
    // pairing and zero lifetime retain their distinct formal responses.
    let lifetime = match (request.issue_confirmed_notifications, request.lifetime) {
        (None, None) => None,
        (Some(_), Some(0)) => {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32,
            })
        }
        (Some(_), Some(lifetime)) => Some(lifetime),
        _ => {
            return Err(Error::Reject {
                reason: RejectReason::INCONSISTENT_PARAMETERS.to_raw(),
            })
        }
    };

    if request.is_cancellation() {
        table.unsubscribe(&CovSubscriptionKey::Property {
            endpoint: SubscriberEndpoint::new(source_mac, source_network),
            process_id: request.subscriber_process_identifier,
            object: request.monitored_object_identifier,
            property: request.monitored_property_identifier,
            index: request.monitored_property_array_index,
        });
        return Ok(Vec::new());
    }

    let object = db
        .get(&request.monitored_object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    if !object.supports_cov() {
        return Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        });
    }

    validate_cov_property(
        object,
        request.monitored_property_identifier,
        request.monitored_property_array_index,
    )?;

    let expires_at = lifetime.map(|secs| Instant::now() + Duration::from_secs(u64::from(secs)));

    let subscription = CovSubscription {
        subscriber_mac: MacAddr::from_slice(source_mac),
        subscriber_network: source_network.cloned(),
        subscriber_process_identifier: request.subscriber_process_identifier,
        monitored_object_identifier: request.monitored_object_identifier,
        issue_confirmed_notifications: request.issue_confirmed_notifications.unwrap_or(false),
        expires_at,
        last_notified_observation: None,
        monitored_property: Some(request.monitored_property_identifier),
        monitored_property_array_index: request.monitored_property_array_index,
        cov_increment: request.cov_increment,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    };
    Ok(vec![table.subscribe(subscription)?])
}
/// Handle a SubscribeCOVPropertyMultiple request.
///
/// Creates individual COV subscriptions for each property in each object
/// referenced by the request.
pub fn handle_subscribe_cov_property_multiple(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<(), Error> {
    handle_subscribe_cov_property_multiple_with_initial(table, db, source_mac, service_data)
        .map(|_| ())
}

pub(crate) fn handle_subscribe_cov_property_multiple_with_initial(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    service_data: &[u8],
) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
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
) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
    use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;

    let request = SubscribeCOVPropertyMultipleRequest::decode(service_data)?;
    handle_subscribe_cov_property_multiple_request_endpoint(
        table,
        db,
        source_mac,
        source_network,
        request,
    )
}

pub(crate) fn handle_subscribe_cov_property_multiple_request_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    request: bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest,
) -> Result<Vec<CovSubscriptionSnapshot>, Error> {
    let confirmed = request.issue_confirmed_notifications;
    let context = MultipleContextKey {
        endpoint: SubscriberEndpoint::new(source_mac, source_network),
        process_id: request.subscriber_process_identifier,
        confirmed,
    };
    let cancellation = match (request.lifetime, request.max_notification_delay) {
        (None, None) => true,
        (Some(_), Some(_)) => false,
        _ => {
            return Err(Error::Reject {
                reason: RejectReason::INCONSISTENT_PARAMETERS.to_raw(),
            });
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
    if timestamped
        && !db
            .clock_frame()
            .is_some_and(|frame| frame.is_valid_actual_datetime())
    {
        return Err(Error::Protocol {
            class: ErrorClass::SERVICES.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        });
    }

    let lifetime = request.lifetime.expect("validated COV-multiple lifetime");
    let max_notification_delay = request
        .max_notification_delay
        .expect("validated COV-multiple max notification delay");
    if lifetime == 0
        || max_notification_delay > 3600
        || u64::from(max_notification_delay) >= u64::from(lifetime)
    {
        return Err(Error::Protocol {
            class: ErrorClass::SERVICES.to_raw() as u32,
            code: ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32,
        });
    }
    let expires_at = Instant::now() + Duration::from_secs(u64::from(lifetime));
    let subscriber_mac = MacAddr::from_slice(source_mac);
    let mut subscriptions = Vec::new();

    for spec in &request.list_of_cov_subscription_specifications {
        let object = db
            .get(&spec.monitored_object_identifier)
            .ok_or(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
            })?;

        if !object.supports_cov() {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT.to_raw() as u32,
                code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
            });
        }

        for cov_ref in &spec.list_of_cov_references {
            let property_identifier = cov_ref.monitored_property.property_identifier;
            let property_array_index = cov_ref.monitored_property.property_array_index;

            validate_cov_property(object, property_identifier, property_array_index)?;

            let subscription = CovSubscription {
                subscriber_mac: subscriber_mac.clone(),
                subscriber_network: source_network.cloned(),
                subscriber_process_identifier: request.subscriber_process_identifier,
                monitored_object_identifier: spec.monitored_object_identifier,
                issue_confirmed_notifications: confirmed,
                expires_at: Some(expires_at),
                last_notified_observation: None,
                monitored_property: Some(property_identifier),
                monitored_property_array_index: property_array_index,
                cov_increment: cov_ref.cov_increment,
                notification_kind: CovNotificationKind::Multiple,
                timestamped: cov_ref.timestamped,
            };
            subscriptions.push(subscription);
        }
    }

    table.subscribe_multiple(&context, expires_at, subscriptions)
}

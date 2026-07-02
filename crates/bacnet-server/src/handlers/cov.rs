use super::*;

const MAX_COV_SUBSCRIPTIONS: usize = 1024;

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
) -> Result<Vec<CovSubscription>, Error> {
    handle_subscribe_cov_with_initial_endpoint(table, db, source_mac, None, service_data)
}

pub(crate) fn handle_subscribe_cov_with_initial_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    service_data: &[u8],
) -> Result<Vec<CovSubscription>, Error> {
    let request = SubscribeCOVRequest::decode(service_data)?;

    if request.is_cancellation() {
        table.unsubscribe_at(
            source_mac,
            source_network,
            request.subscriber_process_identifier,
            request.monitored_object_identifier,
        );
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

    if table.len() >= MAX_COV_SUBSCRIPTIONS
        && !table.contains(
            &MacAddr::from_slice(source_mac),
            source_network,
            request.subscriber_process_identifier,
            request.monitored_object_identifier,
            None,
        )
    {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
        });
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
        last_notified_value: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    };
    table.subscribe(subscription.clone());

    Ok(vec![subscription])
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
) -> Result<Vec<CovSubscription>, Error> {
    handle_subscribe_cov_property_with_initial_endpoint(table, db, source_mac, None, service_data)
}

pub(crate) fn handle_subscribe_cov_property_with_initial_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    service_data: &[u8],
) -> Result<Vec<CovSubscription>, Error> {
    use bacnet_services::cov::SubscribeCOVPropertyRequest;

    let request = SubscribeCOVPropertyRequest::decode(service_data)?;

    if request.is_cancellation() {
        table.unsubscribe_property_at(
            source_mac,
            source_network,
            request.subscriber_process_identifier,
            request.monitored_object_identifier,
            request.monitored_property_identifier,
        );
        return Ok(Vec::new());
    }

    let object = db
        .get(&request.monitored_object_identifier)
        .ok_or(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
        })?;

    object
        .read_property(
            request.monitored_property_identifier,
            request.monitored_property_array_index,
        )
        .map_err(|_| Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
        })?;

    if table.len() >= MAX_COV_SUBSCRIPTIONS
        && !table.contains(
            &MacAddr::from_slice(source_mac),
            source_network,
            request.subscriber_process_identifier,
            request.monitored_object_identifier,
            Some(request.monitored_property_identifier),
        )
    {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
        });
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
        last_notified_value: None,
        monitored_property: Some(request.monitored_property_identifier),
        monitored_property_array_index: request.monitored_property_array_index,
        cov_increment: request.cov_increment,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    };
    table.subscribe(subscription.clone());

    Ok(vec![subscription])
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
) -> Result<Vec<CovSubscription>, Error> {
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
) -> Result<Vec<CovSubscription>, Error> {
    use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;

    let request = SubscribeCOVPropertyMultipleRequest::decode(service_data)?;

    let confirmed = request.issue_confirmed_notifications.unwrap_or(false);
    let subscriber_mac = MacAddr::from_slice(source_mac);
    let mut subscriptions = Vec::new();
    let mut new_keys = HashSet::new();

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

            object
                .read_property(property_identifier, property_array_index)
                .map_err(|_| Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
                })?;

            if !table.contains(
                &subscriber_mac,
                source_network,
                request.subscriber_process_identifier,
                spec.monitored_object_identifier,
                Some(property_identifier),
            ) {
                new_keys.insert((
                    subscriber_mac.clone(),
                    source_network.cloned(),
                    request.subscriber_process_identifier,
                    spec.monitored_object_identifier,
                    Some(property_identifier),
                ));
            }

            let subscription = CovSubscription {
                subscriber_mac: subscriber_mac.clone(),
                subscriber_network: source_network.cloned(),
                subscriber_process_identifier: request.subscriber_process_identifier,
                monitored_object_identifier: spec.monitored_object_identifier,
                issue_confirmed_notifications: confirmed,
                expires_at: None, // SubscribeCOVPropertyMultiple has no lifetime
                last_notified_value: None,
                monitored_property: Some(property_identifier),
                monitored_property_array_index: property_array_index,
                cov_increment: cov_ref.cov_increment,
                notification_kind: CovNotificationKind::Multiple,
                timestamped: cov_ref.timestamped,
            };
            subscriptions.push(subscription);
        }
    }

    if table.len() + new_keys.len() > MAX_COV_SUBSCRIPTIONS {
        return Err(Error::Protocol {
            class: ErrorClass::RESOURCES.to_raw() as u32,
            code: ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32,
        });
    }

    for subscription in &subscriptions {
        table.subscribe(subscription.clone());
    }

    Ok(subscriptions)
}

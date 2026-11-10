use super::*;
use bacnet_device_core::cov::MultipleRefusal;

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

/// SubscribeCOVPropertyMultiple for an already decoded `request`, from the
/// endpoint the request arrived on; see
/// [`CovSubscriptionTable::subscribe_cov_property_multiple`].
/// `subscriber_max_apdu` is the max-APDU-length-accepted of the request's
/// header, when the caller has it.
pub(crate) fn handle_subscribe_cov_property_multiple_request_endpoint(
    table: &mut CovSubscriptionTable,
    db: &ObjectDatabase,
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    subscriber_max_apdu: Option<u16>,
    request: bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest,
) -> Result<Vec<CovSubscriptionSnapshot>, MultipleRefusal> {
    let peer = SubscriberEndpoint::new(source_mac, source_network);
    table.subscribe_cov_property_multiple(db, &peer, subscriber_max_apdu, request)
}

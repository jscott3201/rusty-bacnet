//! SubscribeCOV and SubscribeCOVProperty (Clauses 13.14 and 13.15): decode
//! the request, check it against the object it names, then cancel, renew
//! or add.
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_time::{Duration, Instant};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier, RejectReason};
use bacnet_types::error::Error;

use super::{
    CovNotificationKind, CovRecipient, CovSubscription, CovSubscriptionKey,
    CovSubscriptionSnapshot, CovTable, SubscriberEndpoint, validate_sample,
};

fn cov_property_error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

/// Whether `property` (at `array_index`) of `object` can be monitored: it is
/// there, the object reports changes of it, and its value can be retained.
/// The errors are SubscribeCOVProperty's (Clause 13.15.1.3.1).
pub fn validate_cov_property(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<(), Error> {
    // An index on a scalar is judged by `validate_sample` below, as
    // PROPERTY_IS_NOT_AN_ARRAY; the read here only proves the property is
    // there, so it drops that index rather than take the object's refusal
    // of it as UNKNOWN_PROPERTY (#2274).
    let read_index = array_index
        .filter(|_| bacnet_objects::property_metadata::is_array_property(object, property));
    let value = object
        .read_property(property, read_index)
        .map_err(|_| cov_property_error(ErrorCode::UNKNOWN_PROPERTY))?;
    if !object
        .cov()
        .is_some_and(|cov| cov.supports_cov_property(property))
    {
        return Err(cov_property_error(ErrorCode::NOT_COV_PROPERTY));
    }
    validate_sample(object, property, array_index, &value)?;
    Ok(())
}

impl CovTable {
    /// SubscribeCOV from `peer`, at `now`, against `db`. Absent optional
    /// fields cancel the subscription, which succeeds whether or not it
    /// existed. Otherwise the subscription is added or renewed, and comes
    /// back: it is owed an initial notification.
    pub fn subscribe_cov(
        &mut self,
        db: &ObjectDatabase,
        peer: &SubscriberEndpoint,
        service_data: &[u8],
        now: Instant,
    ) -> Result<Option<CovSubscriptionSnapshot>, Error> {
        let request =
            SubscribeCOVRequest::decode(service_data).map_err(Error::into_request_reject)?;

        // Service consistency is distinct from structural parsing. Validate before
        // lookup, expiry purge or subscription replacement/initial notification.
        if request.lifetime.is_some() && request.issue_confirmed_notifications.is_none() {
            return Err(Error::reject(RejectReason::INCONSISTENT_PARAMETERS));
        }

        if request.is_cancellation() {
            self.unsubscribe(&CovSubscriptionKey::Object {
                recipient: CovRecipient::from_endpoint(&peer.mac, peer.network.as_ref()),
                process_id: request.subscriber_process_identifier,
                object: request.monitored_object_identifier,
            });
            return Ok(None);
        }

        match db.get(&request.monitored_object_identifier) {
            None => {
                return Err(Error::protocol(
                    ErrorClass::OBJECT,
                    ErrorCode::UNKNOWN_OBJECT,
                ));
            }
            Some(obj) if !obj.cov().is_some_and(|cov| cov.supports_subscribe_cov()) => {
                return Err(Error::protocol(
                    ErrorClass::OBJECT,
                    ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                ));
            }
            _ => {}
        }

        let expires_at = request.lifetime.and_then(|secs| {
            if secs == 0 {
                None
            } else {
                Some(now + Duration::from_secs(u64::from(secs)))
            }
        });

        let subscription = CovSubscription {
            subscriber_mac: peer.mac.clone(),
            subscriber_network: peer.network.clone(),
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
        self.subscribe(subscription, now).map(Some)
    }

    /// SubscribeCOVProperty from `peer`, at `now`, against `db`: as
    /// [`subscribe_cov`](Self::subscribe_cov), for one property (and array
    /// index) of the object, with its own COV increment if the request
    /// gives one.
    pub fn subscribe_cov_property(
        &mut self,
        db: &ObjectDatabase,
        peer: &SubscriberEndpoint,
        service_data: &[u8],
        now: Instant,
    ) -> Result<Option<CovSubscriptionSnapshot>, Error> {
        let request = SubscribeCOVPropertyRequest::decode(service_data)
            .map_err(Error::into_request_reject)?;

        // Keep service validity separate from structural decoding so malformed
        // pairing and zero lifetime retain their distinct formal responses.
        let lifetime = match (request.issue_confirmed_notifications, request.lifetime) {
            (None, None) => None,
            (Some(_), Some(0)) => {
                return Err(Error::protocol(
                    ErrorClass::SERVICES,
                    ErrorCode::VALUE_OUT_OF_RANGE,
                ));
            }
            (Some(_), Some(lifetime)) => Some(lifetime),
            _ => {
                return Err(Error::reject(RejectReason::INCONSISTENT_PARAMETERS));
            }
        };

        if request.is_cancellation() {
            self.unsubscribe(&CovSubscriptionKey::Property {
                recipient: CovRecipient::from_endpoint(&peer.mac, peer.network.as_ref()),
                process_id: request.subscriber_process_identifier,
                object: request.monitored_object_identifier,
                property: request.monitored_property_identifier,
                index: request.monitored_property_array_index,
            });
            return Ok(None);
        }

        let object = db
            .get(&request.monitored_object_identifier)
            .ok_or(Error::protocol(
                ErrorClass::OBJECT,
                ErrorCode::UNKNOWN_OBJECT,
            ))?;

        // An object can take property subscriptions without supporting
        // SubscribeCOV (Averaging, which Table 13-1 doesn't list).
        if object.cov().is_none() {
            return Err(Error::protocol(
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ));
        }

        validate_cov_property(
            object,
            request.monitored_property_identifier,
            request.monitored_property_array_index,
        )?;

        let expires_at = lifetime.map(|secs| now + Duration::from_secs(u64::from(secs)));

        let subscription = CovSubscription {
            subscriber_mac: peer.mac.clone(),
            subscriber_network: peer.network.clone(),
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
        self.subscribe(subscription, now).map(Some)
    }
}

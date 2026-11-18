use super::*;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::cov_multiple::SubscribeCOVPropertyMultipleRequest;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::ObjectIdentifier;
use std::num::NonZeroU32;

/// Parameters of a single-property COV subscription with an explicit finite lifetime.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CovPropertySubscription {
    /// Subscriber process identifier echoed in notifications.
    pub subscriber_process_identifier: u32,
    /// Object whose property is monitored.
    pub monitored_object_identifier: ObjectIdentifier,
    /// Property to monitor.
    pub monitored_property_identifier: PropertyIdentifier,
    /// Optional array index of the monitored property.
    pub monitored_property_array_index: Option<u32>,
    /// Whether notifications are confirmed.
    pub confirmed: bool,
    /// Subscription lifetime in seconds; must be positive.
    pub lifetime: NonZeroU32,
    /// Optional COV increment for numeric properties.
    pub cov_increment: Option<f32>,
}

impl CovPropertySubscription {
    pub(super) fn into_request(self) -> SubscribeCOVPropertyRequest {
        SubscribeCOVPropertyRequest {
            subscriber_process_identifier: self.subscriber_process_identifier,
            monitored_object_identifier: self.monitored_object_identifier,
            issue_confirmed_notifications: Some(self.confirmed),
            lifetime: Some(self.lifetime.get()),
            monitored_property_identifier: self.monitored_property_identifier,
            monitored_property_array_index: self.monitored_property_array_index,
            cov_increment: self.cov_increment,
        }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    pub(super) fn subscribe_cov_request(
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: Option<bool>,
        lifetime: Option<u32>,
    ) -> SubscribeCOVRequest {
        SubscribeCOVRequest {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications: confirmed,
            lifetime,
        }
    }

    fn subscribe_cov_property_request(
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: Option<bool>,
        lifetime: Option<u32>,
        monitored_property_identifier: PropertyIdentifier,
        monitored_property_array_index: Option<u32>,
        cov_increment: Option<f32>,
    ) -> SubscribeCOVPropertyRequest {
        SubscribeCOVPropertyRequest {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications: confirmed,
            lifetime,
            monitored_property_identifier,
            monitored_property_array_index,
            cov_increment,
        }
    }

    /// Subscribe to COV notifications for an object on the station `to`.
    pub async fn subscribe_cov(
        &self,
        to: &(impl ToStation + ?Sized),
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            Some(confirmed),
            lifetime,
        );
        self.call(to, request).await
    }

    /// Cancel a COV subscription on the station `to`.
    pub async fn unsubscribe_cov(
        &self,
        to: &(impl ToStation + ?Sized),
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );
        self.call(to, request).await
    }

    /// Subscribe to COV notifications for a single property on the station
    /// `to`. Lifetime must be positive; use `unsubscribe_cov_property` for
    /// cancellation.
    pub async fn subscribe_cov_property(
        &self,
        to: &(impl ToStation + ?Sized),
        subscription: CovPropertySubscription,
    ) -> Result<(), Error> {
        self.call(to, subscription.into_request()).await
    }

    /// Cancel a single-property COV subscription on the station `to`.
    pub async fn unsubscribe_cov_property(
        &self,
        to: &(impl ToStation + ?Sized),
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
        monitored_property_identifier: PropertyIdentifier,
        monitored_property_array_index: Option<u32>,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_property_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
            monitored_property_identifier,
            monitored_property_array_index,
            None,
        );
        self.call(to, request).await
    }

    /// Subscribe to COV notifications for several properties of several
    /// objects on the station `to`, or cancel such a subscription, with one
    /// SubscribeCOVPropertyMultiple (Clause 13.16). The whole request is
    /// checked before anything is sent
    /// ([`SubscribeCOVPropertyMultipleRequest::validate`]). A device that
    /// refuses one of the subscriptions says which in the error's
    /// first-failed-subscription detail.
    ///
    /// The notifications such a subscription brings arrive on
    /// [`Self::cov_notifications`] as [`COVEvent::Multiple`], beside the
    /// single-object ones, in the order they came.
    pub async fn subscribe_cov_property_multiple(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &SubscribeCOVPropertyMultipleRequest,
    ) -> Result<(), Error> {
        self.call(to, request).await
    }

    /// Get a receiver for incoming COV notifications of both forms: about one
    /// object ([`COVEvent::Object`]: ConfirmedCOVNotification and
    /// UnconfirmedCOVNotification) or about several ([`COVEvent::Multiple`]:
    /// ConfirmedCOVNotificationMultiple and UnconfirmedCOVNotificationMultiple),
    /// in the order they arrive. Each call returns a new independent receiver.
    ///
    /// The client acknowledges a confirmed notification of either form, and
    /// publishes it here, whether or not it knows the subscription: it keeps
    /// no list of them. Match one by its subscriber process identifier and
    /// its source ([`Station::is_source_of`]), or set
    /// [`ClientOptions::with_confirmed_cov_notification_ack_policy`] to
    /// answer otherwise. A notification the sender sends again, its answer
    /// lost, is published again.
    pub fn cov_notifications(&self) -> broadcast::Receiver<COVEvent> {
        self.cov_tx.subscribe()
    }
}

#[cfg(test)]
#[path = "cov_property_validation_tests.rs"]
mod property_validation_tests;

#[cfg(test)]
#[path = "cov_request_validation_tests.rs"]
mod request_validation_tests;

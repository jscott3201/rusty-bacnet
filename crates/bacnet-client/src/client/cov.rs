use super::*;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
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

    pub(super) async fn send_subscribe_cov_request(
        &self,
        target: ConfirmedTarget<'_>,
        request: SubscribeCOVRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request_inner(target, ConfirmedServiceChoice::SUBSCRIBE_COV, &buf)
            .await?;

        Ok(())
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

    pub(super) async fn send_subscribe_cov_property_request(
        &self,
        target: ConfirmedTarget<'_>,
        request: SubscribeCOVPropertyRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request_inner(target, ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, &buf)
            .await?;

        Ok(())
    }

    /// Subscribe to COV notifications for an object at a directly reachable MAC address.
    pub async fn subscribe_cov(
        &self,
        destination_mac: &[u8],
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

        self.send_subscribe_cov_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Subscribe to COV notifications for a discovered device, auto-routing if needed.
    pub async fn subscribe_cov_to_device(
        &self,
        device_instance: u32,
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

        request.validate()?;
        let (mac, routing) = self.resolve_device(device_instance).await?;

        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Cancel a COV subscription on a remote device.
    pub async fn unsubscribe_cov(
        &self,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );

        self.send_subscribe_cov_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Cancel a COV subscription for a discovered device, auto-routing if needed.
    pub async fn unsubscribe_cov_to_device(
        &self,
        device_instance: u32,
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );

        request.validate()?;
        let (mac, routing) = self.resolve_device(device_instance).await?;

        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Subscribe to COV notifications for a single property at a directly reachable MAC address.
    /// Lifetime must be positive; use `unsubscribe_cov_property` for cancellation.
    pub async fn subscribe_cov_property(
        &self,
        destination_mac: &[u8],
        subscription: CovPropertySubscription,
    ) -> Result<(), Error> {
        let request = subscription.into_request();

        self.send_subscribe_cov_property_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Subscribe to COV notifications for a single property on a discovered device,
    /// auto-routing if needed. Lifetime must be positive; cancellation has a separate method.
    pub async fn subscribe_cov_property_to_device(
        &self,
        device_instance: u32,
        subscription: CovPropertySubscription,
    ) -> Result<(), Error> {
        let request = subscription.into_request();

        request.validate()?;
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_property_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_property_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Cancel a single-property COV subscription at a directly reachable MAC address.
    pub async fn unsubscribe_cov_property(
        &self,
        destination_mac: &[u8],
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

        self.send_subscribe_cov_property_request(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            request,
        )
        .await
    }

    /// Cancel a single-property COV subscription for a discovered device,
    /// auto-routing if needed.
    pub async fn unsubscribe_cov_property_to_device(
        &self,
        device_instance: u32,
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

        request.validate()?;
        let (mac, routing) = self.resolve_device(device_instance).await?;

        if let Some((dnet, dadr)) = routing {
            self.send_subscribe_cov_property_request(
                ConfirmedTarget::Routed {
                    router_mac: &mac,
                    dest_network: dnet,
                    dest_mac: &dadr,
                },
                request,
            )
            .await
        } else {
            self.send_subscribe_cov_property_request(ConfirmedTarget::Local { mac: &mac }, request)
                .await
        }
    }

    /// Cancel a COV subscription at `target`, the device a
    /// [`manage_cov_subscription`](Self::manage_cov_subscription) went to.
    /// The cancel goes the way the subscription did, with no device-table
    /// lookup, so it reaches the same device even if the table has since
    /// dropped or moved its entry.
    pub async fn unsubscribe_cov_target(
        &self,
        target: &CovTarget,
        subscriber_process_identifier: u32,
        monitored_object_identifier: ObjectIdentifier,
    ) -> Result<(), Error> {
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );
        self.send_subscribe_cov_request(target.confirmed(), request)
            .await
    }

    /// Cancel a single-property COV subscription at `target`:
    /// [`unsubscribe_cov_target`](Self::unsubscribe_cov_target) for one
    /// property.
    pub async fn unsubscribe_cov_property_target(
        &self,
        target: &CovTarget,
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
        self.send_subscribe_cov_property_request(target.confirmed(), request)
            .await
    }

    /// Get a receiver for incoming COV notifications. Each call returns a new
    /// independent receiver.
    pub fn cov_notifications(&self) -> broadcast::Receiver<ReceivedCOVNotification> {
        self.cov_tx.subscribe()
    }
}

#[cfg(test)]
#[path = "cov_property_validation_tests.rs"]
mod property_validation_tests;

#[cfg(test)]
#[path = "cov_request_validation_tests.rs"]
mod request_validation_tests;

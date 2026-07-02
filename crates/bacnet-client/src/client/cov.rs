use super::*;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_types::primitives::ObjectIdentifier;

impl<T: TransportPort + 'static> BACnetClient<T> {
    fn subscribe_cov_request(
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

    async fn send_subscribe_cov_request(
        &self,
        target: ConfirmedTarget<'_>,
        request: SubscribeCOVRequest,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request_inner(target, ConfirmedServiceChoice::SUBSCRIBE_COV, &buf)
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
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            Some(confirmed),
            lifetime,
        );

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
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = Self::subscribe_cov_request(
            subscriber_process_identifier,
            monitored_object_identifier,
            None,
            None,
        );

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

    /// Get a receiver for incoming COV notifications. Each call returns a new
    /// independent receiver.
    pub fn cov_notifications(&self) -> broadcast::Receiver<COVNotificationRequest> {
        self.cov_tx.subscribe()
    }
}

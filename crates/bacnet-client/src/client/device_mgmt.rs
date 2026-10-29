use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send DeviceCommunicationControl to a remote device.
    pub async fn device_communication_control(
        &self,
        destination_mac: &[u8],
        enable_disable: bacnet_types::enums::EnableDisable,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;

        let request = DeviceCommunicationControlRequest {
            time_duration,
            enable_disable,
            password,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Send ReinitializeDevice to a remote device.
    pub async fn reinitialize_device(
        &self,
        destination_mac: &[u8],
        reinitialized_state: bacnet_types::enums::ReinitializedState,
        password: Option<String>,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::ReinitializeDeviceRequest;

        let request = ReinitializeDeviceRequest {
            reinitialized_state,
            password,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::REINITIALIZE_DEVICE,
                &buf,
            )
            .await?;

        Ok(())
    }
    /// Send a TimeSynchronization request (unconfirmed, no response expected).
    pub async fn time_synchronization(
        &self,
        destination_mac: &[u8],
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::TimeSynchronizationRequest;

        let request = TimeSynchronizationRequest { date, time };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.unconfirmed_request(
            destination_mac,
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            &buf,
        )
        .await
    }

    /// Send a UTCTimeSynchronization request (unconfirmed, no response expected).
    pub async fn utc_time_synchronization(
        &self,
        destination_mac: &[u8],
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::TimeSynchronizationRequest;

        let request = TimeSynchronizationRequest { date, time };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.unconfirmed_request(
            destination_mac,
            UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
            &buf,
        )
        .await
    }
    /// Send DeviceCommunicationControl to a discovered device, routing when needed.
    ///
    /// Preserves the request fields and confirmation semantics of [`Self::device_communication_control`].
    /// A missing device fails before sending.
    pub async fn device_communication_control_from_device(
        &self,
        device_instance: u32,
        enable_disable: bacnet_types::enums::EnableDisable,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
                time_duration,
                enable_disable,
                password,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf)?;
            self.confirmed_request_routed(
                &mac,
                network,
                &address,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                &buf,
            )
            .await?;
            Ok(())
        } else {
            self.device_communication_control(&mac, enable_disable, time_duration, password)
                .await
        }
    }

    /// Send ReinitializeDevice to a discovered device, routing when needed.
    ///
    /// Preserves the request fields and confirmation semantics of [`Self::reinitialize_device`].
    /// A missing device fails before sending.
    pub async fn reinitialize_device_from_device(
        &self,
        device_instance: u32,
        reinitialized_state: bacnet_types::enums::ReinitializedState,
        password: Option<String>,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        if let Some((network, address)) = routing {
            let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
                reinitialized_state,
                password,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf)?;
            self.confirmed_request_routed(
                &mac,
                network,
                &address,
                ConfirmedServiceChoice::REINITIALIZE_DEVICE,
                &buf,
            )
            .await?;
            Ok(())
        } else {
            self.reinitialize_device(&mac, reinitialized_state, password)
                .await
        }
    }

    /// Send local time synchronization to a discovered device, routing when needed.
    ///
    /// This unconfirmed service sends once without waiting for a response.
    /// Success means local send completion, not acceptance by the remote clock.
    pub async fn time_synchronization_from_device(
        &self,
        device_instance: u32,
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        self.time_synchronization_to_device(
            device_instance,
            date,
            time,
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
        )
        .await
    }

    /// Send UTC time synchronization to a discovered device, routing when needed.
    ///
    /// This unconfirmed service sends once without waiting for a response.
    /// Success means local send completion, not acceptance by the remote clock.
    pub async fn utc_time_synchronization_from_device(
        &self,
        device_instance: u32,
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        self.time_synchronization_to_device(
            device_instance,
            date,
            time,
            UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
        )
        .await
    }

    async fn time_synchronization_to_device(
        &self,
        device_instance: u32,
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
        service_choice: UnconfirmedServiceChoice,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        let request = bacnet_services::device_mgmt::TimeSynchronizationRequest { date, time };
        let mut data = BytesMut::new();
        request.encode(&mut data);
        let Some((network, address)) = routing else {
            return self.unconfirmed_request(&mac, service_choice, &data).await;
        };
        check_routed_unicast(network, address.len())?;
        if self.network.local_network_number().get() == Some(network) {
            if self.network.transport().is_group_destination(&address) {
                return Err(Error::Encoding(
                    "routed time synchronization requires a unicast station".into(),
                ));
            }
            return self
                .unconfirmed_request(&address, service_choice, &data)
                .await;
        }
        let apdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: data.freeze(),
        });
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &apdu)?;
        self.network
            .send_apdu_routed(
                &buf,
                network,
                &address,
                &mac,
                false,
                NetworkPriority::Normal,
            )
            .await
    }
}

#[cfg(test)]
#[path = "device_mgmt_tests.rs"]
mod tests;

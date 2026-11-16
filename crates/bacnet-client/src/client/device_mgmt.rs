use super::service::device_target;
use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send DeviceCommunicationControl to the station `to`.
    pub async fn device_communication_control(
        &self,
        to: &(impl ToStation + ?Sized),
        enable_disable: bacnet_types::enums::EnableDisable,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), Error> {
        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration,
            enable_disable,
            password,
        };
        self.call(to, request).await
    }

    /// Send ReinitializeDevice to the station `to`.
    pub async fn reinitialize_device(
        &self,
        to: &(impl ToStation + ?Sized),
        reinitialized_state: bacnet_types::enums::ReinitializedState,
        password: Option<String>,
    ) -> Result<(), Error> {
        let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
            reinitialized_state,
            password,
        };
        self.call(to, request).await
    }

    /// Send a TimeSynchronization request to the station `to` (unconfirmed,
    /// no response expected).
    pub async fn time_synchronization(
        &self,
        to: &(impl ToStation + ?Sized),
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        let choice = UnconfirmedServiceChoice::TIME_SYNCHRONIZATION;
        self.tell(to, clock(choice, date, time)).await
    }

    /// Send a UTCTimeSynchronization request to the station `to`
    /// (unconfirmed, no response expected).
    pub async fn utc_time_synchronization(
        &self,
        to: &(impl ToStation + ?Sized),
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        let choice = UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION;
        self.tell(to, clock(choice, date, time)).await
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
        let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
            time_duration,
            enable_disable,
            password,
        };
        self.call_via(device_target(&mac, routing.as_ref()), request)
            .await
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
        let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
            reinitialized_state,
            password,
        };
        self.call_via(device_target(&mac, routing.as_ref()), request)
            .await
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
        let clock = clock(service_choice, date, time);
        match routing {
            None => self.tell(&mac[..], clock).await,
            // Routed as an address: through the router its row names.
            Some((network, address)) => {
                let station = bacnet_types::constructed::BACnetAddress {
                    network_number: network,
                    mac_address: MacAddr::from_slice(&address),
                };
                self.tell(&station, clock).await
            }
        }
    }
}

/// One time service's request for `date` and `time`.
fn clock(
    choice: UnconfirmedServiceChoice,
    date: bacnet_types::primitives::Date,
    time: bacnet_types::primitives::Time,
) -> super::service::Clock {
    super::service::Clock {
        choice,
        request: bacnet_services::device_mgmt::TimeSynchronizationRequest { date, time },
    }
}

#[cfg(test)]
#[path = "device_mgmt_tests.rs"]
mod tests;

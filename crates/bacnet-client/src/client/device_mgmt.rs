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

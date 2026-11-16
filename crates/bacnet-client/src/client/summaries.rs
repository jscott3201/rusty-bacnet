//! GetEventInformation, GetAlarmSummary and GetEnrollmentSummary, sent and
//! decoded (#2077).

use super::*;

use bacnet_services::alarm_event::{GetEventInformationAck, GetEventInformationRequest};
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_types::primitives::ObjectIdentifier;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// GetEventInformation to the station `to`, its answer decoded: the
    /// objects after `last_received_object_identifier` (from the first when
    /// `None`) with an active event or an unacknowledged transition. While
    /// the answer's `more_events` is set, ask again from its last object.
    pub async fn get_event_information(
        &self,
        to: &(impl ToStation + ?Sized),
        last_received_object_identifier: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        let request = GetEventInformationRequest {
            last_received_object_identifier,
        };
        self.call(to, request).await
    }

    /// [`Self::get_event_information`] to a discovered device, routed when
    /// it is behind a router. A device not in the table fails before
    /// sending.
    pub async fn get_event_information_from_device(
        &self,
        device_instance: u32,
        last_received_object_identifier: Option<ObjectIdentifier>,
    ) -> Result<GetEventInformationAck, Error> {
        let buf = event_information_request(last_received_object_identifier);
        let ack = self
            .confirmed_request_to_device(
                device_instance,
                ConfirmedServiceChoice::GET_EVENT_INFORMATION,
                &buf,
            )
            .await?;
        GetEventInformationAck::decode(&ack)
    }

    /// GetAlarmSummary to the station `to`, its answer decoded.
    pub async fn get_alarm_summary(
        &self,
        to: &(impl ToStation + ?Sized),
    ) -> Result<GetAlarmSummaryAck, Error> {
        self.call(to, super::service::AlarmSummary).await
    }

    /// [`Self::get_alarm_summary`] to a discovered device, routed when it
    /// is behind a router. A device not in the table fails before sending.
    pub async fn get_alarm_summary_from_device(
        &self,
        device_instance: u32,
    ) -> Result<GetAlarmSummaryAck, Error> {
        let ack = self
            .confirmed_request_to_device(
                device_instance,
                ConfirmedServiceChoice::GET_ALARM_SUMMARY,
                &[],
            )
            .await?;
        GetAlarmSummaryAck::decode(&ack)
    }

    /// GetEnrollmentSummary to the station `to`, its answer decoded. A
    /// filter out of its range fails before sending.
    pub async fn get_enrollment_summary(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        self.call(to, request).await
    }

    /// [`Self::get_enrollment_summary`] to a discovered device, routed when
    /// it is behind a router. A device not in the table fails before
    /// sending.
    pub async fn get_enrollment_summary_from_device(
        &self,
        device_instance: u32,
        request: &GetEnrollmentSummaryRequest,
    ) -> Result<GetEnrollmentSummaryAck, Error> {
        let mut buf = BytesMut::new();
        request.try_encode(&mut buf)?;
        let ack = self
            .confirmed_request_to_device(
                device_instance,
                ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
                &buf,
            )
            .await?;
        GetEnrollmentSummaryAck::decode(&ack)
    }

    /// A confirmed request to a discovered device: through its router when
    /// the table holds one, and otherwise to its MAC.
    async fn confirmed_request_to_device(
        &self,
        device_instance: u32,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;
        match routing {
            Some((network, address)) => {
                self.confirmed_request_routed(&mac, network, &address, service_choice, service_data)
                    .await
            }
            None => {
                self.confirmed_request(&mac, service_choice, service_data)
                    .await
            }
        }
    }
}

fn event_information_request(
    last_received_object_identifier: Option<ObjectIdentifier>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    GetEventInformationRequest {
        last_received_object_identifier,
    }
    .encode(&mut buf);
    buf
}

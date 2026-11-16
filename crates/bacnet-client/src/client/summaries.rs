//! GetEventInformation, GetAlarmSummary and GetEnrollmentSummary, sent and
//! decoded (#2077).

use super::service::AlarmSummary;
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

    /// GetAlarmSummary to the station `to`, its answer decoded.
    pub async fn get_alarm_summary(
        &self,
        to: &(impl ToStation + ?Sized),
    ) -> Result<GetAlarmSummaryAck, Error> {
        self.call(to, AlarmSummary).await
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
}

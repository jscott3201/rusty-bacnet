//! The typed calls' one mechanism (#2588): a service is described once, by
//! its service choice, how its request encodes and the check its answer
//! must pass, and every typed call sends it the same way to any station.
//! A new service is one more description and a thin documented method.

use bacnet_services::alarm_event::{
    AcknowledgeAlarmRequest, GetEventInformationAck, GetEventInformationRequest,
};
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest};
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::device_mgmt::{
    DeviceCommunicationControlRequest, ReinitializeDeviceRequest, TimeSynchronizationRequest,
};
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_services::file::{AtomicReadFileAck, AtomicReadFileRequest, AtomicWriteFileRequest};
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::object_mgmt::{CreateObjectAck, CreateObjectRequest, DeleteObjectRequest};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::read_range::ReadRangeValidation;
use bacnet_services::read_range::{ReadRangeAck, ReadRangeReply, ReadRangeRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_services::who_is::WhoIsRequest;
use bacnet_services::wpm::WritePropertyMultipleRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::primitives::ObjectIdentifier;

use super::*;

/// A confirmed service as a typed call sends it.
pub(super) trait Confirmed {
    /// What a call returns once the answer passes its check.
    type Answer;
    /// The service the request asks for.
    fn choice(&self) -> ConfirmedServiceChoice;
    /// The request's service data, appended to `buf`. An error here fails
    /// the call before anything is sent.
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error>;
    /// The answer's service data (empty for a SimpleAck), checked against
    /// the request and decoded.
    fn answer(self, data: Bytes) -> Result<Self::Answer, Error>;
}

/// An unconfirmed service as a typed call sends it.
pub(super) trait Unconfirmed {
    /// The service the request asks for.
    fn choice(&self) -> UnconfirmedServiceChoice;
    /// The request's service data, appended to `buf`.
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error>;
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send `service` to `to` and check its answer: every typed confirmed
    /// call is this, through the request path every request takes.
    pub(super) async fn call<S: Confirmed>(
        &self,
        to: &(impl ToStation + ?Sized),
        service: S,
    ) -> Result<S::Answer, Error> {
        let mut buf = BytesMut::new();
        service.encode_into(&mut buf)?;
        let answer = to.confirmed(self, service.choice(), &buf).await?;
        service.answer(answer)
    }

    /// Send the unconfirmed `service` to `to`, once: success means it was
    /// sent.
    pub(super) async fn tell<S: Unconfirmed>(
        &self,
        to: &(impl ToStation + ?Sized),
        service: S,
    ) -> Result<(), Error> {
        let mut buf = BytesMut::new();
        service.encode_into(&mut buf)?;
        to.unconfirmed(self, service.choice(), &buf).await
    }

    /// [`Self::call`] to a target already resolved: the by-instance calls'
    /// row and the managed COV subscriptions' [`CovTarget`].
    pub(super) async fn call_via<S: Confirmed>(
        &self,
        target: ConfirmedTarget<'_>,
        service: S,
    ) -> Result<S::Answer, Error> {
        self.send_via(target, Encoded::new(service)?).await
    }

    /// Send a service encoded beforehand to `target` and check its answer.
    pub(super) async fn send_via<S: Confirmed>(
        &self,
        target: ConfirmedTarget<'_>,
        encoded: Encoded<S>,
    ) -> Result<S::Answer, Error> {
        let Encoded { service, data } = encoded;
        let answer = self
            .confirmed_request_inner(target, service.choice(), &data)
            .await?;
        service.answer(answer)
    }
}

/// A service with its request encoded, for a call that refuses a request it
/// can't encode before it looks the device up.
pub(super) struct Encoded<S> {
    service: S,
    data: BytesMut,
}

impl<S: Confirmed> Encoded<S> {
    /// `service`'s request, encoded.
    pub(super) fn new(service: S) -> Result<Self, Error> {
        let mut data = BytesMut::new();
        service.encode_into(&mut data)?;
        Ok(Self { service, data })
    }
}

/// Where a by-instance call goes: to the row's MAC, or through it as the
/// router to the station `routing` names. #2588 PR 2: removed with the
/// by-instance family.
pub(super) fn device_target<'a>(
    mac: &'a [u8],
    routing: &'a Option<(u16, Vec<u8>)>,
) -> ConfirmedTarget<'a> {
    match routing {
        Some((network, address)) => ConfirmedTarget::Routed {
            router_mac: mac,
            dest_network: *network,
            dest_mac: address,
        },
        None => ConfirmedTarget::Local { mac },
    }
}

/// A service whose answer is taken as its raw service data, unchecked.
pub(super) struct Raw<S>(pub(super) S);

impl<S: Confirmed> Confirmed for Raw<S> {
    type Answer = Bytes;
    fn choice(&self) -> ConfirmedServiceChoice {
        self.0.choice()
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.0.encode_into(buf)
    }
    fn answer(self, data: Bytes) -> Result<Bytes, Error> {
        Ok(data)
    }
}

/// A service answered with a SimpleAck: its encoding, and `()` for the
/// answer. `$encode` writes `$request` (`self`) into `buf`. Each choice is
/// written out in full, so the source scan of `SENT_SERVICES` sees it.
macro_rules! simple {
    ($($request:ty => $choice:expr, |$this:ident, $buf:ident| $encode:expr;)*) => {$(
        impl Confirmed for $request {
            type Answer = ();
            fn choice(&self) -> ConfirmedServiceChoice {
                $choice
            }
            fn encode_into(&self, $buf: &mut BytesMut) -> Result<(), Error> {
                let $this = self;
                $encode
            }
            fn answer(self, _: Bytes) -> Result<(), Error> {
                Ok(())
            }
        }
    )*};
}

simple! {
    WritePropertyRequest => ConfirmedServiceChoice::WRITE_PROPERTY, |r, buf| r.encode(buf);
    WritePropertyMultipleRequest => ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        |r, buf| r.encode(buf);
    DeviceCommunicationControlRequest => ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        |r, buf| r.encode(buf);
    ReinitializeDeviceRequest => ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        |r, buf| r.encode(buf);
    DeleteObjectRequest => ConfirmedServiceChoice::DELETE_OBJECT, |r, buf| {
        r.encode(buf);
        Ok(())
    };
    &AcknowledgeAlarmRequest => ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, |r, buf| r.encode(buf);
    SubscribeCOVRequest => ConfirmedServiceChoice::SUBSCRIBE_COV, |r, buf| r.encode(buf);
    SubscribeCOVPropertyRequest => ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
        |r, buf| r.encode(buf);
    &AuditNotificationRequest => ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        |r, buf| r.try_encode(buf);
}

/// AddListElement or RemoveListElement, which share a request.
pub(super) struct ListChange {
    pub(super) choice: ConfirmedServiceChoice,
    pub(super) request: ListElementRequest,
}

impl Confirmed for ListChange {
    type Answer = ();
    fn choice(&self) -> ConfirmedServiceChoice {
        self.choice
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.request.encode(buf)
    }
    fn answer(self, _: Bytes) -> Result<(), Error> {
        Ok(())
    }
}

impl Confirmed for ReadPropertyRequest {
    type Answer = ReadPropertyACK;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::READ_PROPERTY
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.encode(buf);
        Ok(())
    }
    fn answer(self, data: Bytes) -> Result<ReadPropertyACK, Error> {
        crate::acks::read_property(&self, &data)
    }
}

impl Confirmed for ReadPropertyMultipleRequest {
    type Answer = ReadPropertyMultipleACK;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.encode(buf)
    }
    fn answer(self, data: Bytes) -> Result<ReadPropertyMultipleACK, Error> {
        ReadPropertyMultipleACK::decode(&data)
    }
}

/// ReadRange, its answer checked against the request under `validation`.
pub(super) struct Range<'a> {
    pub(super) request: &'a ReadRangeRequest,
    pub(super) validation: ReadRangeValidation,
}

impl Confirmed for Range<'_> {
    type Answer = ReadRangeReply;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::READ_RANGE
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.request.encode(buf)
    }
    fn answer(self, data: Bytes) -> Result<ReadRangeReply, Error> {
        ReadRangeReply::check(self.request, ReadRangeAck::decode(&data)?, self.validation)
    }
}

impl Confirmed for AtomicReadFileRequest {
    type Answer = AtomicReadFileAck;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::ATOMIC_READ_FILE
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.encode(buf);
        Ok(())
    }
    fn answer(self, data: Bytes) -> Result<AtomicReadFileAck, Error> {
        crate::acks::atomic_read_file(&self.access, &data)
    }
}

/// AtomicWriteFile: its answer is returned raw.
impl Confirmed for AtomicWriteFileRequest {
    type Answer = Bytes;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::ATOMIC_WRITE_FILE
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.encode(buf);
        Ok(())
    }
    fn answer(self, data: Bytes) -> Result<Bytes, Error> {
        Ok(data)
    }
}

impl Confirmed for CreateObjectRequest {
    type Answer = ObjectIdentifier;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::CREATE_OBJECT
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.encode(buf)
    }
    fn answer(self, data: Bytes) -> Result<ObjectIdentifier, Error> {
        CreateObjectAck::decode(&data).map(|ack| ack.object_identifier)
    }
}

impl Confirmed for GetEventInformationRequest {
    type Answer = GetEventInformationAck;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::GET_EVENT_INFORMATION
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.encode(buf);
        Ok(())
    }
    fn answer(self, data: Bytes) -> Result<GetEventInformationAck, Error> {
        GetEventInformationAck::decode(&data)
    }
}

/// GetAlarmSummary, which has no request parameters.
pub(super) struct AlarmSummary;

impl Confirmed for AlarmSummary {
    type Answer = GetAlarmSummaryAck;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::GET_ALARM_SUMMARY
    }
    fn encode_into(&self, _: &mut BytesMut) -> Result<(), Error> {
        Ok(())
    }
    fn answer(self, data: Bytes) -> Result<GetAlarmSummaryAck, Error> {
        GetAlarmSummaryAck::decode(&data)
    }
}

impl Confirmed for &GetEnrollmentSummaryRequest {
    type Answer = GetEnrollmentSummaryAck;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.try_encode(buf)
    }
    fn answer(self, data: Bytes) -> Result<GetEnrollmentSummaryAck, Error> {
        GetEnrollmentSummaryAck::decode(&data)
    }
}

impl Confirmed for &AuditLogQueryRequest {
    type Answer = AuditLogQueryAck;
    fn choice(&self) -> ConfirmedServiceChoice {
        ConfirmedServiceChoice::AUDIT_LOG_QUERY
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.try_encode(buf)
    }
    fn answer(self, data: Bytes) -> Result<AuditLogQueryAck, Error> {
        AuditLogQueryAck::decode(&data)
    }
}

/// TimeSynchronization or UTCTimeSynchronization, which share a request.
pub(super) struct Clock {
    pub(super) choice: UnconfirmedServiceChoice,
    pub(super) request: TimeSynchronizationRequest,
}

impl Unconfirmed for Clock {
    fn choice(&self) -> UnconfirmedServiceChoice {
        self.choice
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.request.encode(buf);
        Ok(())
    }
}

impl Unconfirmed for &AuditNotificationRequest {
    fn choice(&self) -> UnconfirmedServiceChoice {
        UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.try_encode(buf)
    }
}

impl Unconfirmed for WhoIsRequest {
    fn choice(&self) -> UnconfirmedServiceChoice {
        UnconfirmedServiceChoice::WHO_IS
    }
    fn encode_into(&self, buf: &mut BytesMut) -> Result<(), Error> {
        self.encode(buf);
        Ok(())
    }
}

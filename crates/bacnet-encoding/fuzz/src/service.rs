//! `service_decode`: the service parameter codecs of `bacnet-services`.
//!
//! The first octet picks a codec, by its remainder over the number of them
//! ([`CODECS`] names them in that order); the octets after it are the
//! service parameters. Selectors 0 to 33 are the confirmed requests by
//! service choice (Clause 21), so a seed's first octet can be its request's
//! own, except that GetAlarmSummary (3), whose request has no parameters,
//! and the reserved choices 13, 24 and 25 hold an ACK instead. Then come the
//! unconfirmed requests, the other ACKs and the formal error productions,
//! which a client decodes from its peers. Each round trips as
//! [`round_trip`] states; the forwarded event notification and the borrowed
//! ReadProperty-ACK, the last two, have no encoder and only decode.
//!
//! [`round_trip`]: crate::round_trip

use bacnet_encoding::constructed::{decode_event_notification, encode_event_notification};
use bacnet_services::alarm_event::{
    AcknowledgeAlarmRequest, ForwardedEventNotification, GetEventInformationAck,
    GetEventInformationRequest,
};
use bacnet_services::alarm_summary::GetAlarmSummaryAck;
use bacnet_services::audit::{AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest};
use bacnet_services::cov::{
    COVNotificationRequest, SubscribeCOVPropertyRequest, SubscribeCOVRequest,
};
use bacnet_services::cov_multiple::{
    COVNotificationMultipleRequest, SubscribeCOVPropertyMultipleError,
    SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::device_mgmt::{
    DeviceCommunicationControlRequest, ReinitializeDeviceRequest, TimeSynchronizationRequest,
};
use bacnet_services::enrollment_summary::{GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest};
use bacnet_services::file::{
    AtomicReadFileAck, AtomicReadFileRequest, AtomicWriteFileAck, AtomicWriteFileRequest,
};
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::list_manipulation::{ChangeListError, ListElementRequest};
use bacnet_services::object_mgmt::{
    CreateObjectAck, CreateObjectError, CreateObjectRequest, DeleteObjectRequest,
};
use bacnet_services::private_transfer::{
    PrivateTransferAck, PrivateTransferError, PrivateTransferRequest,
};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyAckRef, ReadPropertyRequest};
use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_services::text_message::TextMessageRequest;
use bacnet_services::virtual_terminal::{
    VTCloseError, VTCloseRequest, VTDataAck, VTDataRequest, VTOpenAck, VTOpenRequest,
};
use bacnet_services::who_am_i::{WhoAmIRequest, YouAreRequest};
use bacnet_services::who_has::{IHaveRequest, WhoHasRequest};
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_services::wpm::{WritePropertyMultipleError, WritePropertyMultipleRequest};
use bacnet_services::write_group::WriteGroupRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::EventNotificationRequest;
use bacnet_types::error::Error;

use crate::{select, whole};

/// An encoder's result, whether it can refuse or not.
trait Encoded {
    fn into_result(self) -> Result<(), Error>;
}

impl Encoded for () {
    fn into_result(self) -> Result<(), Error> {
        Ok(())
    }
}

impl Encoded for Result<(), Error> {
    fn into_result(self) -> Result<(), Error> {
        self
    }
}

/// ConfirmedEventNotification-Request, whose codec is a pair of
/// functions in the encoding crate, as a type with the methods the table
/// calls.
#[derive(Debug)]
struct EventNotification(EventNotificationRequest);

impl EventNotification {
    fn decode(data: &[u8]) -> Result<Self, Error> {
        decode_event_notification(data).map(Self)
    }

    fn encode(&self, buf: &mut bytes::BytesMut) -> Result<(), Error> {
        encode_event_notification(&self.0, buf)
    }
}

/// Builds the codec table from service types, each with its `decode` and
/// `encode` methods, then the decoders with no encoder.
macro_rules! services {
    ($($name:literal: $type:ty;)*) => {
        codecs! {
            $($name: whole(<$type>::decode) => |value: &$type, buf: &mut bytes::BytesMut| {
                value.encode(buf).into_result()
            };)*
            "ForwardedEventNotification": whole(ForwardedEventNotification::decode);
            "ReadProperty-ACK, borrowed": |data: &[u8]| {
                ReadPropertyAckRef::decode(data).map(|_| ((), data.len()))
            };
        }
    };
}

services! {
    "AcknowledgeAlarm-Request": AcknowledgeAlarmRequest;
    "ConfirmedCOVNotification-Request": COVNotificationRequest;
    "ConfirmedEventNotification-Request": EventNotification;
    "GetAlarmSummary-ACK (the request has no parameters)": GetAlarmSummaryAck;
    "GetEnrollmentSummary-Request": GetEnrollmentSummaryRequest;
    "SubscribeCOV-Request": SubscribeCOVRequest;
    "AtomicReadFile-Request": AtomicReadFileRequest;
    "AtomicWriteFile-Request": AtomicWriteFileRequest;
    "AddListElement-Request": ListElementRequest;
    "RemoveListElement-Request": ListElementRequest;
    "CreateObject-Request": CreateObjectRequest;
    "DeleteObject-Request": DeleteObjectRequest;
    "ReadProperty-Request": ReadPropertyRequest;
    "ReadProperty-ACK (13 is reserved)": ReadPropertyACK;
    "ReadPropertyMultiple-Request": ReadPropertyMultipleRequest;
    "WriteProperty-Request": WritePropertyRequest;
    "WritePropertyMultiple-Request": WritePropertyMultipleRequest;
    "DeviceCommunicationControl-Request": DeviceCommunicationControlRequest;
    "ConfirmedPrivateTransfer-Request": PrivateTransferRequest;
    "ConfirmedTextMessage-Request": TextMessageRequest;
    "ReinitializeDevice-Request": ReinitializeDeviceRequest;
    "VT-Open-Request": VTOpenRequest;
    "VT-Close-Request": VTCloseRequest;
    "VT-Data-Request": VTDataRequest;
    "ReadPropertyMultiple-ACK (24 is reserved)": ReadPropertyMultipleACK;
    "ReadRange-ACK (25 is reserved)": ReadRangeAck;
    "ReadRange-Request": ReadRangeRequest;
    "LifeSafetyOperation-Request": LifeSafetyOperationRequest;
    "SubscribeCOVProperty-Request": SubscribeCOVPropertyRequest;
    "GetEventInformation-Request": GetEventInformationRequest;
    "SubscribeCOVPropertyMultiple-Request": SubscribeCOVPropertyMultipleRequest;
    "ConfirmedCOVNotificationMultiple-Request": COVNotificationMultipleRequest;
    "ConfirmedAuditNotification-Request": AuditNotificationRequest;
    "AuditLogQuery-Request": AuditLogQueryRequest;
    "I-Am-Request": IAmRequest;
    "I-Have-Request": IHaveRequest;
    "TimeSynchronization-Request": TimeSynchronizationRequest;
    "Who-Has-Request": WhoHasRequest;
    "Who-Is-Request": WhoIsRequest;
    "WriteGroup-Request": WriteGroupRequest;
    "Who-Am-I-Request": WhoAmIRequest;
    "You-Are-Request": YouAreRequest;
    "AtomicReadFile-ACK": AtomicReadFileAck;
    "AtomicWriteFile-ACK": AtomicWriteFileAck;
    "AuditLogQuery-ACK": AuditLogQueryAck;
    "CreateObject-ACK": CreateObjectAck;
    "ConfirmedPrivateTransfer-ACK": PrivateTransferAck;
    "GetEnrollmentSummary-ACK": GetEnrollmentSummaryAck;
    "GetEventInformation-ACK": GetEventInformationAck;
    "VT-Open-ACK": VTOpenAck;
    "VT-Data-ACK": VTDataAck;
    "ChangeList-Error": ChangeListError;
    "CreateObject-Error": CreateObjectError;
    "ConfirmedPrivateTransfer-Error": PrivateTransferError;
    "SubscribeCOVPropertyMultiple-Error": SubscribeCOVPropertyMultipleError;
    "VT-Close-Error": VTCloseError;
    "WritePropertyMultiple-Error": WritePropertyMultipleError;
}

/// Runs the codec the first octet picks on the rest.
pub fn check(input: &[u8]) {
    let (selector, rest) = select(input);
    run(usize::from(selector) % CODECS.len(), rest);
}

#[cfg(test)]
mod tests {
    use super::CODECS;

    /// Selectors 0 to 33 are the confirmed service choices.
    #[test]
    fn confirmed_requests_sit_at_their_service_choice() {
        assert_eq!(CODECS[0], "AcknowledgeAlarm-Request");
        assert_eq!(CODECS[12], "ReadProperty-Request");
        assert_eq!(CODECS[15], "WriteProperty-Request");
        assert_eq!(CODECS[26], "ReadRange-Request");
        assert_eq!(CODECS[33], "AuditLogQuery-Request");
    }
}

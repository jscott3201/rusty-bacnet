//! PrivateTransfer, TextMessage, LifeSafetyOperation and the virtual
//! terminal services as typed calls (#2588), each one service description
//! sent to any station.

use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::private_transfer::{PrivateTransferAck, PrivateTransferRequest};
use bacnet_services::text_message::TextMessageRequest;
use bacnet_services::virtual_terminal::{
    VTCloseRequest, VTDataAck, VTDataRequest, VTOpenAck, VTOpenRequest,
};

use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Send a ConfirmedPrivateTransfer to the station `to` (Clause 16.2):
    /// a vendor's own service, by vendor and service number, with its
    /// parameters as the vendor encodes them. Returns the device's
    /// acknowledgement, its result block included when it sent one.
    pub async fn confirmed_private_transfer(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &PrivateTransferRequest,
    ) -> Result<PrivateTransferAck, Error> {
        self.call(to, request).await
    }

    /// Send an UnconfirmedPrivateTransfer to the station `to` (Clause
    /// 16.3), once: success means it was sent.
    pub async fn unconfirmed_private_transfer(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &PrivateTransferRequest,
    ) -> Result<(), Error> {
        self.tell(to, request).await
    }

    /// Send a ConfirmedTextMessage to the station `to` (Clause 16.5). A
    /// message or class text that can't be encoded fails before anything
    /// is sent.
    pub async fn confirmed_text_message(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &TextMessageRequest,
    ) -> Result<(), Error> {
        self.call(to, request).await
    }

    /// Send an UnconfirmedTextMessage to the station `to` (Clause 16.6),
    /// once: success means it was sent.
    pub async fn unconfirmed_text_message(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &TextMessageRequest,
    ) -> Result<(), Error> {
        self.tell(to, request).await
    }

    /// Send a LifeSafetyOperation to the station `to` (Clause 13.13): a
    /// silence, reset or unsilence, of one object or every one the device
    /// applies it to. A requesting source that can't be encoded fails
    /// before anything is sent.
    pub async fn life_safety_operation(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &LifeSafetyOperationRequest,
    ) -> Result<(), Error> {
        self.call(to, request).await
    }

    /// Open a virtual terminal session on the station `to` (Clause 17.2).
    /// Returns the device's acknowledgement, which names the session by
    /// the device's own number for it.
    pub async fn vt_open(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &VTOpenRequest,
    ) -> Result<VTOpenAck, Error> {
        self.call(to, request).await
    }

    /// Close virtual terminal sessions on the station `to` (Clause 17.3).
    /// An empty list fails before anything is sent
    /// ([`VTCloseRequest::validate`]).
    pub async fn vt_close(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &VTCloseRequest,
    ) -> Result<(), Error> {
        self.call(to, request).await
    }

    /// Send data over a virtual terminal session on the station `to`
    /// (Clause 17.4). Returns whether the device took all of it, or how
    /// many octets it took.
    pub async fn vt_data(
        &self,
        to: &(impl ToStation + ?Sized),
        request: &VTDataRequest,
    ) -> Result<VTDataAck, Error> {
        self.call(to, request).await
    }
}

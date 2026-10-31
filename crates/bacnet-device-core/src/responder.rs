//! [`Responder`]: the staged steps strung together for a host with nothing
//! to await between them, such as firmware that owns its database.
use crate::discovery;
use crate::read::{self, ReadFailure, ReadOverlay, ReadPropertyMultipleBudget, ReadScope};
use crate::reply::{self, SEGMENTED, emit};
use bacnet_encoding::apdu::{
    AbortPdu, Apdu, ComplexAck, ConfirmedRequest, UnconfirmedRequest, decode_apdu,
};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, PduType, Segmentation, ServiceSupported,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

/// The services a [`Responder`] runs, for the Device's
/// Protocol_Services_Supported: build the database from
/// `DeviceIdentity::with_services(SERVICES)`.
pub const SERVICES: &[ServiceSupported] = &[
    ServiceSupported::READ_PROPERTY,
    ServiceSupported::WHO_HAS,
    ServiceSupported::WHO_IS,
];

/// What a request needs from the database, read from its header alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Access {
    /// ReadProperty, Who-Is or Who-Has: [`Responder::read`] under a shared
    /// guard.
    Read,
    /// Nothing: a request the core doesn't run, answered (if at all) by
    /// [`reply::refuse`], or one the host routes to its own handlers. A
    /// segmented request is one.
    None,
}

/// One request APDU, as a host received it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Inbound {
    /// The whole APDU, header included.
    pub apdu: Bytes,
    /// The longest reply APDU the requester and the route to it take: the
    /// request's Max-APDU-Length-Accepted, capped by the route's own limit.
    pub max_reply: u16,
}

impl Inbound {
    /// The request `apdu`, whose reply may be up to `max_reply` octets.
    pub fn new(apdu: Bytes, max_reply: u16) -> Self {
        Self { apdu, max_reply }
    }
}

/// What the core did with one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// A confirmed reply is in `out`, for the host to send to the requester.
    Reply,
    /// An I-Am or I-Have is in `out`, for the host to address and
    /// rate-limit.
    Announce,
    /// Nothing to send.
    Silent,
    /// Not a service this core runs; `out` is as it was.
    Unhandled,
}

/// The limits a [`Responder`] reads within.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Result rows one ReadProperty may expand: its own row plus, for a
    /// Group's whole Present_Value, every member row. A read past it is
    /// aborted with OUT_OF_RESOURCES.
    pub read_work: usize,
}

impl Limits {
    /// Limits of `read_work` result rows per ReadProperty.
    pub fn new(read_work: usize) -> Self {
        Self { read_work }
    }
}

impl Default for Limits {
    /// The server's default: the ReadPropertyMultiple budget's result-row
    /// limit, which the server also applies to ReadProperty.
    fn default() -> Self {
        Self::new(ReadPropertyMultipleBudget::default().max_result_elements)
    }
}

/// Answers ReadProperty, Who-Is and Who-Has from a borrowed database, for
/// one Device.
///
/// It holds the Device's frozen identity: a Who-Is is answered for it, and
/// the Device wildcard instance reads it, even after the object is gone. A
/// Who-Has names the database's selected Device, as the server's does.
#[derive(Clone, Debug)]
pub struct Responder {
    identity: DeviceIdentity,
    scope: ReadScope,
}

impl Responder {
    /// A responder for `identity`, within `limits`.
    ///
    /// It refuses an identity that claims segmentation support: the core
    /// aborts every segmented request and every reply too long for one APDU,
    /// so an I-Am naming segmentation would overstate it. Give it
    /// `Segmentation::NONE`, the [`DeviceIdentity`] default. It also refuses
    /// a `read_work` of zero, which no ReadProperty fits.
    pub fn new(identity: DeviceIdentity, limits: Limits) -> Result<Self, Error> {
        if identity.segmentation() != Segmentation::NONE {
            return Err(Error::Encoding(
                "bacnet-device-core segments neither way: the Device identity must claim no segmentation"
                    .into(),
            ));
        }
        if limits.read_work == 0 {
            return Err(Error::Encoding("read_work must be positive".into()));
        }
        let scope = ReadScope {
            work_limit: limits.read_work,
            registered_port: None,
            device: Some(identity.device_oid()),
        };
        Ok(Self { identity, scope })
    }

    /// Resolve the Network_Port wildcard instance to `port`, the port the
    /// requests arrive on, while the database registers it.
    pub fn with_registered_port(mut self, port: ObjectIdentifier) -> Self {
        self.scope.registered_port = Some(port);
        self
    }

    /// What `apdu` needs from the database, from its PDU type, segmented
    /// flag and service choice alone; nothing is decoded.
    pub fn classify(apdu: &[u8]) -> Access {
        let Some(&first) = apdu.first() else {
            return Access::None;
        };
        let pdu_type = first >> 4;
        if pdu_type == PduType::CONFIRMED_REQUEST.to_raw() {
            // Type and flags, max segments and APDU, invoke ID, service.
            let read = first & SEGMENTED == 0
                && apdu.get(3) == Some(&ConfirmedServiceChoice::READ_PROPERTY.to_raw());
            if read {
                return Access::Read;
            }
        } else if pdu_type == PduType::UNCONFIRMED_REQUEST.to_raw()
            && apdu.get(1).is_some_and(|&service| {
                service == UnconfirmedServiceChoice::WHO_IS.to_raw()
                    || service == UnconfirmedServiceChoice::WHO_HAS.to_raw()
            })
        {
            return Access::Read;
        }
        Access::None
    }

    /// Run `request` if it is a ReadProperty, Who-Is or Who-Has, writing any
    /// answer to `out`; `overlay` serves objects as the host does. A
    /// confirmed request that arrived segmented is refused. Any other
    /// service is [`Outcome::Unhandled`], for the host to route on.
    pub fn read<O: ReadOverlay>(
        &self,
        db: &ObjectDatabase,
        overlay: Option<&O>,
        request: Inbound,
        out: &mut BytesMut,
    ) -> Outcome {
        match decode_apdu(request.apdu) {
            Ok(Apdu::ConfirmedRequest(confirmed)) if confirmed.segmented => emit(
                out,
                &reply::refuse_segmented(confirmed.invoke_id),
                Outcome::Reply,
            ),
            Ok(Apdu::ConfirmedRequest(confirmed))
                if confirmed.service_choice == ConfirmedServiceChoice::READ_PROPERTY =>
            {
                let answer = self.read_property(db, overlay, &confirmed);
                emit(out, &reply::fit(answer, request.max_reply), Outcome::Reply)
            }
            Ok(Apdu::UnconfirmedRequest(unconfirmed)) => self.discover(db, &unconfirmed, out),
            Ok(_) => Outcome::Unhandled,
            // Neither a type nor an invoke ID to answer.
            Err(_) => Outcome::Silent,
        }
    }

    /// For firmware that owns its database: [`read`](Self::read) with no
    /// overlay, and every confirmed request the core doesn't run answered
    /// by [`reply::refuse`]. An unconfirmed request it doesn't run is
    /// [`Outcome::Silent`], and so is an APDU that doesn't decode, which the
    /// server drops too.
    pub fn handle(&self, db: &mut ObjectDatabase, request: Inbound, out: &mut BytesMut) -> Outcome {
        let apdu = request.apdu.clone();
        // `read` decodes every APDU, so only a well-formed request it
        // doesn't run reaches the refusal.
        match self.read(db, None::<&read::NoOverlay>, request, out) {
            Outcome::Unhandled => reply::refuse(&apdu, out),
            outcome => outcome,
        }
    }

    /// ReadProperty, as the server runs it: a request that doesn't decode is
    /// rejected, a refused read is its Error PDU, and a read past the work
    /// limit is aborted with OUT_OF_RESOURCES.
    fn read_property<O: ReadOverlay>(
        &self,
        db: &ObjectDatabase,
        overlay: Option<&O>,
        request: &ConfirmedRequest,
    ) -> Apdu {
        let mut service_ack = BytesMut::new();
        let result = ReadPropertyRequest::decode(&request.service_request)
            .map_err(|error| ReadFailure::Service(Error::into_request_reject(error)))
            .and_then(|decoded| {
                let target = read::resolve_target(db, &self.scope, &decoded.object_identifier);
                let plan = read::plan_property(
                    db,
                    &self.scope,
                    overlay,
                    target,
                    decoded.property_identifier,
                    decoded.property_array_index,
                )?;
                read::read_property_ack(db, overlay, &decoded, plan, &mut service_ack, |_, _, _| {})
                    .map_err(ReadFailure::Service)
            });
        match result {
            Ok(()) => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_ack: service_ack.freeze(),
            }),
            Err(ReadFailure::Service(error)) => {
                reply::error_apdu(request.invoke_id, request.service_choice, &error)
            }
            Err(ReadFailure::Work | ReadFailure::Bytes) => Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id: request.invoke_id,
                abort_reason: AbortReason::OUT_OF_RESOURCES,
            }),
        }
    }

    /// Who-Is and Who-Has. An unconfirmed request is never refused: one
    /// that doesn't decode, or that names nothing here, draws no answer.
    fn discover(
        &self,
        db: &ObjectDatabase,
        request: &UnconfirmedRequest,
        out: &mut BytesMut,
    ) -> Outcome {
        let service = request.service_choice;
        let mut parameters = BytesMut::new();
        let announcement = if service == UnconfirmedServiceChoice::WHO_IS {
            match discovery::who_is(&self.identity, db, &request.service_request) {
                Ok(Some(i_am)) => {
                    i_am.encode(&mut parameters);
                    UnconfirmedServiceChoice::I_AM
                }
                _ => return Outcome::Silent,
            }
        } else if service == UnconfirmedServiceChoice::WHO_HAS {
            let Some(device) = db.selected_device() else {
                return Outcome::Silent;
            };
            match discovery::who_has(db, &request.service_request, device) {
                Ok(Some(i_have)) if i_have.encode(&mut parameters).is_ok() => {
                    UnconfirmedServiceChoice::I_HAVE
                }
                _ => return Outcome::Silent,
            }
        } else {
            return Outcome::Unhandled;
        };
        let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: announcement,
            service_request: parameters.freeze(),
        });
        emit(out, &pdu, Outcome::Announce)
    }
}

#[cfg(test)]
#[path = "responder_tests.rs"]
mod tests;

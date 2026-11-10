//! [`Responder`]: the staged steps strung together for a host with nothing
//! to await between them, such as firmware that owns its database.
use crate::clock::{self, Basis, DeviceClock, Zone};
use crate::discovery;
use crate::read::{self, ReadFailure, ReadOverlay, ReadPropertyMultipleBudget, ReadScope, RpmPlan};
use crate::reply::{self, SEGMENTED, emit};
use crate::write::{self, Applied, WpmOutcome, WriteCommitObserver};
use alloc::vec::Vec;
use bacnet_encoding::apdu::{
    AbortPdu, Apdu, ComplexAck, ConfirmedRequest, RejectPdu, SimpleAck, UnconfirmedRequest,
    decode_apdu,
};
use bacnet_objects::DeviceIdentity;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::rpm::ReadPropertyMultipleRequest;
use bacnet_services::wpm::WritePropertyMultipleError;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, PduType, Segmentation, ServiceSupported,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::{Bytes, BytesMut};

/// The services a [`Responder`] runs, for the Device's
/// Protocol_Services_Supported: build the database from
/// `DeviceIdentity::with_services(SERVICES)`.
///
/// WriteProperty and WritePropertyMultiple are among them. A write to a
/// commandable Present_Value (the default Binary Value, Analog Value and
/// Analog Output among them) needs the requester, which the host gives with
/// [`Inbound::with_source`]; without it the object refuses the command with
/// WRITE_ACCESS_DENIED.
pub const SERVICES: &[ServiceSupported] = &[
    ServiceSupported::READ_PROPERTY,
    ServiceSupported::READ_PROPERTY_MULTIPLE,
    ServiceSupported::WRITE_PROPERTY,
    ServiceSupported::WRITE_PROPERTY_MULTIPLE,
    ServiceSupported::WHO_HAS,
    ServiceSupported::WHO_IS,
];

/// What a request needs from the database, read from its header alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Access {
    /// ReadProperty, ReadPropertyMultiple, Who-Is or Who-Has:
    /// [`Responder::read`] under a shared guard.
    Read,
    /// WriteProperty, WritePropertyMultiple, TimeSynchronization or
    /// UTCTimeSynchronization: [`Responder::write`] under an exclusive
    /// guard. A synchronization sets the clock and lends it to the database
    /// again, when the responder has a clock ([`Responder::with_clock`]).
    Write,
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
    /// Who sent the request, as a write commands an object for them
    /// (Clause 19.5). It is the host's own claim, trusted as given: nothing
    /// checks it against the database or the network. A request from the
    /// network takes [`CommandOrigin::Remote`], with the address it came
    /// from (network 0 for one on the local network) and the Device the
    /// host binds to that address, if any. [`CommandOrigin::Local`] names
    /// one of the host's own objects as the writer, and lets its owning
    /// Device correct the slot later; it is not for network requests. The
    /// type's docs in `bacnet_objects::command_source` give the rules.
    /// `None` when the host doesn't say; a commandable Present_Value then
    /// refuses the write with WRITE_ACCESS_DENIED.
    pub source: Option<CommandOrigin>,
}

impl Inbound {
    /// The request `apdu`, whose reply may be up to `max_reply` octets, from
    /// no named requester.
    pub fn new(apdu: Bytes, max_reply: u16) -> Self {
        Self {
            apdu,
            max_reply,
            source: None,
        }
    }

    /// This request, sent by `source`: the command origin its writes carry
    /// ([`Inbound::source`] says which kind to give). The host vouches for
    /// it; the object checks only its form ([`CommandOrigin::validate`]),
    /// so a malformed one is refused there.
    pub fn with_source(mut self, source: CommandOrigin) -> Self {
        self.source = Some(source);
        self
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

/// What the core did with a request that may write, and what the write
/// changed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Written {
    /// What the core did with the request.
    pub outcome: Outcome,
    /// The objects a write changed, for the host's post-write work (change
    /// reports, event evaluation, saving). A WritePropertyMultiple refused
    /// after some attempts succeeded keeps those writes, and names their
    /// objects here beside its Error. Empty when nothing changed: a
    /// WriteProperty refused, a request refused before any attempt
    /// succeeded, a NULL the property left as it was, or a request that
    /// writes nothing.
    pub objects: Vec<ObjectIdentifier>,
}

impl From<Outcome> for Written {
    /// `outcome`, with nothing written.
    fn from(outcome: Outcome) -> Self {
        Self {
            outcome,
            objects: Vec::new(),
        }
    }
}

/// The limits a [`Responder`] reads within.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Result rows one ReadProperty may expand: its own row plus, for a
    /// Group's whole Present_Value, every member row. A read past it is
    /// aborted with OUT_OF_RESOURCES.
    pub read_work: usize,
    /// What one ReadPropertyMultiple may expand and encode. Past its
    /// result rows the request is aborted with OUT_OF_RESOURCES, in
    /// planning, before any value is read; past its service-ACK octets,
    /// after the reads, with BUFFER_OVERFLOW.
    pub rpm: ReadPropertyMultipleBudget,
}

impl Limits {
    /// Limits of `read_work` result rows per ReadProperty, and the default
    /// [`ReadPropertyMultipleBudget`].
    pub fn new(read_work: usize) -> Self {
        Self {
            read_work,
            rpm: ReadPropertyMultipleBudget::default(),
        }
    }

    /// These limits, with `rpm` for ReadPropertyMultiple.
    pub fn with_rpm(mut self, rpm: ReadPropertyMultipleBudget) -> Self {
        self.rpm = rpm;
        self
    }
}

impl Default for Limits {
    /// The server's defaults: the default ReadPropertyMultiple budget, and
    /// its result-row limit for ReadProperty too.
    fn default() -> Self {
        Self::new(ReadPropertyMultipleBudget::default().max_result_elements)
    }
}

/// Answers ReadProperty, ReadPropertyMultiple, WriteProperty,
/// WritePropertyMultiple, Who-Is and Who-Has from a borrowed database, for
/// one Device, and with a clock ([`with_clock`](Self::with_clock)) runs
/// TimeSynchronization and UTCTimeSynchronization too.
///
/// It holds the Device's frozen identity: a Who-Is and a Who-Has are
/// answered for it, and only while the database holds its Device, and the
/// Device wildcard instance reads it, even after the object is gone.
#[derive(Clone, Debug)]
pub struct Responder {
    identity: DeviceIdentity,
    scope: ReadScope,
    rpm: ReadPropertyMultipleBudget,
    clock: Option<Clocked>,
}

/// The Device's clock, and the step that runs a synchronization on it:
/// [`clock::synchronize`], which only [`Responder::with_clock`] names, so
/// an image that never gives a responder a clock links none of it: 4.5 KB
/// of the `objects` firmware image's flash on thumbv7m.
#[derive(Clone, Debug)]
struct Clocked {
    clock: DeviceClock,
    synchronize: fn(
        &mut DeviceClock,
        &mut ObjectDatabase,
        ObjectIdentifier,
        UnconfirmedServiceChoice,
        &[u8],
    ) -> bool,
}

impl Responder {
    /// A responder for `identity`, within `limits`.
    ///
    /// It refuses an identity that claims segmentation support: the core
    /// aborts every segmented request and every reply too long for one APDU,
    /// so an I-Am naming segmentation would overstate it. Give it
    /// `Segmentation::NO_SEGMENTATION`, the [`DeviceIdentity`] default. It also refuses
    /// a `read_work` of zero, which no ReadProperty fits, and an RPM budget
    /// [`ReadPropertyMultipleBudget::validate`] refuses.
    pub fn new(identity: DeviceIdentity, limits: Limits) -> Result<Self, Error> {
        if identity.segmentation() != Segmentation::NO_SEGMENTATION {
            return Err(Error::Encoding(
                "bacnet-device-core segments neither way: the Device identity must claim no segmentation"
                    .into(),
            ));
        }
        if limits.read_work == 0 {
            return Err(Error::Encoding("read_work must be positive".into()));
        }
        limits.rpm.validate()?;
        let scope = ReadScope {
            work_limit: limits.read_work,
            registered_port: None,
            device: Some(identity.device_oid()),
        };
        Ok(Self {
            identity,
            scope,
            rpm: limits.rpm,
            clock: None,
        })
    }

    /// This responder with `clock` as the Device's clock, lent to `db` (the
    /// database it answers from) now: the Device then serves Local_Date and
    /// Local_Time from it, and lists TimeSynchronization and
    /// UTCTimeSynchronization in Protocol_Services_Supported when its
    /// identity names them ([`clock::SERVICES`]). [`write`](Self::write)
    /// runs each one the Device lists, setting the clock and lending it to
    /// the database again; one the Device doesn't list, or whose date and
    /// time aren't specific, changes nothing. See [`clock`] for how each
    /// sets it.
    pub fn with_clock(mut self, clock: DeviceClock, db: &mut ObjectDatabase) -> Self {
        clock.lend(db);
        self.clock = Some(Clocked {
            clock,
            synchronize: clock::synchronize,
        });
        self
    }

    /// The Device's clock, if it has one.
    pub fn clock(&self) -> Option<&DeviceClock> {
        self.clock.as_ref().map(|clocked| &clocked.clock)
    }

    /// Set the Device's clock to `date` and `time` on `basis`, as a
    /// real-time clock on the board does, and lend it to `db` again. It
    /// counts as synchronized. Refused, changing nothing, when the
    /// responder has no clock or [`SyncedClock::set`](clock::SyncedClock::set)
    /// refuses the value.
    pub fn set_time(
        &mut self,
        db: &mut ObjectDatabase,
        basis: Basis,
        date: Date,
        time: Time,
    ) -> Result<(), Error> {
        let clock = self.clock_mut()?;
        clock.set(basis, date, time)?;
        clock.lend(db);
        Ok(())
    }

    /// Move the Device's clock to `zone`, when daylight saving starts or
    /// ends or the host corrects its UTC_Offset, and lend it to `db` again:
    /// the Device's UTC_Offset, Daylight_Savings_Status, Local_Date and
    /// Local_Time, and every object's clock, read the new zone from this
    /// call on. A synchronized clock stays synchronized
    /// ([`SyncedClock::set_zone`](clock::SyncedClock::set_zone)). Refused,
    /// changing nothing, when the responder has no clock or the clock
    /// refuses the zone.
    pub fn set_zone(&mut self, db: &mut ObjectDatabase, zone: Zone) -> Result<(), Error> {
        let clock = self.clock_mut()?;
        clock.set_zone(zone)?;
        clock.lend(db);
        Ok(())
    }

    /// The Device's clock, to set.
    fn clock_mut(&mut self) -> Result<&mut DeviceClock, Error> {
        self.clock
            .as_mut()
            .map(|clocked| &mut clocked.clock)
            .ok_or(Error::range("the responder has no clock"))
    }

    /// The Device identity this responder answers for.
    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
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
            if first & SEGMENTED == 0 {
                let service = apdu.get(3).copied().map(ConfirmedServiceChoice::from_raw);
                match service {
                    Some(
                        ConfirmedServiceChoice::READ_PROPERTY
                        | ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                    ) => return Access::Read,
                    Some(
                        ConfirmedServiceChoice::WRITE_PROPERTY
                        | ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                    ) => return Access::Write,
                    _ => {}
                }
            }
        } else if pdu_type == PduType::UNCONFIRMED_REQUEST.to_raw() {
            match apdu.get(1).copied().map(UnconfirmedServiceChoice::from_raw) {
                Some(UnconfirmedServiceChoice::WHO_IS | UnconfirmedServiceChoice::WHO_HAS) => {
                    return Access::Read;
                }
                Some(
                    UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
                    | UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
                ) => return Access::Write,
                _ => {}
            }
        }
        Access::None
    }

    /// Run `request` if it is a ReadProperty, ReadPropertyMultiple, Who-Is or
    /// Who-Has, writing any answer to `out`; `overlay` serves objects as the
    /// host does. A confirmed request that arrived segmented is refused. Any
    /// other service, WritePropertyMultiple included, is
    /// [`Outcome::Unhandled`], for the host to route on.
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
            Ok(Apdu::ConfirmedRequest(confirmed))
                if confirmed.service_choice == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE =>
            {
                let answer = self.read_property_multiple(db, overlay, &confirmed);
                emit(out, &reply::fit(answer, request.max_reply), Outcome::Reply)
            }
            Ok(Apdu::UnconfirmedRequest(unconfirmed)) => self.discover(db, &unconfirmed, out),
            Ok(_) => Outcome::Unhandled,
            // Neither a type nor an invoke ID to answer.
            Err(_) => Outcome::Silent,
        }
    }

    /// Run `request` if it is a WriteProperty or a WritePropertyMultiple,
    /// writing the SimpleACK, or the Error or Reject that refuses it, to
    /// `out`; or, with a clock, a TimeSynchronization or
    /// UTCTimeSynchronization, which is [`Outcome::Silent`] whatever it
    /// does ([`with_clock`](Self::with_clock)). `observer` sees each
    /// attempt, as the server's audit and change capture do. A
    /// WritePropertyMultiple keeps the attempts that succeeded
    /// before one failed, and its Error names the one that failed. A write
    /// that queues a Command's or Channel's run ends that run at once, unmade:
    /// the core has no task to make its writes in, so it performs none of
    /// the run's actions, and the object reports them unsuccessful. A
    /// confirmed request that arrived segmented is refused. Any other
    /// service is [`Outcome::Unhandled`], for the host to route on.
    ///
    /// Each write carries the request's [`Inbound::source`] as its command
    /// origin, as the server's carries the requester it derives from the
    /// source address, so a commandable Present_Value is commanded for that
    /// requester at the request's priority. A request with no source has no
    /// origin to give: an object whose Present_Value is commandable (the
    /// default Binary Value, Analog Value and Analog Output among them)
    /// refuses a write to it with WRITE_ACCESS_DENIED.
    pub fn write(
        &mut self,
        db: &mut ObjectDatabase,
        observer: Option<&mut dyn WriteCommitObserver>,
        request: Inbound,
        out: &mut BytesMut,
    ) -> Written {
        let origin = request.source.as_ref();
        let confirmed = match decode_apdu(request.apdu) {
            Ok(Apdu::ConfirmedRequest(confirmed)) if confirmed.segmented => {
                let refused = reply::refuse_segmented(confirmed.invoke_id);
                return emit(out, &refused, Outcome::Reply).into();
            }
            Ok(Apdu::ConfirmedRequest(confirmed))
                if confirmed.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY =>
            {
                confirmed
            }
            Ok(Apdu::ConfirmedRequest(confirmed))
                if confirmed.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE =>
            {
                return Self::write_property_multiple(db, observer, origin, &confirmed, out);
            }
            Ok(Apdu::UnconfirmedRequest(request)) => {
                let device = self.identity.device_oid();
                let ran = self.clock.as_mut().is_some_and(|clocked| {
                    let service = request.service_choice;
                    let parameters = &request.service_request;
                    (clocked.synchronize)(&mut clocked.clock, db, device, service, parameters)
                });
                return if ran {
                    Outcome::Silent
                } else {
                    Outcome::Unhandled
                }
                .into();
            }
            Ok(_) => return Outcome::Unhandled.into(),
            // Neither a type nor an invoke ID to answer.
            Err(_) => return Outcome::Silent.into(),
        };
        let mut objects = Vec::new();
        let answer =
            match write::write_property(db, &confirmed.service_request, observer, None, origin) {
                Ok((oid, applied)) => {
                    if applied == Applied::Written {
                        write::end_unmade(db, &[oid]);
                        objects.push(oid);
                    }
                    Apdu::SimpleAck(SimpleAck {
                        invoke_id: confirmed.invoke_id,
                        service_choice: confirmed.service_choice,
                    })
                }
                Err(error) => {
                    reply::error_apdu(confirmed.invoke_id, confirmed.service_choice, &error)
                }
            };
        let outcome = emit(out, &answer, Outcome::Reply);
        Written { outcome, objects }
    }

    /// For firmware that owns its database: [`classify`](Self::classify),
    /// then [`write`](Self::write) with no observer or [`read`](Self::read)
    /// with no overlay, and every confirmed request the core doesn't run
    /// answered by [`reply::refuse`]. An unconfirmed request it doesn't run
    /// is [`Outcome::Silent`], and so is an APDU that doesn't decode, which
    /// the server drops too.
    ///
    /// Its writes are [`write`](Self::write)'s: commanded for the request's
    /// [`Inbound::source`] (a commandable Present_Value refuses a request
    /// with none), and with any Command or Channel run they start ended
    /// unmade. With a clock it runs the time synchronizations the Device
    /// lists, too.
    pub fn handle(
        &mut self,
        db: &mut ObjectDatabase,
        request: Inbound,
        out: &mut BytesMut,
    ) -> Written {
        let apdu = request.apdu.clone();
        let handled = if Self::classify(&apdu) == Access::Write {
            self.write(db, None, request, out)
        } else {
            self.read(db, None::<&read::NoOverlay>, request, out).into()
        };
        // `read` and `write` decode every APDU, so only a well-formed
        // request neither runs reaches the refusal.
        match handled.outcome {
            Outcome::Unhandled => reply::refuse(&apdu, out).into(),
            _ => handled,
        }
    }

    /// WritePropertyMultiple, as the server runs it, with no gate or audit
    /// source, commanded for `origin`: the objects its successful attempts changed, and any
    /// run they queued ended unmade, even when a later attempt failed.
    fn write_property_multiple(
        db: &mut ObjectDatabase,
        observer: Option<&mut dyn WriteCommitObserver>,
        origin: Option<&CommandOrigin>,
        request: &ConfirmedRequest,
        out: &mut BytesMut,
    ) -> Written {
        let invoke_id = request.invoke_id;
        let (answer, objects) = match write::write_property_multiple(
            db,
            &request.service_request,
            None,
            observer,
            None,
            origin,
        ) {
            WpmOutcome::Success { committed_oids } => (
                Apdu::SimpleAck(SimpleAck {
                    invoke_id,
                    service_choice: request.service_choice,
                }),
                committed_oids,
            ),
            WpmOutcome::Error {
                error,
                first_failed_write_attempt,
                committed_oids,
            } => {
                let (error_class, error_code) = reply::error_fields(&error);
                let refused = WritePropertyMultipleError {
                    error_class,
                    error_code,
                    first_failed_write_attempt,
                };
                (Apdu::Error(refused.to_error_pdu(invoke_id)), committed_oids)
            }
            WpmOutcome::Reject { reason } => (
                Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: reason,
                }),
                Vec::new(),
            ),
        };
        write::end_unmade(db, &objects);
        let outcome = emit(out, &answer, Outcome::Reply);
        Written { outcome, objects }
    }

    /// ReadPropertyMultiple, as the server runs it within its budget: a
    /// request that doesn't decode is rejected, one past the result-row
    /// limit is aborted with OUT_OF_RESOURCES before any value is read, and
    /// one whose ACK passes the octet limit is aborted with BUFFER_OVERFLOW.
    fn read_property_multiple<O: ReadOverlay>(
        &self,
        db: &ObjectDatabase,
        overlay: Option<&O>,
        request: &ConfirmedRequest,
    ) -> Apdu {
        let mut service_ack = BytesMut::new();
        let result = ReadPropertyMultipleRequest::decode(&request.service_request)
            .map_err(|error| ReadFailure::Service(Error::into_request_reject(error)))
            .and_then(|decoded| {
                let plan = RpmPlan::new(
                    db,
                    &self.scope,
                    overlay,
                    &decoded,
                    self.rpm.max_result_elements,
                )?;
                plan.read(
                    db,
                    overlay,
                    &mut service_ack,
                    self.rpm.max_service_ack_bytes,
                    |_, _, _, _| {},
                )
            });
        Self::read_answer(request, result, service_ack)
    }

    /// The reply to a read: its ComplexACK, the Error or Reject of a service
    /// error, or the Abort of a limit.
    fn read_answer(
        request: &ConfirmedRequest,
        result: Result<(), ReadFailure>,
        service_ack: BytesMut,
    ) -> Apdu {
        let abort = |abort_reason| {
            Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id: request.invoke_id,
                abort_reason,
            })
        };
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
            Err(ReadFailure::Bytes) => abort(AbortReason::BUFFER_OVERFLOW),
            Err(ReadFailure::Work) => abort(AbortReason::OUT_OF_RESOURCES),
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
        Self::read_answer(request, result, service_ack)
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
            match discovery::who_has(&self.identity, db, &request.service_request) {
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

#[cfg(test)]
#[path = "responder_write_tests.rs"]
mod write_tests;

#[cfg(test)]
#[path = "responder_multiple_tests.rs"]
mod multiple_tests;

#[cfg(test)]
#[path = "responder_legacy_tests.rs"]
mod legacy_tests;

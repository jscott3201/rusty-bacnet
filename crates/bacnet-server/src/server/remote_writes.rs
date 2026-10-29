//! Writes this server makes in other devices on behalf of its own objects: a
//! Command action naming another Device (Clause 12.10.8, #1180), or a Channel
//! member in another device (Clause 12.53.11, #1264). A Channel also reads
//! such a member's property first, to learn the datatype it coerces its value
//! to (#1342); that ReadProperty takes the same path, asks for an unsegmented
//! answer and takes only that, and everything below about a write holds for
//! it too.
//!
//! Each write goes out as one unsegmented confirmed WriteProperty. The address
//! comes from the server's device bindings: a configured binding, or an I-Am
//! heard in the last ten minutes. A device with neither is looked for first
//! (#1322): one Who-Is limited to its instance, then a wait of the APDU
//! timeout, a minute at most, from the send, for its I-Am. The Who-Is goes to
//! every network for a device never heard from, or to the network the
//! device's stale observation names, which a Who-Is that draws nothing
//! drops. A write that misses while that Who-Is is out waits on it instead
//! of sending another, and a device gets at most one Who-Is a minute
//! (`binding_probes`), so a write that misses within a minute of a Who-Is
//! that drew nothing fails at once. A write that ends with no binding sends no WriteProperty. A binding
//! routed through the network numbered as this device's own, once that number
//! is known, names a device on this network: the write goes to its MAC with
//! no DNET, not through the router (#1358), since a non-routing device drops
//! an NPDU whose DNET names a network (Clause 6.5.2.1).
//!
//! The invoke ID is leased from the same device-wide pool as confirmed
//! notifications (and, in an endpoint, client requests), so no two
//! outstanding transactions share one, and the answer comes back through the
//! notifications' dispatch fast path. The lease carries the peer and the
//! WriteProperty service: only that peer's answer ends it, and a SimpleACK or
//! Error only when it names WriteProperty, so a late answer to a notification
//! that once held the same invoke ID can't complete the write.
//!
//! Each attempt waits `cov_retry_timeout_ms` for an answer, and only silence
//! earns another attempt, up to the server's three retries; an Error (BUSY
//! included), Reject or Abort is final, and the caller gets what it said
//! (#1323): the Error's class and code, or the Reject or Abort reason.
//! Nothing is sent while DeviceCommunicationControl restricts initiation, a
//! Who-Is included, since Clause 16.1 lets such a device start no request but
//! I-Am answers and audit notifications: a retry it would block, or one whose
//! observed binding has lapsed, ends the write there and frees its invoke ID.
//! The caller holds no database guard while a write is outstanding or waits
//! for an I-Am, and the binding table's guard is never held across a send or
//! a wait.

use super::binding_probes::{DeviceLookup, LookupMiss, LookupStart, can_look_for};
use super::device_bindings::{DeviceBindingTable, DeviceResolution};
use super::event_recipient_route::{
    ConfirmedRecipientRoute, ConfirmedRouteRefusal, RecipientRoute,
};
use super::notification_transactions::{
    Attempt, AttemptsEnd, NotificationReserveError, run_attempts,
};
use super::*;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetActionCommand;
use std::fmt;

/// Octets of an unsegmented Confirmed-Request header: type, segmentation
/// limits, invoke ID and service choice.
const CONFIRMED_HEADER_LEN: usize = 4;

const WRITE_PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;
const READ_PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// Why a write, or a read, in another device got no answer it could use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteRequestError {
    /// DeviceCommunicationControl restricts initiation: nothing was sent, or
    /// the write ended at the first retry after it took effect.
    Disabled,
    /// No usable binding for the device, and no Who-Is sent for it: the
    /// identifier can't be looked for, a Who-Is for it drew nothing within
    /// the last minute, too many devices are being looked for, or an observed
    /// binding lapsed before a retry.
    Unbound,
    /// No binding for the device, and the Who-Is sent for it drew no I-Am
    /// within the APDU timeout.
    Undiscovered,
    /// The value has no encoding.
    Unencodable,
    /// The request doesn't fit in one APDU this device can send.
    TooLong,
    /// Every invoke ID is leased to another outstanding transaction.
    NoInvokeId,
    /// The server is stopping.
    Stopping,
    /// The device answered with an Error, Reject or Abort, saying this.
    Refused(RemoteError),
    /// No answer to the first attempt or any retry.
    Unanswered,
    /// The runner has no network to send on: a run made without a server.
    NoNetwork,
    /// A read's answer doesn't decode, or names another property than the
    /// one asked for.
    Malformed,
}

impl fmt::Display for RemoteRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::Disabled => "initiation is restricted by DeviceCommunicationControl",
            Self::Unbound => "no usable binding for the device",
            Self::Undiscovered => "the device didn't answer a Who-Is",
            Self::Unencodable => "the value has no encoding",
            Self::TooLong => "the request is longer than one APDU",
            Self::NoInvokeId => "no invoke ID is free",
            Self::Stopping => "the server is stopping",
            Self::Refused(refusal) => {
                let answer = Error::from(*refusal);
                return write!(formatter, "the device refused the request: {answer}");
            }
            Self::Unanswered => "the device didn't answer",
            Self::NoNetwork => "no network to send the request on",
            Self::Malformed => "the device's answer doesn't fit the read",
        };
        formatter.write_str(reason)
    }
}

/// One WriteProperty to send to another device.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemoteWrite {
    /// The Device that holds the target object.
    pub(crate) device: ObjectIdentifier,
    /// The request, its value already encoded.
    pub(crate) request: WritePropertyRequest,
}

impl RemoteWrite {
    /// The write `command` makes in `device`.
    pub(crate) fn for_command(
        device: ObjectIdentifier,
        command: &BACnetActionCommand,
    ) -> Result<Self, RemoteRequestError> {
        let mut value = BytesMut::new();
        encode_property_value(&mut value, &command.property_value)
            .map_err(|_| RemoteRequestError::Unencodable)?;
        Ok(Self {
            device,
            request: WritePropertyRequest {
                object_identifier: command.object_identifier,
                property_identifier: command.property_identifier,
                property_array_index: command.property_array_index,
                property_value: value.to_vec(),
                priority: command.priority,
            },
        })
    }
}

/// The server handles a write in another device needs.
pub(super) struct RemoteWriter<'a, T: TransportPort + 'static> {
    pub(super) network: &'a Arc<NetworkLayer<T>>,
    pub(super) transactions: &'a Arc<NotificationTransactions>,
    pub(super) bindings: &'a RwLock<DeviceBindingTable>,
    pub(super) comm_state: &'a CommState,
    /// How long each attempt waits for the answer.
    pub(super) timeout: Duration,
    /// Attempts after the first, each earned by silence.
    pub(super) retries: u8,
    /// The longest APDU this device sends or takes.
    pub(super) max_apdu: u32,
}

impl<T: TransportPort + 'static> RemoteWriter<'_, T> {
    /// Send `write` and wait for the device's answer.
    pub(super) async fn write(&self, write: &RemoteWrite) -> Result<(), RemoteRequestError> {
        let mut service = BytesMut::new();
        write
            .request
            .encode(&mut service)
            .map_err(|_| RemoteRequestError::Unencodable)?;
        // A write's lease admits no ComplexAck, so any answer taken is a
        // SimpleAck.
        self.request(write.device, WRITE_PROPERTY, service)
            .await
            .map(drop)
    }

    /// Read `request`'s property in `device` and return the value the device
    /// answers with, decoded. An answer naming another property, or one that
    /// doesn't decode, is [`RemoteRequestError::Malformed`].
    pub(super) async fn read(
        &self,
        device: ObjectIdentifier,
        request: &ReadPropertyRequest,
    ) -> Result<PropertyValue, RemoteRequestError> {
        let mut service = BytesMut::new();
        request.encode(&mut service);
        let Some(data) = self.request(device, READ_PROPERTY, service).await? else {
            return Err(RemoteRequestError::Malformed);
        };
        let ack = ReadPropertyACK::decode(&data).map_err(|_| RemoteRequestError::Malformed)?;
        if (
            ack.object_identifier,
            ack.property_identifier,
            ack.property_array_index,
        ) != (
            request.object_identifier,
            request.property_identifier,
            request.property_array_index,
        ) {
            return Err(RemoteRequestError::Malformed);
        }
        decode_value(&ack.property_value).ok_or(RemoteRequestError::Malformed)
    }

    /// Send one confirmed request for `service`, whose encoded parameters are
    /// `parameters`, to `device`, and wait for its answer: `None` for a
    /// SimpleAck, the service data of a ComplexAck to a ReadProperty.
    async fn request(
        &self,
        device: ObjectIdentifier,
        service_choice: ConfirmedServiceChoice,
        parameters: BytesMut,
    ) -> Result<Option<Bytes>, RemoteRequestError> {
        if self.initiation_restricted() {
            return Err(RemoteRequestError::Disabled);
        }
        let route = self.route(device).await?;
        // Checked before an invoke ID is leased.
        let capacity = usize::try_from(self.max_apdu).unwrap_or(usize::MAX);
        if CONFIRMED_HEADER_LEN + parameters.len() > capacity {
            return Err(RemoteRequestError::TooLong);
        }
        let peer = route.canonical_peer.clone();
        let reserved = if service_choice == READ_PROPERTY {
            self.transactions.reserve_read(peer, service_choice)
        } else {
            self.transactions.reserve(peer, service_choice)
        };
        let (operation, answer) = match reserved {
            Ok(reservation) => reservation,
            Err(NotificationReserveError::Closed) => return Err(RemoteRequestError::Stopping),
            Err(_) => return Err(RemoteRequestError::NoInvokeId),
        };
        let invoke_id = operation.invoke_id();
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: apdu::max_apdu_header_at_or_below(self.max_apdu)
                    .expect("validated local APDU capacity"),
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_request: parameters.freeze(),
            }),
        )
        .expect("valid APDU encoding");
        let (apdu, route, network) = (&apdu, &route, self.network);
        let outcome = run_attempts(operation, answer, self.timeout, self.retries, |attempt| {
            // A retry is a send too: DCC and the binding's lifetime are
            // checked again before each one, and either ends the write.
            let withdrawn = if self.initiation_restricted() {
                Some(RemoteRequestError::Disabled)
            } else if route
                .freshness
                .is_some_and(|freshness| !freshness.permits_attempt_at(tokio::time::Instant::now()))
            {
                Some(RemoteRequestError::Unbound)
            } else {
                None
            };
            async move {
                if let Some(reason) = withdrawn {
                    return Attempt::Withdrawn(reason);
                }
                match send(network, apdu, route, invoke_id, attempt).await {
                    Ok(()) => Attempt::Sent,
                    Err(()) => Attempt::NotSent,
                }
            }
        })
        .await?;
        match outcome {
            AttemptsEnd::Answered(CovAckResult::Ack) => Ok(None),
            AttemptsEnd::Answered(CovAckResult::Data(data)) => Ok(Some(data)),
            AttemptsEnd::Answered(CovAckResult::Error(refusal)) => {
                Err(RemoteRequestError::Refused(refusal))
            }
            AttemptsEnd::Exhausted => Err(RemoteRequestError::Unanswered),
            AttemptsEnd::Closed => Err(RemoteRequestError::Stopping),
        }
    }

    /// The route to `device`: its binding, or the one a targeted Who-Is finds
    /// when it has none fresh.
    async fn route(
        &self,
        device: ObjectIdentifier,
    ) -> Result<ConfirmedRecipientRoute, RemoteRequestError> {
        let lookup = DeviceLookup {
            network: self.network,
            bindings: self.bindings,
            comm_state: self.comm_state,
            wait: self.timeout,
        };
        let resolution = lookup.resolve(device).await;
        if !can_look_for(device, &resolution) {
            return self.confirmed(resolution, RemoteRequestError::Unbound);
        }
        let wait = match lookup.start(device).await {
            LookupStart::Resolved(resolution) => {
                return self.confirmed(resolution, RemoteRequestError::Unbound);
            }
            LookupStart::Waiting(wait) => wait,
            LookupStart::NotLooking => return Err(RemoteRequestError::Unbound),
            LookupStart::Disabled => return Err(RemoteRequestError::Disabled),
        };
        // Woken by the I-Am or the deadline, the write looks again either way.
        wait.answered().await;
        match lookup.found(device).await {
            Ok(resolution) => self.confirmed(resolution, RemoteRequestError::Undiscovered),
            Err(LookupMiss::Disabled) => Err(RemoteRequestError::Disabled),
            Err(LookupMiss::Undiscovered) => Err(RemoteRequestError::Undiscovered),
        }
    }

    /// Whether `mac` reaches a group of nodes on this link: no binding takes
    /// one, and no request goes to one (#1493).
    fn is_group(&self, mac: &[u8]) -> bool {
        self.network.transport().is_group_destination(mac)
    }

    /// The confirmed route `resolution` gives, or `missing` when it gives
    /// none. A binding routed through this network's own number, read now,
    /// is the local device it is (#1358): the write goes straight to its MAC
    /// with no DNET and is answered from there, and one at a group address,
    /// the link's broadcast MAC or another, names no device, so it gives no
    /// route (#1493).
    fn confirmed(
        &self,
        resolution: DeviceResolution,
        missing: RemoteRequestError,
    ) -> Result<ConfirmedRecipientRoute, RemoteRequestError> {
        let is_group = |mac: &[u8]| self.is_group(mac);
        RecipientRoute::from_device_resolution(resolution)
            .localize(
                self.network.local_network_number().get(),
                is_group,
                is_group,
            )
            .into_confirmed(is_group)
            .map_err(|refusal| {
                if refusal == ConfirmedRouteRefusal::GroupNextHop {
                    warn!("No request sent to another device at a group address");
                }
                missing
            })
    }

    fn initiation_restricted(&self) -> bool {
        self.comm_state.initiation_restricted()
    }
}

/// One attempt: the request to the bound peer, a device on this network
/// included, or through the binding's router to a device on another network.
async fn send<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    apdu: &[u8],
    route: &ConfirmedRecipientRoute,
    invoke_id: u8,
    attempt: u8,
) -> Result<(), ()> {
    let priority = NetworkPriority::Normal;
    let sent = match (&route.local_target, &route.remote) {
        (Some(mac), None) => network.send_apdu(apdu, mac, true, priority).await,
        (None, Some((dnet, dadr, Some(router)))) => {
            network
                .send_apdu_routed(apdu, *dnet, dadr, router, true, priority)
                .await
        }
        // A Device binding always names its next hop.
        _ => return Err(()),
    };
    match &sent {
        Ok(()) => debug!(invoke_id, attempt, "Request to another device sent"),
        Err(error) => warn!(
            %error,
            invoke_id, attempt, "Request to another device not sent"
        ),
    }
    sent.map_err(|_| ())
}

/// A ReadProperty-ACK's value: one application-tagged primitive, or `None`
/// for anything else, which carries no datatype a Channel can coerce to.
fn decode_value(encoded: &[u8]) -> Option<PropertyValue> {
    match bacnet_encoding::primitives::decode_application_value(encoded, 0) {
        Ok((value, end)) if end == encoded.len() => Some(value),
        _ => None,
    }
}

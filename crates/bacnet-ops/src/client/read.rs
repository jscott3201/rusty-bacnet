//! The reading strategy, one confirmed request at a time per device.
//!
//! - Properties: one ReadPropertyMultiple, with ALL for an object or an
//!   explicit list for the device card (Clause 15.7). When the device
//!   refuses RPM, ReadProperty each property instead (Clause 15.5), and
//!   remember the refusal for later reads of that device. For ALL, the
//!   object's Property_List names the properties; it leaves out the four
//!   every object has, which are added back (Clause 12.1.1.4.1).
//! - Object_List: one ReadProperty of the whole array. When the device
//!   can't return it in one answer (it would need segmentation one side
//!   lacks, say), read element 0, which holds the length, and then each
//!   element in turn (Clause 12.11.16).

use std::sync::atomic::{AtomicBool, Ordering};

use bacnet_client::client::{BACnetClient, Station};
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{AbortReason, ErrorClass, ObjectType, PropertyIdentifier, RejectReason};
use bacnet_types::error::{Error, RangeError, RemoteError, SegmentationError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::device::{DeviceAddress, path_to};
use crate::format::{PropertyError, RawRow, decode_all};
use crate::limits::{RequestLimits, Turn};
use crate::target::AddressStyle;

/// How a property list was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadVia {
    /// ReadPropertyMultiple.
    Rpm,
    /// ReadProperty per property: the device refused RPM, or an answer
    /// would have been too large for it.
    Rp,
    /// A bulk read ([`Device::read_many`](crate::Device::read_many)) only:
    /// some of its objects by ReadPropertyMultiple, others by ReadProperty,
    /// because the device failed one batch's RPM as a whole, or a single
    /// object's answer was too large for one RPM, or it refused RPM part
    /// way through the read.
    Mixed,
}

/// Elements a device's Object_List may claim before the browser stops
/// reading it; a device holds far fewer.
pub const MAX_OBJECTS: u32 = 100_000;

/// Properties read when an object's Property_List can't be: the ones the
/// browser shows most.
const FALLBACK_PROPERTIES: [PropertyIdentifier; 9] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::DESCRIPTION,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::EVENT_STATE,
    PropertyIdentifier::RELIABILITY,
    PropertyIdentifier::OUT_OF_SERVICE,
    PropertyIdentifier::UNITS,
    PropertyIdentifier::PRIORITY_ARRAY,
    PropertyIdentifier::RELINQUISH_DEFAULT,
];

/// Why a read stopped.
#[derive(Debug)]
pub enum Halt {
    /// The job was stopped between requests.
    Cancelled,
    /// The device, or the way to it, failed the request.
    Device(Error),
}

/// What [`Reader::properties`] reads.
#[derive(Debug, Clone, Copy)]
pub enum Wanted<'a> {
    /// Every property (ALL, or Property_List over ReadProperty).
    All,
    /// These properties.
    List(&'a [PropertyIdentifier]),
}

/// What an RPM failure says about trying ReadProperty instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpmFailure {
    /// The device doesn't take RPM: a reject (an unknown service, say) or a
    /// services-class error. Remembered for the rest of the session.
    Refused,
    /// This answer was too large to send (an abort for segmentation or a
    /// full buffer), or the request too long for the link or the routed
    /// path: ReadProperty for this request only.
    TooLarge,
    /// Anything else, a timeout included: the read fails.
    Other,
}

/// Classify an RPM failure.
pub fn rpm_failure(error: &Error) -> RpmFailure {
    let too_large = [
        AbortReason::SEGMENTATION_NOT_SUPPORTED,
        AbortReason::BUFFER_OVERFLOW,
        AbortReason::APDU_TOO_LONG,
    ];
    match error {
        Error::Remote(RemoteError::Reject { reason })
            if *reason == RejectReason::BUFFER_OVERFLOW =>
        {
            RpmFailure::TooLarge
        }
        Error::Remote(RemoteError::Reject { .. }) => RpmFailure::Refused,
        Error::Remote(RemoteError::Error { class, .. }) if *class == ErrorClass::SERVICES => {
            RpmFailure::Refused
        }
        Error::Remote(RemoteError::Abort { reason }) if too_large.contains(reason) => {
            RpmFailure::TooLarge
        }
        // The request itself was too long, for the link or for a router on
        // the way (a router that refuses a frame above what its path carries;
        // a frame of 228 octets or fewer never earns this, #2665): one
        // property at a time is shorter.
        Error::SendTooLong { .. } | Error::RoutedPathTooLong { .. } => RpmFailure::TooLarge,
        // The request needs segments the client may not send (a node whose
        // Device doesn't transmit them, #2617), or that the device can't
        // take, or not so many: refused before it went out, and one
        // property at a time may fit one APDU.
        Error::Segmentation(
            SegmentationError::LocalCannotTransmit
            | SegmentationError::PeerCannotReceive { .. }
            | SegmentationError::PeerSegmentLimit { .. }
            | SegmentationError::TooManySegments { .. },
        ) => RpmFailure::TooLarge,
        _ => RpmFailure::Other,
    }
}

/// True when `error`, a low-level client's error such as a [`Halt`]
/// holds, says the device, or the way to it, isn't answering: a timeout, a
/// transport failure, or a router on the way that refused the request at
/// the network layer ([`Error::NetworkRejected`], #2724). Reading on would
/// only fail the same way. A timeout or a router's refusal makes a
/// following handle look for its device elsewhere
/// ([`Device`](crate::Device)). The high-level calls' errors say it with
/// [`OpsError::unreachable`](crate::OpsError::unreachable).
pub fn unreachable(error: &Error) -> bool {
    timed_out(error) || matches!(error, Error::Transport(_) | Error::NetworkRejected(_))
}

/// True when `error` is the client's or the link's own failure of a
/// request, not the device's answer and not a wait for one: nothing a
/// property caused, so the next request would fail the same way, and a
/// read fails with it once where it used to become a row of text for
/// every property (#2610). It covers exactly:
///
/// - [`Error::UnknownDevice`]: no route to the address's network. Nothing
///   was sent.
/// - [`Error::UnsupportedTransport`] and [`Error::Encode`]: the link can't
///   carry the request, or it can't be encoded. Nothing was sent.
/// - [`Error::RoutedPathCapacityExceeded`]: the client has no room for
///   another routed path. Nothing was sent.
/// - [`Error::InitiationDisabled`]: the node the client is seated on has
///   had initiation turned off by DeviceCommunicationControl (#2617).
///   Nothing was sent, or, with `sent`, a request that went out had its
///   retry withheld; either way no word of the device's, and the next
///   request fails the same way until the node is enabled again.
/// - The local refusals (#1833): [`Error::State`] (a
///   transport not started or stopped, a router-application client's
///   router closed, a BACnet/SC link that isn't connected to its hub, a
///   closed loopback channel, a client seat that has closed),
///   [`Error::InvalidConfig`], [`Error::InvalidArgument`] (a MAC of the
///   wrong form for the link, or a router's application given a station
///   that names no network), [`Error::Unsupported`] and
///   [`Error::LimitReached`] (a client out of invoke IDs). Each is the
///   client's or the link's own refusal: a stopped local link is not a
///   device that stopped answering, so [`unreachable()`] doesn't count it.
/// - [`Error::RequestCutOff`]: the client or its link stopped under the
///   request before an answer came (the client's own stop, its node's, or
///   the link's after the request went out). With `sent: true` it **may
///   already have left**, and the device may have carried it out.
/// - [`Error::Persistence`]: this side's own stored state failed to load or
///   save (#1833). No answer of the device's.
///
/// So a failure here is not "never sent", and nothing may send again on the
/// strength of one: reads only stop on it, since the client, not the
/// property, failed.
///
/// A request too long for the link or the routed path
/// ([`Error::SendTooLong`], [`Error::RoutedPathTooLong`]), or one that
/// needs segments the client or the device can't use
/// ([`Error::Segmentation`]), is not one: a smaller request may pass, so
/// those fall back as an answer too large does ([`rpm_failure`]).
pub fn failed_locally(error: &Error) -> bool {
    matches!(
        error,
        Error::State(..)
            | Error::InvalidConfig(_)
            | Error::InvalidArgument(_)
            | Error::Unsupported(_)
            | Error::LimitReached(_)
            | Error::Encode(_)
            | Error::UnknownDevice { .. }
            | Error::UnsupportedTransport { .. }
            | Error::RoutedPathCapacityExceeded { .. }
            | Error::InitiationDisabled { .. }
            | Error::RequestCutOff { .. }
            | Error::Persistence(_)
    )
}

/// True when `error`, a low-level client's error, says the request timed
/// out: the client's own timeout, or the transaction state machine giving
/// up on its retries, also after a router said it was busy
/// ([`Error::RouterBusyTimeout`], #2724). A transport failure, or a
/// router's network reject, is unreachable but not a timeout. The high-level calls' errors say it with
/// [`OpsError::timed_out`](crate::OpsError::timed_out).
pub fn timed_out(error: &Error) -> bool {
    match error {
        Error::Remote(RemoteError::Abort { reason }) => *reason == AbortReason::TSM_TIMEOUT,
        Error::Timeout(_) | Error::RouterBusyTimeout(_) => true,
        _ => false,
    }
}

/// How an Object_List read went.
#[derive(Debug)]
pub struct ListRead {
    /// The entries read, in device order.
    pub objects: Vec<ObjectIdentifier>,
    /// It took element-by-element reads.
    pub indexed: bool,
    /// An element-by-element read stopped early, and why: `objects` holds
    /// the elements before it.
    pub stopped: Option<String>,
}

/// Reads one device, one request at a time, each with its turn at the
/// device's gate and the global rate cap.
///
/// Every request goes to the device's address: to its MAC on this network,
/// or to its station behind a router, through the router the low-level
/// client's discovery table holds for it. A device named only by its
/// instance is looked up first ([`DeviceAddress::of`] on its row, or a
/// [`Device`](crate::Device) handle).
pub struct Reader<'a, T: TransportPort + 'static> {
    /// The client the requests go through.
    pub client: &'a BACnetClient<T>,
    /// Where the device answers: every request goes there, and waits for
    /// that address's gate.
    pub device: DeviceAddress,
    /// The device's instance, for its Device object; `None` until known,
    /// and then [`instance`](Self::instance) asks the device once.
    pub instance: Option<u32>,
    /// The limits each request waits for.
    pub rate: &'a RequestLimits,
    /// Set to stop the read before its next request.
    pub cancel: &'a AtomicBool,
    /// False once the device has refused RPM.
    pub rpm: bool,
}

impl<T: TransportPort + 'static> Reader<'_, T> {
    /// Wait for this request's turn at the device, unless the job was
    /// stopped. Hold it for the one request (see [`RequestLimits::turn`]).
    /// A scheduling hook for the TUI's own request loops; the read methods
    /// take their turns themselves. A device with no path, a routed one the
    /// discovery table has no row for, fails first, taking no turn, slot or
    /// charge.
    pub async fn turn(&self) -> Result<Turn, Halt> {
        self.path().await?;
        self.take_turn().await
    }

    /// [`turn`](Self::turn), for a request whose path is already known.
    async fn take_turn(&self) -> Result<Turn, Halt> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        let turn = self.rate.turn(&self.gate()).await;
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        Ok(turn)
    }

    /// The address the device's gate keys on: an address on the client's
    /// own network, once it knows the number, is the local station's.
    pub(crate) fn gate(&self) -> DeviceAddress {
        self.device.clone().localized(self.client.network_number())
    }

    /// Wait until a request could go at once, without taking the device's
    /// gate or a slot, unless the job was stopped (see
    /// [`RequestLimits::wait_free`]). Another worker can take the gate
    /// before [`Self::turn`]; the TUI's name reads, the hook's one caller,
    /// check again then which names are still wanted.
    pub async fn free_turn(&self) -> Result<(), Halt> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        self.rate.wait_device_free(&self.gate()).await;
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        Ok(())
    }

    /// The Device object: by its instance once known, else by the
    /// wildcard instance, which a device answers as its own (Clause
    /// 15.5.2).
    pub fn device_object(&self) -> Result<ObjectIdentifier, Halt> {
        let instance = self.instance.unwrap_or(ObjectIdentifier::WILDCARD_INSTANCE);
        ObjectIdentifier::new(ObjectType::DEVICE, instance).map_err(Halt::Device)
    }

    /// The device's instance: as known, or from one ReadProperty of its
    /// Device object's Object_Identifier by the wildcard instance,
    /// remembered for the reader's later calls.
    pub async fn instance(&mut self) -> Result<u32, Halt> {
        if let Some(instance) = self.instance {
            return Ok(instance);
        }
        let any = self.device_object()?;
        let data = self
            .rp(any, PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .await?;
        let instance = match decode_all(&data).as_deref() {
            Some([PropertyValue::ObjectIdentifier(device)])
                if device.object_type() == ObjectType::DEVICE =>
            {
                device.instance_number()
            }
            _ => {
                return Err(Halt::Device(Error::decoding(
                    0,
                    "the device's Object_Identifier is not one Device object identifier",
                )));
            }
        };
        self.instance = Some(instance);
        Ok(instance)
    }

    /// The device's address as this client's link writes it.
    fn shown(&self) -> String {
        self.device
            .styled(AddressStyle::of(self.client.transport()))
    }

    /// The path to the device, from the client's discovery table for one
    /// behind a router.
    async fn path(&self) -> Result<Station, Halt> {
        path_to(self.client, &self.device)
            .await
            .map_err(Halt::Device)
    }

    /// ReadProperty, routed if the device is. The path comes first, so a
    /// device with none takes no turn.
    pub async fn rp(
        &self,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<Vec<u8>, Halt> {
        let path = self.path().await?;
        let _turn = self.take_turn().await?;
        self.client
            .read_property(&path, object, property, index)
            .await
            .map(|ack| ack.property_value)
            .map_err(Halt::Device)
    }

    /// ReadPropertyMultiple of `specs`, in the turn the caller holds
    /// ([`turn`](Self::turn)): for a caller that decides what to ask only
    /// once its turn has come, as the TUI's name reads do.
    pub async fn rpm_in(
        &self,
        _turn: &Turn,
        specs: Vec<ReadAccessSpecification>,
    ) -> Result<ReadPropertyMultipleACK, Halt> {
        let path = self.path().await?;
        self.client
            .read_property_multiple(&path, specs)
            .await
            .map_err(Halt::Device)
    }

    async fn rpm_rows(
        &self,
        object: ObjectIdentifier,
        wanted: &Wanted<'_>,
    ) -> Result<Vec<RawRow>, Halt> {
        let references = match wanted {
            Wanted::All => vec![PropertyIdentifier::ALL],
            Wanted::List(list) => list.to_vec(),
        };
        let spec = ReadAccessSpecification {
            object_identifier: object,
            list_of_property_references: references
                .into_iter()
                .map(|property_identifier| PropertyReference {
                    property_identifier,
                    property_array_index: None,
                })
                .collect(),
        };
        let path = self.path().await?;
        let _turn = self.take_turn().await?;
        let ack = self
            .client
            .read_property_multiple(&path, vec![spec])
            .await
            .map_err(Halt::Device)?;
        Ok(rpm_rows(ack))
    }

    /// The properties `wanted` of `object`, by RPM or, once the device has
    /// refused it, by ReadProperty each.
    pub async fn properties(
        &mut self,
        object: ObjectIdentifier,
        wanted: Wanted<'_>,
    ) -> Result<(Vec<RawRow>, ReadVia), Halt> {
        if self.rpm {
            match self.rpm_rows(object, &wanted).await {
                Ok(rows) => return Ok((rows, ReadVia::Rpm)),
                Err(Halt::Device(error)) => match rpm_failure(&error) {
                    RpmFailure::Refused => {
                        tracing::info!(
                            "device {} refused ReadPropertyMultiple ({error}); reading one property at a time",
                            self.shown()
                        );
                        self.rpm = false;
                    }
                    RpmFailure::TooLarge => tracing::info!(
                        "{object}: the RPM answer is too large ({error}); reading one property at a time"
                    ),
                    RpmFailure::Other => return Err(Halt::Device(error)),
                },
                Err(Halt::Cancelled) => return Err(Halt::Cancelled),
            }
        }
        let properties = match wanted {
            Wanted::All => self.property_list(object).await?,
            Wanted::List(list) => list.to_vec(),
        };
        let mut rows = Vec::with_capacity(properties.len());
        for property in properties {
            let value = match self.rp(object, property, None).await {
                Ok(value) => Ok(value),
                // A device that stopped answering fails the read once, and
                // so does a request that was never sent.
                Err(Halt::Device(error)) if unreachable(&error) || failed_locally(&error) => {
                    return Err(Halt::Device(error));
                }
                Err(Halt::Device(error)) => Err(PropertyError::from(error)),
                Err(Halt::Cancelled) => return Err(Halt::Cancelled),
            };
            rows.push((property, value));
        }
        Ok((rows, ReadVia::Rp))
    }

    /// The properties to read one by one for ALL.
    async fn property_list(
        &self,
        object: ObjectIdentifier,
    ) -> Result<Vec<PropertyIdentifier>, Halt> {
        let mut list = vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
        ];
        let listed = match self
            .rp(object, PropertyIdentifier::PROPERTY_LIST, None)
            .await
        {
            Ok(data) => decode_all(&data)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| match value {
                    PropertyValue::Enumerated(raw) => Some(PropertyIdentifier::from_raw(raw)),
                    _ => None,
                })
                .collect(),
            Err(Halt::Device(error)) if unreachable(&error) || failed_locally(&error) => {
                return Err(Halt::Device(error));
            }
            Err(Halt::Device(error)) => {
                tracing::info!(
                    "{object}: no Property_List ({error}); reading the usual properties"
                );
                FALLBACK_PROPERTIES.to_vec()
            }
            Err(Halt::Cancelled) => return Err(Halt::Cancelled),
        };
        for property in listed {
            if !list.contains(&property) && property != PropertyIdentifier::PROPERTY_LIST {
                list.push(property);
            }
        }
        Ok(list)
    }

    /// The device's Object_List. `progress` hears `(done, total)` along an
    /// element-by-element read.
    pub async fn object_list(&self, progress: impl FnMut(u32, u32)) -> Result<ListRead, Halt> {
        let (mut list, stopped) = self.object_list_until(progress).await?;
        list.stopped = stopped.map(|stop| stop.to_string());
        Ok(list)
    }

    /// [`object_list`](Self::object_list), with the error an
    /// element-by-element read stopped on beside the list, whose `stopped`
    /// is left `None`.
    pub(crate) async fn object_list_until(
        &self,
        mut progress: impl FnMut(u32, u32),
    ) -> Result<(ListRead, Option<ListStop>), Halt> {
        let device = self.device_object()?;
        let list = PropertyIdentifier::OBJECT_LIST;
        let whole = match self.rp(device, list, None).await {
            Ok(data) => {
                let read = ListRead {
                    objects: identifiers(&data),
                    indexed: false,
                    stopped: None,
                };
                return Ok((read, None));
            }
            // Element 0 would only time out too.
            Err(Halt::Device(error)) if unreachable(&error) => return Err(Halt::Device(error)),
            Err(Halt::Device(error)) => error,
            Err(Halt::Cancelled) => return Err(Halt::Cancelled),
        };
        tracing::info!(
            "device {}: Object_List not readable whole ({whole}); reading it element by element",
            self.shown()
        );
        let total = match self.rp(device, list, Some(0)).await {
            Ok(data) => match decode_all(&data).as_deref() {
                Some([PropertyValue::Unsigned(n)]) => u32::try_from(*n).unwrap_or(u32::MAX),
                _ => return Err(Halt::Device(whole)),
            },
            Err(Halt::Device(_)) => return Err(Halt::Device(whole)),
            Err(Halt::Cancelled) => return Err(Halt::Cancelled),
        };
        if total > MAX_OBJECTS {
            return Err(Halt::Device(too_many(total)));
        }
        let mut objects = Vec::with_capacity(total as usize);
        let mut stopped = None;
        for index in 1..=total {
            match self.rp(device, list, Some(index)).await {
                Ok(data) => objects.extend(identifiers(&data)),
                // Keep what was read rather than lose the whole list.
                Err(Halt::Device(error)) => {
                    stopped = Some(ListStop {
                        index,
                        total,
                        error,
                    });
                    break;
                }
                Err(Halt::Cancelled) => return Err(Halt::Cancelled),
            }
            progress(index, total);
        }
        let read = ListRead {
            objects,
            indexed: true,
            stopped: None,
        };
        Ok((read, stopped))
    }
}

/// Where an element-by-element read of an Object_List stopped, and on
/// what. Written `element 3 of 40: ` and the error.
#[derive(Debug)]
pub struct ListStop {
    /// The element whose read failed, from 1.
    pub index: u32,
    /// How many elements the list said it has.
    pub total: u32,
    /// The failure.
    pub error: Error,
}

impl std::fmt::Display for ListStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            index,
            total,
            error,
        } = self;
        write!(f, "element {index} of {total}: {error}")
    }
}

/// The properties an RPM answer holds, each its value or the device's
/// error for it, as the rows every read here returns. For a caller that
/// sends ReadPropertyMultiple itself through the client, such as the CLI's
/// one-shot reads.
pub fn rpm_rows(ack: ReadPropertyMultipleACK) -> Vec<RawRow> {
    ack.list_of_read_access_results
        .into_iter()
        .flat_map(|result| result.list_of_results)
        .map(|element| {
            let value = match (element.property_value, element.error) {
                (Some(value), _) => Ok(value),
                (None, Some((class, code))) => {
                    Err(PropertyError::Remote(RemoteError::Error { class, code }))
                }
                (None, None) => Err(PropertyError::NoValue),
            };
            (element.property_identifier, value)
        })
        .collect()
}

/// The error for an Object_List that claims `total` elements, more than
/// [`MAX_OBJECTS`]: a count past a limit, which the client's errors hold
/// best as a refused value with its numbers ([`Error::Range`]) until the
/// raw layer has typed errors of its own (#1833).
fn too_many(total: u32) -> Error {
    Error::Range(
        RangeError::new("Object_List claims {} objects; at most {} are read")
            .with_values(u64::from(total), u64::from(MAX_OBJECTS)),
    )
}

/// The object identifiers in an application-tagged value.
fn identifiers(data: &[u8]) -> Vec<ObjectIdentifier> {
    decode_all(data)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| match value {
            PropertyValue::ObjectIdentifier(object) => Some(object),
            _ => None,
        })
        .collect()
}

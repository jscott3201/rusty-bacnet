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

use bacnet_client::client::BACnetClient;
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{AbortReason, ErrorClass, ObjectType, PropertyIdentifier, RejectReason};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::format::{RawRow, RowError, decode_all};
use crate::limits::{RequestLimits, Turn};

/// How a property list was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadVia {
    /// One ReadPropertyMultiple.
    Rpm,
    /// ReadProperty per property, after the device refused RPM.
    Rp,
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
    /// full buffer): ReadProperty for this request only.
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
        _ => RpmFailure::Other,
    }
}

/// True when `error` says the device, or the way to it, isn't answering: a
/// timeout or a transport failure. Reading on would only time out again.
pub fn unreachable(error: &Error) -> bool {
    timed_out(error) || matches!(error, Error::Transport(_))
}

/// True when `error` says the request timed out: the client's own timeout,
/// or the transaction state machine giving up on its retries. A transport
/// failure is unreachable but not a timeout.
pub fn timed_out(error: &Error) -> bool {
    match error {
        Error::Remote(RemoteError::Abort { reason }) => *reason == AbortReason::TSM_TIMEOUT,
        Error::Timeout(_) => true,
        _ => false,
    }
}

/// The network `device` is on, for limits that pace networks: its routed
/// network, or 0 for the local one. `None` when the limits don't pace
/// networks, so the lookup is skipped, or the device isn't known.
pub async fn network_of<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    device: u32,
) -> Option<u16> {
    if !limits.paces_networks() {
        return None;
    }
    let found = client.get_device(device).await?;
    Some(found.source_network.unwrap_or(0))
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
pub struct Reader<'a, T: TransportPort + 'static> {
    /// The client the requests go through.
    pub client: &'a BACnetClient<T>,
    /// The device instance read, looked up in the client's discovery table.
    pub device: u32,
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
    /// take their turns themselves.
    pub async fn turn(&self) -> Result<Turn, Halt> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        let network = network_of(self.client, self.rate, self.device).await;
        let turn = self.rate.turn_on(self.device, network).await;
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        Ok(turn)
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
        self.rate.wait_device_free(self.device).await;
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Halt::Cancelled);
        }
        Ok(())
    }

    /// The Device object.
    pub fn device_object(&self) -> Result<ObjectIdentifier, Halt> {
        ObjectIdentifier::new(ObjectType::DEVICE, self.device).map_err(Halt::Device)
    }

    /// ReadProperty, routed if the device is.
    pub async fn rp(
        &self,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<Vec<u8>, Halt> {
        let _turn = self.turn().await?;
        self.client
            .read_property_from_device(self.device, object, property, index)
            .await
            .map(|ack| ack.property_value)
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
        let _turn = self.turn().await?;
        let ack = self
            .client
            .read_property_multiple_from_device(self.device, vec![spec])
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
                            self.device
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
                // A device that stopped answering fails the read once.
                Err(Halt::Device(error)) if unreachable(&error) => return Err(Halt::Device(error)),
                Err(Halt::Device(error)) => Err(RowError::from(error)),
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
            Err(Halt::Device(error)) if unreachable(&error) => return Err(Halt::Device(error)),
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
    pub async fn object_list(&self, mut progress: impl FnMut(u32, u32)) -> Result<ListRead, Halt> {
        let device = self.device_object()?;
        let list = PropertyIdentifier::OBJECT_LIST;
        let whole = match self.rp(device, list, None).await {
            Ok(data) => {
                return Ok(ListRead {
                    objects: identifiers(&data),
                    indexed: false,
                    stopped: None,
                });
            }
            // Element 0 would only time out too.
            Err(Halt::Device(error)) if unreachable(&error) => return Err(Halt::Device(error)),
            Err(Halt::Device(error)) => error,
            Err(Halt::Cancelled) => return Err(Halt::Cancelled),
        };
        tracing::info!(
            "device {}: Object_List not readable whole ({whole}); reading it element by element",
            self.device
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
            return Err(Halt::Device(Error::Encoding(format!(
                "Object_List claims {total} objects; the browser reads at most {MAX_OBJECTS}"
            ))));
        }
        let mut objects = Vec::with_capacity(total as usize);
        let mut stopped = None;
        for index in 1..=total {
            match self.rp(device, list, Some(index)).await {
                Ok(data) => objects.extend(identifiers(&data)),
                // Keep what was read rather than lose the whole list.
                Err(Halt::Device(error)) => {
                    stopped = Some(format!("element {index} of {total}: {error}"));
                    break;
                }
                Err(Halt::Cancelled) => return Err(Halt::Cancelled),
            }
            progress(index, total);
        }
        Ok(ListRead {
            objects,
            indexed: true,
            stopped,
        })
    }
}

/// The properties an RPM answer holds, each its value or the device's
/// error for it.
pub(crate) fn rpm_rows(ack: ReadPropertyMultipleACK) -> Vec<RawRow> {
    ack.list_of_read_access_results
        .into_iter()
        .flat_map(|result| result.list_of_results)
        .map(|element| {
            let value = match (element.property_value, element.error) {
                (Some(value), _) => Ok(value),
                (None, Some((class, code))) => {
                    Err(RowError::Remote(RemoteError::Error { class, code }))
                }
                (None, None) => Err(RowError::NoValue),
            };
            (element.property_identifier, value)
        })
        .collect()
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

//! Asking the device at an address which it is (#2603, #2795): the reads
//! the watch hub and a following handle send before they take a device's
//! word, never the discovery table's. The table is filled by I-Ams nobody
//! asked for, which anyone can send; an answer to the client's own request
//! needs the request's invoke ID.
//!
//! One ReadProperty of the Device object's Object_Identifier by the
//! wildcard instance names the device ([`Answer::Is`]). A device that
//! refused that read before is asked for one instance's Device object
//! instead ([`Answer::Has`]), which names nothing for certain, as a gateway
//! may answer for any instance.
//!
//! A handle made from an instance is placed by these questions too
//! (#2824): every address the discovery table has claiming the instance is
//! asked, unless its device confirmed the instance to the client before,
//! and the handle goes to the one that confirms it ([`placed`]). An I-Am,
//! even one answering the client's own Who-Is, confirms nothing: it carries
//! nothing that ties it to a request, so anyone can send one at any time.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::task::Poll;

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::DeviceAddress;
use crate::client::{Client, Reader, device_error, failed_locally, timed_out, unreachable};
use crate::error::OpsError;
use crate::format::RawRow;
use crate::limits::RequestLimits;

/// The most addresses a handle's first placement asks which device
/// answers there (#2824): an instance claimed at more addresses needing a
/// question is refused as ambiguous, with nothing sent, unless a single
/// claim among them is confirmed already, which places the handle.
pub const MAX_ASKED: usize = 8;

/// What the device at an address said of itself, asked which device it is
/// ([`identify`], and the questions a [`Client`] asks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Answer {
    /// It named itself this instance, read by the wildcard instance.
    Is(u32),
    /// It has this Device object, read by its instance: no proof of which
    /// device it is, as a gateway may answer for any instance.
    Has(u32),
    /// It has no Device object of the instance asked for.
    Not,
    /// It answered with another error, which says neither.
    Unclear,
    /// The client's own failure (a request never sent, or cut short):
    /// nothing from the device.
    Failed,
    /// It didn't answer.
    Silent,
}

impl Answer {
    /// The instance it answered for, by either read.
    pub fn instance(self) -> Option<u32> {
        match self {
            Self::Is(instance) | Self::Has(instance) => Some(instance),
            _ => None,
        }
    }

    /// Whether this answer confirms the device at the address as device
    /// `instance`, for a first placement ([`placed`]): it named itself so,
    /// or, refusing the wildcard, has that instance's Device object.
    pub fn confirms(self, instance: u32) -> bool {
        self.instance() == Some(instance)
    }

    /// Whether this answer leaves open that the device there is device
    /// `instance`: it confirms it, or says nothing either way (an unclear
    /// answer, or the client's own failure). Silence, another instance, or
    /// no such Device object rule it out.
    fn open(self, instance: u32) -> bool {
        self.confirms(instance) || matches!(self, Self::Unclear | Self::Failed)
    }
}

/// Where a handle made from `instance` is placed (#2824), given each
/// address claiming it with what its device answered when asked which it
/// is: the one address whose device confirms it ([`Answer::confirms`])
/// while no other claim is left open (an unclear answer, or the client's
/// own failure), so a choice is never made silently.
///
/// # Errors
///
/// [`OpsError::NotConfirmed`] for one claim whose device didn't confirm
/// the instance, naming it; [`OpsError::Ambiguous`] for several when none
/// confirms (naming them all), or two or more are left open (naming
/// those); [`OpsError::UnknownDevice`] for no claim at all.
pub fn placed(instance: u32, asked: &[(DeviceAddress, Answer)]) -> Result<DeviceAddress, OpsError> {
    let mut open: Vec<DeviceAddress> = asked
        .iter()
        .filter(|(_, answer)| answer.open(instance))
        .map(|(address, _)| address.clone())
        .collect();
    let confirmed = asked
        .iter()
        .filter(|(_, answer)| answer.confirms(instance))
        .count();
    let ambiguous = |mut candidates: Vec<DeviceAddress>| {
        candidates.sort();
        candidates.dedup();
        OpsError::Ambiguous {
            instance,
            candidates,
        }
    };
    match (confirmed, open.len(), asked) {
        (1, 1, _) => Ok(open.remove(0)),
        (0, _, []) => Err(OpsError::UnknownDevice {
            target: instance.to_string(),
            asked: None,
        }),
        (0, _, [(address, answer)]) => Err(OpsError::NotConfirmed {
            instance,
            at: address.clone(),
            candidate: None,
            answer: Some(*answer),
            placing: true,
        }),
        (0, _, _) => Err(ambiguous(
            asked.iter().map(|(address, _)| address.clone()).collect(),
        )),
        _ => Err(ambiguous(open)),
    }
}

/// When more of `claims` than [`MAX_ASKED`] would need a question (no
/// answer known for them yet): the one claim confirmed already (its
/// device's earlier answer remembered, or a row registered by hand) places
/// the handle, as a place confirmed before is joined, and otherwise the
/// instance is ambiguous, with nothing sent. `None` while the questions may
/// be asked.
pub(crate) fn over_cap(
    instance: u32,
    claims: &[(DeviceAddress, Option<Answer>)],
) -> Option<Result<DeviceAddress, OpsError>> {
    let unasked = claims.iter().filter(|(_, known)| known.is_none()).count();
    if unasked <= MAX_ASKED {
        return None;
    }
    let confirmed: Vec<&DeviceAddress> = claims
        .iter()
        .filter(|(_, known)| known.is_some_and(|answer| answer.confirms(instance)))
        .map(|(address, _)| address)
        .collect();
    Some(match confirmed.as_slice() {
        [address] => Ok((*address).clone()),
        _ => {
            let mut candidates: Vec<DeviceAddress> =
                claims.iter().map(|(address, _)| address.clone()).collect();
            candidates.sort();
            candidates.dedup();
            Err(OpsError::Ambiguous {
                instance,
                candidates,
            })
        }
    })
}

/// Ask the device at `address` which device it is, for a first contact
/// with device `instance` (#2824), as a [`Client`]'s handle asks: one
/// ReadProperty of its Device object's Object_Identifier by the wildcard
/// instance, and, refused, one of `instance`'s Device object. For a front
/// end that holds a low-level client and no [`Client`]: nothing is
/// remembered. Each request takes its turn at the device's gate under
/// `limits`. Only a timeout is the device's silence ([`Answer::Silent`]):
/// a router's reject or the link failing the send says nothing of it
/// ([`Answer::Failed`]).
pub async fn identify<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    address: &DeviceAddress,
    instance: u32,
) -> Answer {
    let never = AtomicBool::new(false);
    let mut reader = Reader {
        client,
        device: address.clone(),
        instance: None,
        rate: limits,
        cancel: &never,
        rpm: true,
    };
    match reader.instance().await.map_err(device_error) {
        Ok(found) => Answer::Is(found),
        Err(error) => match left_silence(&error) {
            Some(answer) => answer,
            None => {
                reader.instance = Some(instance);
                object_answer(&reader, instance, left_silence).await
            }
        },
    }
}

/// What `reader`'s device says to one ReadProperty of device `instance`'s
/// Object_Identifier: [`Answer::Has`] with the instance its answer holds,
/// [`Answer::Not`] for no such object; `quiet` says which errors are no
/// answer.
async fn object_answer<T: TransportPort + 'static>(
    reader: &Reader<'_, T>,
    instance: u32,
    quiet: Quiet,
) -> Answer {
    let Ok(device) = ObjectIdentifier::new(ObjectType::DEVICE, instance) else {
        return Answer::Unclear;
    };
    let property = PropertyIdentifier::OBJECT_IDENTIFIER;
    match reader
        .rp(device, property, None)
        .await
        .map_err(device_error)
    {
        Ok(raw) => named(&[(property, Ok(raw))]).map_or(Answer::Unclear, Answer::Has),
        Err(error) => quiet(&error).unwrap_or(match error.remote() {
            Some(RemoteError::Error { code, .. }) if code == ErrorCode::UNKNOWN_OBJECT => {
                Answer::Not
            }
            _ => Answer::Unclear,
        }),
    }
}

/// Poll every one of `futures` until all are done: their outputs, in
/// order. Each wake polls every one still running, so none waits unpolled
/// behind another (#1494); dropping it drops them all.
pub(crate) async fn every<F: Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut running: Vec<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = running.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (future, output) in running.iter_mut().zip(done.iter_mut()) {
            if output.is_none() {
                match future.as_mut().poll(cx) {
                    Poll::Ready(value) => *output = Some(value),
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    done.into_iter().flatten().collect()
}

/// Which errors are no answer, for a read asking which device answers.
pub(crate) type Quiet = fn(&Error) -> Option<Answer>;

/// The answer `error` makes when the device said nothing: none, or the
/// client's own failure, which says nothing of it.
pub(crate) fn silence(error: &Error) -> Option<Answer> {
    if unreachable(error) {
        Some(Answer::Silent)
    } else if failed_locally(error) {
        Some(Answer::Failed)
    } else {
        None
    }
}

/// [`silence`] for whether a device left its address: only the request
/// timing out is its silence. A router's reject (which anyone on a routed
/// path can send), the link failing the send, or another of the client's
/// own failures says nothing of the device ([`Answer::Failed`]).
pub(crate) fn left_silence(error: &Error) -> Option<Answer> {
    if timed_out(error) {
        Some(Answer::Silent)
    } else if unreachable(error) || failed_locally(error) {
        Some(Answer::Failed)
    } else {
        None
    }
}

/// The instance `rows` (an answer for a Device object's
/// Object_Identifier) name, if they name one.
pub(crate) fn named(rows: &[RawRow]) -> Option<u32> {
    let (_, raw) = rows
        .iter()
        .find(|(p, _)| *p == PropertyIdentifier::OBJECT_IDENTIFIER)?;
    match crate::format::decode_all(raw.as_ref().ok()?).as_deref() {
        Some([PropertyValue::ObjectIdentifier(device)])
            if device.object_type() == ObjectType::DEVICE =>
        {
            Some(device.instance_number())
        }
        _ => None,
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// Ask the device at `address` which instance it is: one ReadProperty
    /// of its Device object's Object_Identifier by the wildcard instance,
    /// or, for a device that refused that before (or refuses it now), of
    /// `instance`'s Device object, which only that device has. Through the
    /// device's gate and the client's rate, as any request; `quiet` says
    /// which errors are no answer.
    pub(crate) async fn which_at(
        &self,
        address: &DeviceAddress,
        instance: u32,
        quiet: Quiet,
    ) -> Answer {
        if !self.wildcard_refused(address) {
            match self.ask_instance(address).await {
                Ok(found) => return Answer::Is(found),
                Err(error) => {
                    if let Some(answer) = quiet(&error) {
                        return answer;
                    }
                }
            }
        }
        self.has_device(address, instance, quiet).await
    }

    /// Whether the device at `address` has device `instance`'s Device
    /// object: one ReadProperty of its Object_Identifier ([`Answer::Has`],
    /// with the instance the answer holds); another answers that it has no
    /// such object ([`Answer::Not`]). Through the device's gate and the
    /// client's rate.
    pub(crate) async fn has_device(
        &self,
        address: &DeviceAddress,
        instance: u32,
        quiet: Quiet,
    ) -> Answer {
        let reader = self.reader(address, Some(instance));
        object_answer(&reader, instance, quiet).await
    }
}

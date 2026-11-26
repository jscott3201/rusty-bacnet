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
/// answers there (#2824): an instance claimed at more is refused as
/// ambiguous, with nothing sent.
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
        (0, _, [(address, _)]) => Err(OpsError::NotConfirmed {
            instance,
            at: address.clone(),
            candidate: None,
        }),
        (0, _, _) => Err(ambiguous(
            asked.iter().map(|(address, _)| address.clone()).collect(),
        )),
        _ => Err(ambiguous(open)),
    }
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
async fn every<F: Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
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

    /// Where a handle made from `instance` is placed among `claims`, the
    /// addresses claiming it (#2824): each is asked which device answers
    /// there ([`which_at`](Self::which_at), all at once), unless its device
    /// confirmed the instance to the client before, or `known` holds its
    /// answer already, and the answers decide ([`placed`]). A device
    /// refusing the wildcard that has the instance's Device object is
    /// remembered so too. More than [`MAX_ASKED`] claims are refused as
    /// ambiguous, with nothing sent.
    ///
    /// # Errors
    ///
    /// As [`placed`].
    pub(crate) async fn confirm(
        &self,
        instance: u32,
        claims: &[DeviceAddress],
        known: Option<&(DeviceAddress, Answer)>,
    ) -> Result<DeviceAddress, OpsError> {
        if claims.len() > MAX_ASKED {
            return Err(OpsError::Ambiguous {
                instance,
                candidates: claims.to_vec(),
            });
        }
        let asks = claims.iter().map(|claim| async move {
            match known {
                Some((at, answer)) if at == claim => *answer,
                _ => self.claimed(claim, instance).await,
            }
        });
        let answers = every(asks).await;
        let asked: Vec<(DeviceAddress, Answer)> = claims.iter().cloned().zip(answers).collect();
        for (claim, answer) in &asked {
            tracing::debug!("device {instance} is claimed at {claim}, which answered {answer:?}");
        }
        placed(instance, &asked)
    }

    /// What the device at `claim` says of being device `instance`: as it
    /// confirmed before, or asked now. Only a timeout is its silence. A
    /// row the application registered by hand, never announced, is its own
    /// word, as an address it names is: taken as the device's, unasked.
    async fn claimed(&self, claim: &DeviceAddress, instance: u32) -> Answer {
        let registered = claim.row(self.low_level()).await.is_some_and(|row| {
            row.last_announced.is_none() && row.object_identifier.instance_number() == instance
        });
        if registered {
            return Answer::Is(instance);
        }
        if self.confirmed_as(claim) == Some(instance) {
            return match self.said(claim) {
                Some(_) => Answer::Is(instance),
                None => Answer::Has(instance),
            };
        }
        let answer = self.which_at(claim, instance, left_silence).await;
        if answer == Answer::Has(instance) {
            self.vouch(claim.clone(), instance);
        }
        answer
    }
}

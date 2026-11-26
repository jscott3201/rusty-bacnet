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

use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::DeviceAddress;
use crate::client::{Client, device_error, failed_locally, timed_out, unreachable};
use crate::format::RawRow;

/// What the device at an address said of itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
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
    pub(crate) fn instance(self) -> Option<u32> {
        match self {
            Self::Is(instance) | Self::Has(instance) => Some(instance),
            _ => None,
        }
    }
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
        let Ok(device) = ObjectIdentifier::new(ObjectType::DEVICE, instance) else {
            return Answer::Unclear;
        };
        let property = PropertyIdentifier::OBJECT_IDENTIFIER;
        let reader = self.reader(address, Some(instance));
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
}

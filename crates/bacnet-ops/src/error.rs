//! [`OpsError`]: why a [`Client`](crate::Client) or
//! [`Device`](crate::Device) call failed as a whole (#2560).
//!
//! The low-level client's errors are sorted by what a caller does next:
//! fix the text it gave ([`OpsError::Parse`]), name the device another way
//! ([`OpsError::UnknownDevice`], [`OpsError::Ambiguous`]), try again later
//! ([`OpsError::Unreachable`]), or take the device's answer
//! ([`OpsError::Refused`]). Every other client error passes through as
//! [`OpsError::Client`] unchanged, so the typed errors of `bacnet-types`
//! (#1833) reach the caller as they are. A device that breaks the rules of
//! a paged read is [`OpsError::Paging`]. In a bulk read a property the
//! device refused is its row's [`PropertyError`], and a device that stops
//! answering part way is the [`Readings::stopped`](crate::Readings::stopped)
//! of what was read.

use std::fmt;

use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;

use crate::client::{ResolveScope, timed_out as is_timed_out, unreachable as is_unreachable};
use crate::device::DeviceAddress;
use crate::format::PropertyError;
use crate::label::kebab;

/// Why a high-level call failed as a whole.
///
/// A timeout and a failure of the link are both [`Unreachable`]: the device
/// didn't answer, so it may not have seen the request. [`Refused`] is the
/// device's own Error, Reject or Abort PDU, and a request that timed out is
/// never one, even when the low-level client reports the timeout as an
/// abort. Not `Clone`: a link failure holds the operating system's error.
///
/// [`Unreachable`]: Self::Unreachable
/// [`Refused`]: Self::Refused
#[derive(Debug)]
#[non_exhaustive]
pub enum OpsError {
    /// Text or settings this client can't use: a target, an address, an
    /// object or a property that names nothing it can address, a property a
    /// call doesn't read (every property, ALL, is
    /// [`Device::read_object`](crate::Device::read_object)'s), a discovery
    /// scope or range that can't be sent, or MS/TP settings the transport
    /// refuses. Nothing was sent.
    Parse {
        /// What the text was meant to be: `"target"`, `"address"`,
        /// `"object"` or `"property"`; for a discovery's scope or range,
        /// `"network"` or `"range"`; for an MS/TP connect builder's
        /// settings, `"MS/TP settings"`.
        what: &'static str,
        /// The text as given.
        input: String,
        /// Why it was refused, as a sentence that names the text.
        reason: String,
    },
    /// No address claims the device named, and none answered a Who-Is for
    /// it; no router is known for a remote station's network; or the
    /// B/IPv6 node at an address didn't say its VMAC. Nothing was sent to
    /// the device.
    UnknownDevice {
        /// The device as it was named: an instance (`1234`, `2:1234`) or an
        /// address (`2:0x05`).
        target: String,
        /// Where the one Who-Is for it went unanswered, or `None` when none
        /// was sent: the text then says how to reach further, from a bare
        /// instance asked on this network alone (the client's default
        /// [`ResolveScope`]).
        asked: Option<ResolveScope>,
    },
    /// Two or more addresses claim the instance named, so which device is
    /// meant is unknown. Nothing was sent. Name one of `candidates` by its
    /// address instead.
    Ambiguous {
        /// The Device instance named.
        instance: u32,
        /// Every address claiming it, ordered by network and then MAC.
        candidates: Vec<DeviceAddress>,
    },
    /// The device, or the way to it, didn't answer: the request timed out
    /// (the client's own timeout, or the transaction state machine giving
    /// up on its retries), or the link failed.
    Unreachable {
        /// Where the request went.
        device: DeviceAddress,
        /// The low-level client's error.
        source: Error,
    },
    /// The device answered with an Error, Reject or Abort PDU.
    Refused {
        /// Where the request went.
        device: DeviceAddress,
        /// The device's answer: an Error PDU's class and code, or a Reject
        /// or Abort PDU's reason.
        error: RemoteError,
        /// The fields an Error PDU's production adds after the class and
        /// code, as the client decoded them (`Error::Structured`); `None`
        /// for an answer without them.
        detail: Option<Box<ErrorDetail>>,
    },
    /// The device broke a rule of a paged read, so reading on would repeat
    /// what was read or never end. What was read stays unreported: the
    /// call fails.
    Paging(PagingFault),
    /// Any other failure the low-level client reported, as it reported it:
    /// an answer that doesn't decode, a request too long for the path, and
    /// the like.
    Client(Error),
}

impl OpsError {
    /// The low-level client's `error` for a request to `device`.
    pub(crate) fn at(device: &DeviceAddress, error: Error) -> Self {
        if is_unreachable(&error) {
            return Self::Unreachable {
                device: device.clone(),
                source: error,
            };
        }
        let (error, detail) = match error {
            Error::Remote(remote) => (remote, None),
            Error::Structured {
                class,
                code,
                detail,
            } => (RemoteError::Error { class, code }, Some(detail)),
            error => return Self::from(error),
        };
        Self::Refused {
            device: device.clone(),
            error,
            detail,
        }
    }

    /// Text refused as a `what`, for `reason`.
    pub(crate) fn parse(what: &'static str, input: &str, reason: String) -> Self {
        Self::Parse {
            what,
            input: input.to_string(),
            reason,
        }
    }

    /// True when the device, or the way to it, isn't answering: a timeout
    /// or a link failure. Reading on would only fail the same way. Also
    /// true for a [`Client`](Self::Client) error of that kind, which a
    /// conversion that names no device ([`From<Error>`]) leaves there.
    pub fn unreachable(&self) -> bool {
        match self {
            Self::Unreachable { .. } => true,
            Self::Client(error) => is_unreachable(error),
            _ => false,
        }
    }

    /// True when the request timed out; a link failure is unreachable but
    /// not a timeout.
    pub fn timed_out(&self) -> bool {
        match self {
            Self::Unreachable { source: error, .. } | Self::Client(error) => is_timed_out(error),
            _ => false,
        }
    }

    /// The device's Error, Reject or Abort, when it answered with one.
    pub fn remote(&self) -> Option<RemoteError> {
        match self {
            Self::Refused { error, .. } => Some(*error),
            Self::Client(error) => error.remote(),
            _ => None,
        }
    }
}

impl From<Error> for OpsError {
    /// A client error that names no device a request went to: the device
    /// lookups' errors as [`UnknownDevice`](Self::UnknownDevice) and
    /// [`Ambiguous`](Self::Ambiguous), anything else as
    /// [`Client`](Self::Client). The [`Device`](crate::Device) methods sort
    /// a request's error by its device instead.
    fn from(error: Error) -> Self {
        match error {
            Error::UnknownDevice { device } => Self::UnknownDevice {
                target: device,
                asked: None,
            },
            Error::AmbiguousDevice {
                instance,
                candidates,
            } => Self::Ambiguous {
                instance,
                candidates: candidates
                    .iter()
                    .filter_map(|c| DeviceAddress::new(c.network_number, &c.mac_address).ok())
                    .collect(),
            },
            error => Self::Client(error),
        }
    }
}

impl From<OpsError> for PropertyError {
    /// A call's error as one property's: the device's answer as
    /// [`PropertyError::Remote`], anything else as its text. For a front
    /// end that reads one property among others and keeps the rest when it
    /// fails, as a bulk read does.
    fn from(error: OpsError) -> Self {
        match error.remote() {
            Some(remote) => Self::Remote(remote),
            None => Self::Other(error.to_string()),
        }
    }
}

impl fmt::Display for OpsError {
    /// The same text the low-level client's error would show, so a front
    /// end's messages read as before: a refused text by its reason, a
    /// device's answer as the client writes it (a structured error's
    /// detail included), the lookups' errors as `Error::UnknownDevice` and
    /// `Error::AmbiguousDevice` write theirs, and a broken paging rule as
    /// [`PagingFault`] writes it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { reason, .. } => f.write_str(reason),
            Self::UnknownDevice {
                target,
                asked: None,
            } => {
                write!(
                    f,
                    "device {target} is not known to the client; discover it first"
                )
            }
            Self::UnknownDevice {
                target,
                asked: Some(ResolveScope::Local),
            } => write!(
                f,
                "device {target} did not answer a Who-Is on this network, the only one \
                 asked: name its network in the target (5:{target} for network 5), or widen \
                 the client's resolve scope (with_resolve_scope)"
            ),
            Self::UnknownDevice {
                target,
                asked: Some(ResolveScope::Network(network)),
            } => write!(
                f,
                "device {target} did not answer a Who-Is on network {network}"
            ),
            Self::UnknownDevice {
                target,
                asked: Some(ResolveScope::Global),
            } => write!(f, "device {target} did not answer a Who-Is on any network"),
            Self::Ambiguous {
                instance,
                candidates,
            } => {
                write!(
                    f,
                    "device {instance} is claimed by {} addresses: ",
                    candidates.len()
                )?;
                for (i, address) in candidates.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{address}")?;
                }
                f.write_str("; name one by its address")
            }
            Self::Unreachable { source, .. } => write!(f, "{source}"),
            Self::Refused {
                error,
                detail: None,
                ..
            } => write!(f, "{error}"),
            Self::Refused {
                error,
                detail: Some(detail),
                ..
            } => write!(f, "{error}, {detail}"),
            Self::Paging(fault) => write!(f, "{fault}"),
            Self::Client(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for OpsError {
    /// The low-level error's own source, not that error: the text above is
    /// already its text, so a front end that prints the chain shows each
    /// cause once.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreachable { source: error, .. } | Self::Client(error) => error.source(),
            _ => None,
        }
    }
}

/// How a device broke the rules of a paged read ([`OpsError::Paging`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PagingFault {
    /// An answer named an object an earlier answer of the same read named:
    /// reading on would list it twice, or go round for ever.
    Repeated {
        /// The paged service, `GetEventInformation`.
        service: &'static str,
        /// The object named again.
        object: ObjectIdentifier,
    },
    /// An answer said more remain but named no new object to go on after.
    Stalled {
        /// The paged service, `GetEventInformation`.
        service: &'static str,
    },
}

impl fmt::Display for PagingFault {
    /// `GetEventInformation named analog-input:3 again`, the object as the
    /// tools' arguments write it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repeated { service, object } => write!(
                f,
                "{service} named {}:{} again",
                kebab(object.object_type()),
                object.instance_number()
            ),
            Self::Stalled { service } => {
                write!(f, "{service} said more remain but named no new object")
            }
        }
    }
}

impl std::error::Error for PagingFault {}

impl From<PagingFault> for OpsError {
    fn from(fault: PagingFault) -> Self {
        Self::Paging(fault)
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

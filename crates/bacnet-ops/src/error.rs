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
//! of what was read. A write the client's
//! [`WritePolicy`](crate::WritePolicy) refuses is [`OpsError::Denied`],
//! with nothing sent; a relinquish that went through but whose read after
//! it failed is [`OpsError::ReadBack`]; and an object the device says has
//! no Priority_Array is [`OpsError::NoPriorityArray`]. With `sc-tls`, TLS
//! material a BACnet/SC configuration refused is `OpsError::Tls`, so `?` on
//! `ScNodeTlsConfig::from_pem_files` keeps its typed error (#2600).
//! [`OpsError::written`] says whether a write went through first.

use std::fmt;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;

use crate::client::{Scope, timed_out as is_timed_out, unreachable as is_unreachable};
use crate::device::DeviceAddress;
use crate::format::PropertyError;
use crate::label::kebab;
use crate::watch::WatchError;
use crate::write::{WriteDenied, Written};

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
        /// Where the one Who-Is for it went unanswered: the named station
        /// or network, or for a bare instance the client's
        /// [`crate::ResolveScope`]; `None` when none was sent. For
        /// `Some(Scope::Local)`, a bare instance asked on this
        /// network alone (the default), the text says how to reach
        /// further.
        asked: Option<Scope>,
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
    /// A watch couldn't take its points: the client's hub watches as many
    /// as it may ([`MAX_WATCHED`](crate::watch::MAX_WATCHED)). Nothing was
    /// watched.
    Watch(WatchError),
    /// TLS material a BACnet/SC configuration refused, as its constructor
    /// reported it: a file that can't be read, PEM that doesn't decode, a
    /// key that isn't the chain leaf's, and the like;
    /// [`ScTlsConfigError::input`](crate::ScTlsConfigError::input) names
    /// the file at fault. Nothing was sent. With `sc-tls`.
    #[cfg(feature = "sc-tls")]
    #[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
    Tls(crate::ScTlsConfigError),
    /// Any other failure the low-level client reported, as it reported it:
    /// an answer that doesn't decode, a request too long for the path, and
    /// the like.
    Client(Error),
    /// The client's [`WritePolicy`](crate::WritePolicy) refuses the
    /// priority a write or relinquish named. Nothing was sent.
    Denied(WriteDenied),
    /// A relinquish the device acknowledged, so it took effect, whose read
    /// of the object's priorities after it failed. Don't send it again:
    /// read them once the device answers
    /// ([`Device::priority_array`](crate::Device::priority_array)).
    ReadBack {
        /// The relinquish, as sent.
        written: Box<Written>,
        /// Why the read after it failed.
        source: Box<OpsError>,
    },
    /// The device says the object has no Priority_Array (an unknown
    /// property), so its Present_Value isn't commandable: a value object
    /// built without one, say. For a relinquish, the device took the NULL
    /// first as a write to a property that isn't commandable, which
    /// changes nothing (Clause 19.2.1), and `relinquished` holds it.
    NoPriorityArray {
        /// Where the read went.
        device: DeviceAddress,
        /// The object.
        object: ObjectIdentifier,
        /// The relinquish acknowledged before the read, if one was.
        relinquished: Option<Box<Written>>,
    },
    /// An Object_List the device couldn't send whole was read element by
    /// element, and one element's read failed: part of a list is not the
    /// device, so the call that needed the list fails
    /// ([`Device::inventory`](crate::Device::inventory)).
    ObjectList {
        /// The element whose read failed, from 1.
        index: u32,
        /// How many elements the list said it has.
        total: u32,
        /// Why that element's read failed.
        source: Box<OpsError>,
    },
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
    /// conversion that names no device ([`From<Error>`]) leaves there. False
    /// for a [`ReadBack`](Self::ReadBack), whose write went through.
    pub fn unreachable(&self) -> bool {
        match self {
            Self::Unreachable { .. } => true,
            Self::Client(error) => is_unreachable(error),
            Self::ObjectList { source, .. } => source.unreachable(),
            _ => false,
        }
    }

    /// True when the request timed out; a link failure is unreachable but
    /// not a timeout.
    pub fn timed_out(&self) -> bool {
        match self {
            Self::Unreachable { source: error, .. } | Self::Client(error) => is_timed_out(error),
            Self::ObjectList { source, .. } => source.timed_out(),
            _ => false,
        }
    }

    /// The device's Error, Reject or Abort, when it answered with one.
    pub fn remote(&self) -> Option<RemoteError> {
        match self {
            Self::Refused { error, .. } => Some(*error),
            Self::Client(error) => error.remote(),
            Self::ObjectList { source, .. } => source.remote(),
            Self::NoPriorityArray { .. } => Some(RemoteError::Error {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
            }),
            _ => None,
        }
    }

    /// The write the device acknowledged before the call failed, when one
    /// was: it took effect, so it needs no second send. A
    /// [`ReadBack`](Self::ReadBack)'s relinquish, whose view of the slots
    /// after isn't known, and a [`NoPriorityArray`](Self::NoPriorityArray)
    /// relinquish, which changed nothing.
    pub fn written(&self) -> Option<&Written> {
        match self {
            Self::ReadBack { written, .. } => Some(written),
            Self::NoPriorityArray { relinquished, .. } => relinquished.as_deref(),
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
                asked: Some(Scope::Local),
            } => write!(
                f,
                "device {target} did not answer a Who-Is on this network, the only one \
                 asked: name its network in the target (5:{target} for network 5), or widen \
                 the client's resolve scope (with_resolve_scope)"
            ),
            Self::UnknownDevice {
                target,
                asked: Some(Scope::Network(network)),
            } => write!(
                f,
                "device {target} did not answer a Who-Is on network {network}"
            ),
            Self::UnknownDevice {
                target,
                asked: Some(Scope::Global),
            } => write!(f, "device {target} did not answer a Who-Is on any network"),
            Self::UnknownDevice {
                target,
                asked: Some(Scope::Directed(_)),
            } => write!(
                f,
                "device {target} did not answer a station-directed Who-Is"
            ),
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
            Self::ObjectList {
                index,
                total,
                source,
            } => write!(f, "Object_List element {index} of {total}: {source}"),
            Self::Watch(error) => write!(f, "{error}"),
            #[cfg(feature = "sc-tls")]
            Self::Tls(error) => write!(f, "{error}"),
            Self::Client(error) => write!(f, "{error}"),
            Self::Denied(denied) => write!(f, "{denied}"),
            Self::ReadBack { written, source } => {
                let priority = written
                    .priority
                    .map(|p| format!(" at priority {p}"))
                    .unwrap_or_default();
                write!(
                    f,
                    "the device acknowledged the relinquish of {} {}{priority}, \
                     but reading its priorities back failed: {source}",
                    crate::label::object_label(written.property.object),
                    kebab(written.property.property),
                )
            }
            Self::NoPriorityArray {
                device,
                object,
                relinquished,
            } => {
                write!(
                    f,
                    "{} at {device} has no Priority_Array, the device says, so its \
                     present-value isn't commandable",
                    crate::label::object_label(*object)
                )?;
                match relinquished.as_ref().and_then(|w| w.priority) {
                    Some(p) => write!(
                        f,
                        "; the NULL sent at priority {p} was acknowledged and changed nothing"
                    ),
                    None => Ok(()),
                }
            }
        }
    }
}

impl std::error::Error for OpsError {
    /// The low-level (or TLS configuration) error's own source, not that
    /// error: the text above is already its text, so a front end that prints
    /// the chain shows each cause once. Match `OpsError::Tls` (with
    /// `sc-tls`) for the typed TLS error itself.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreachable { source: error, .. } | Self::Client(error) => error.source(),
            #[cfg(feature = "sc-tls")]
            Self::Tls(error) => error.source(),
            Self::ReadBack { source, .. } | Self::ObjectList { source, .. } => source.source(),
            _ => None,
        }
    }
}

#[cfg(feature = "sc-tls")]
impl From<crate::ScTlsConfigError> for OpsError {
    /// As [`OpsError::Tls`], so `?` on an SC TLS constructor reaches an
    /// `OpsError` with its typed error.
    fn from(error: crate::ScTlsConfigError) -> Self {
        Self::Tls(error)
    }
}

impl From<WriteDenied> for OpsError {
    fn from(denied: WriteDenied) -> Self {
        Self::Denied(denied)
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

//! [`OpsError`]: why a [`Client`](crate::Client) or
//! [`Device`](crate::Device) call failed as a whole (#2560).
//!
//! The low-level client's errors are sorted by what a caller does next:
//! fix the text it gave ([`OpsError::Parse`]), name the device another way
//! ([`OpsError::UnknownDevice`], [`OpsError::Ambiguous`]), try again later
//! ([`OpsError::Unreachable`]), or take the device's answer
//! ([`OpsError::Refused`]). Every other client error passes through as
//! [`OpsError::Client`] unchanged, so the typed errors of `bacnet-types`
//! (#1833) reach the caller as they are. A bulk read never fails as a
//! whole: a property the device refused is its row's
//! [`PropertyError`], and a device that stops
//! answering is the [`Readings::stopped`](crate::Readings::stopped) of what
//! was read.

use std::fmt;

use bacnet_types::error::{Error, RemoteError};

use crate::client::{timed_out as is_timed_out, unreachable as is_unreachable};
use crate::device::DeviceAddress;
use crate::format::PropertyError;

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
    /// Text that doesn't name something this client can address: a target,
    /// an address, an object or a property. Nothing was sent, apart from
    /// the one Virtual-Address-Resolution that looks a B/IPv6 address's
    /// VMAC up.
    Parse {
        /// What the text was meant to be: `"target"`, `"address"`,
        /// `"object"` or `"property"`.
        what: &'static str,
        /// The text as given.
        input: String,
        /// Why it was refused, as a sentence that names the text.
        reason: String,
    },
    /// No address claims the device named, and none answered a Who-Is for
    /// it; or no router is known for a remote station's network. Nothing
    /// was sent to the device.
    UnknownDevice {
        /// The device as it was named: an instance (`1234`, `2:1234`) or an
        /// address (`2:0x05`).
        target: String,
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
        /// The device's answer; an Error PDU's extra fields (a structured
        /// error) are left out.
        error: RemoteError,
    },
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
        match error.remote() {
            Some(remote) => Self::Refused {
                device: device.clone(),
                error: remote,
            },
            None => Self::from(error),
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
            Error::UnknownDevice { device } => Self::UnknownDevice { target: device },
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
    /// device's answer as the client writes it, and the lookups' errors as
    /// `Error::UnknownDevice` and `Error::AmbiguousDevice` write theirs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { reason, .. } => f.write_str(reason),
            Self::UnknownDevice { target } => {
                write!(
                    f,
                    "device {target} is not known to the client; discover it first"
                )
            }
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
            Self::Refused { error, .. } => write!(f, "{error}"),
            Self::Client(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for OpsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreachable { source: error, .. } | Self::Client(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

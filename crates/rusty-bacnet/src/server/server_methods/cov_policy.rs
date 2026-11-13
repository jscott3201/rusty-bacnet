//! The constructor's keyword-only `cov_policy` dict, read into the Rust
//! `CovPolicy` the server starts with (#1100).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use bacnet_device_core::cov::{CovPolicy, CovRecipient};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;

use super::policy_dict::{entries, unexpected_key};

const POLICY: &str = "cov_policy";

/// Read `cov_policy` into a validated `CovPolicy`.
///
/// Each key is a field under its Rust name, and a key left out keeps the Rust
/// default, so `None` and `{}` both give `CovPolicy::default()`. An unknown
/// or non-str key and a value of the wrong type raise TypeError, a negative
/// or oversized integer OverflowError, and a value `CovPolicy::validate`
/// refuses ValueError: the exception types the constructor's other keywords
/// raise.
///
/// Neither the destructuring of the defaults nor the literal that rebuilds
/// the policy uses `..`, so a field added to `CovPolicy` stops the bindings
/// compiling until it is bound here, and a bound field no key assigns is an
/// unneeded `mut`, which the warning gate rejects.
pub(crate) fn cov_policy(dict: Option<&Bound<'_, PyDict>>) -> PyResult<CovPolicy> {
    cov_policy_named(POLICY, dict)
}

/// [`cov_policy`] for a dict the errors call `name`, such as an endpoint's
/// `cov['policy']`.
pub(crate) fn cov_policy_named(
    name: &str,
    dict: Option<&Bound<'_, PyDict>>,
) -> PyResult<CovPolicy> {
    let CovPolicy {
        mut max_subscriptions_global,
        mut max_subscriptions_per_peer,
        mut reserved_capacity,
        mut reserved_peers,
        mut reserved_recipients,
        mut allow_indefinite_subscriptions,
        mut max_indefinite_per_peer,
        mut max_notifications_per_event,
        mut max_notification_bytes_per_event,
        mut max_confirmed_in_flight_per_peer,
    } = CovPolicy::default();
    for (key, value) in entries(name, dict)? {
        match key.as_str() {
            "max_subscriptions_global" => max_subscriptions_global = field(name, &key, &value)?,
            "max_subscriptions_per_peer" => max_subscriptions_per_peer = field(name, &key, &value)?,
            "reserved_capacity" => reserved_capacity = field(name, &key, &value)?,
            "reserved_peers" => {
                reserved_peers = field::<Vec<Vec<u8>>>(name, &key, &value)?
                    .iter()
                    .map(|mac| MacAddr::from_slice(mac))
                    .collect();
            }
            "reserved_recipients" => {
                reserved_recipients = field::<Vec<(Option<u16>, Vec<u8>)>>(name, &key, &value)?
                    .into_iter()
                    .map(|(network, mac)| recipient(network, &mac))
                    .collect();
            }
            "allow_indefinite_subscriptions" => {
                allow_indefinite_subscriptions = field(name, &key, &value)?;
            }
            "max_indefinite_per_peer" => max_indefinite_per_peer = field(name, &key, &value)?,
            "max_notifications_per_event" => {
                max_notifications_per_event = field(name, &key, &value)?
            }
            "max_notification_bytes_per_event" => {
                max_notification_bytes_per_event = field(name, &key, &value)?;
            }
            "max_confirmed_in_flight_per_peer" => {
                max_confirmed_in_flight_per_peer = field(name, &key, &value)?;
            }
            _ => return Err(unexpected_key(name, &key)),
        }
    }
    let policy = CovPolicy {
        max_subscriptions_global,
        max_subscriptions_per_peer,
        reserved_capacity,
        reserved_peers,
        reserved_recipients,
        allow_indefinite_subscriptions,
        max_indefinite_per_peer,
        max_notifications_per_event,
        max_notification_bytes_per_event,
        max_confirmed_in_flight_per_peer,
    };
    policy
        .validate()
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(policy)
}

/// A reserved recipient in the shape `dcc_source_restriction` uses: `None`
/// for a directly attached peer, otherwise its routed source network.
fn recipient(network: Option<u16>, mac: &[u8]) -> CovRecipient {
    let mac_address = MacAddr::from_slice(mac);
    match network {
        None => CovRecipient::Direct(mac_address),
        Some(network) => CovRecipient::Routed(NpduAddress {
            network,
            mac_address,
        }),
    }
}

/// Extract one value of the policy the errors call `name`; see
/// [`super::policy_dict::field`].
fn field<'py, T: FromPyObjectOwned<'py>>(
    name: &str,
    key: &str,
    value: &Bound<'py, PyAny>,
) -> PyResult<T> {
    super::policy_dict::field(name, key, value)
}

#[cfg(test)]
#[path = "cov_policy_tests.rs"]
mod tests;

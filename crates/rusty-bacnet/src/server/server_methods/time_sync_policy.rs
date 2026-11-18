//! The constructor's keyword-only `time_sync_policy` dict, read into the Rust
//! `TimeSyncPolicy` the server starts with.

use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use bacnet_server::server::{
    TimeSyncPolicy, TimeSyncRateLimit, TimeSyncSource, TimeSyncSourceRestriction,
};

use super::policy_dict::{entries, unexpected_key};

const POLICY: &str = "time_sync_policy";

/// Read `time_sync_policy` into a validated `TimeSyncPolicy`.
///
/// Each key is a field under its Rust name, with `_ms` on the durations,
/// which are whole milliseconds. A key left out keeps the Rust default, so
/// `None` and `{}` both give `TimeSyncPolicy::default()`, which sets the clock
/// from every valid request. `source_restriction` takes the
/// `dcc_source_restriction` shape, and a rate is a `(max_per_second,
/// burst_capacity)` pair.
///
/// An unknown or non-str key and a value of the wrong type raise TypeError, a
/// negative or oversized integer OverflowError, and what
/// `TimeSyncSourceRestriction::new` or `TimeSyncPolicy::validate` refuses
/// ValueError: an entry outside 1..=18 address octets or networks 1..=65534,
/// more than 256 entries, a rate that is not positive and finite, a zero
/// burst, or `max_sources` outside 1..=65536.
///
/// Neither the destructuring of the defaults nor the literal that rebuilds
/// the policy uses `..`, so a field added to `TimeSyncPolicy` stops the
/// bindings compiling until it is bound here.
///
/// The endpoints read their `time_sync_policy` dict here too (#2617); their
/// `None` is time synchronization off, not this default.
pub(crate) fn time_sync_policy(dict: Option<&Bound<'_, PyDict>>) -> PyResult<TimeSyncPolicy> {
    let TimeSyncPolicy {
        mut enabled,
        mut source_restriction,
        mut max_step,
        mut per_source_rate,
        mut global_rate,
        mut coalesce_window,
        mut global_coalesce_window,
        mut max_sources,
    } = TimeSyncPolicy::default();
    let invalid = |error: bacnet_types::error::Error| PyValueError::new_err(error.to_string());
    for (key, value) in entries(POLICY, dict)? {
        match key.as_str() {
            "enabled" => enabled = field(&key, &value)?,
            "source_restriction" => {
                source_restriction = field::<Option<Vec<(Option<u16>, Vec<u8>)>>>(&key, &value)?
                    .map(|entries| {
                        TimeSyncSourceRestriction::new(entries.into_iter().map(source).collect())
                    })
                    .transpose()
                    .map_err(invalid)?;
            }
            "max_step_ms" => {
                max_step = field::<Option<u64>>(&key, &value)?.map(Duration::from_millis);
            }
            "per_source_rate" => per_source_rate = field::<Option<_>>(&key, &value)?.map(rate),
            "global_rate" => global_rate = field::<Option<_>>(&key, &value)?.map(rate),
            "coalesce_window_ms" => {
                coalesce_window = Duration::from_millis(field(&key, &value)?);
            }
            "global_coalesce_window_ms" => {
                global_coalesce_window = Duration::from_millis(field(&key, &value)?);
            }
            "max_sources" => max_sources = field(&key, &value)?,
            _ => return Err(unexpected_key(POLICY, &key)),
        }
    }
    let policy = TimeSyncPolicy {
        enabled,
        source_restriction,
        max_step,
        per_source_rate,
        global_rate,
        coalesce_window,
        global_coalesce_window,
        max_sources,
    };
    policy.validate().map_err(invalid)?;
    Ok(policy)
}

/// An allowlist entry in the shape `dcc_source_restriction` uses: `None` for
/// a directly attached source, otherwise its routed source network.
fn source((network, address): (Option<u16>, Vec<u8>)) -> TimeSyncSource {
    match network {
        None => TimeSyncSource::Direct(address),
        Some(network) => TimeSyncSource::Routed { network, address },
    }
}

/// A token bucket from its `(max_per_second, burst_capacity)` pair.
fn rate((max_per_second, burst_capacity): (f64, u32)) -> TimeSyncRateLimit {
    TimeSyncRateLimit {
        max_per_second,
        burst_capacity,
    }
}

/// Extract one `time_sync_policy` value; see [`super::policy_dict::field`].
fn field<'py, T: FromPyObjectOwned<'py>>(key: &str, value: &Bound<'py, PyAny>) -> PyResult<T> {
    super::policy_dict::field(POLICY, key, value)
}

#[cfg(test)]
#[path = "time_sync_policy_tests.rs"]
mod tests;

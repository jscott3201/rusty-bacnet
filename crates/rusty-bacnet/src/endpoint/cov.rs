//! The endpoint constructors' keyword-only `cov` dict (#2490): the
//! `ReportConfig` the session's COV reporting runs under. Its `policy` is
//! read as `BACnetServer`'s `cov_policy` is.

use bacnet_device_core::cov::FlightTiming;
use bacnet_device_core::report::{OutboxLimits, ReportConfig};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use bacnet_endpoint::session::{
    COV_CONFIRMED_TIMEOUT_MAX_MS, COV_OUTBOX_MAX_ENTRIES, COV_OUTBOX_MAX_OCTETS,
};

use crate::server::{cov_policy, policy_dict};

const COV: &str = "cov";

/// Read `cov` into the configuration the endpoint turns COV on with, or
/// `None` (COV off) for `None`.
///
/// Each key is optional and keeps the Rust default, so `{}` turns COV on
/// with the defaults: `policy` (a `cov_policy` dict), `outbox_entries`
/// (16), `outbox_octets` (2048), `confirmed_timeout_ms` (3000) and
/// `confirmed_retries` (3). An unknown or non-str key and a value of the
/// wrong type raise TypeError, an integer out of range OverflowError, and
/// a value the session would refuse at start ValueError, here rather than
/// at start: an outbox of no entry or no octet, or past the endpoint's
/// [`COV_OUTBOX_MAX_ENTRIES`] or [`COV_OUTBOX_MAX_OCTETS`], and a timeout
/// of zero or past [`COV_CONFIRMED_TIMEOUT_MAX_MS`].
pub(super) fn parse(dict: Option<&Bound<'_, PyDict>>) -> PyResult<Option<ReportConfig>> {
    let Some(dict) = dict else {
        return Ok(None);
    };
    let mut policy = None;
    let mut outbox = OutboxLimits::default();
    let mut timing = FlightTiming::default();
    for (key, value) in policy_dict::entries(COV, Some(dict))? {
        match key.as_str() {
            "policy" => {
                let inner: Bound<'_, PyDict> = policy_dict::field(COV, &key, &value)?;
                policy = Some(cov_policy::cov_policy_named("cov['policy']", Some(&inner))?);
            }
            "outbox_entries" => outbox.entries = policy_dict::field(COV, &key, &value)?,
            "outbox_octets" => outbox.octets = policy_dict::field(COV, &key, &value)?,
            "confirmed_timeout_ms" => {
                let ms: u64 = policy_dict::field(COV, &key, &value)?;
                if !(1..=COV_CONFIRMED_TIMEOUT_MAX_MS).contains(&ms) {
                    return Err(PyValueError::new_err(format!(
                        "cov['confirmed_timeout_ms'] must be 1 to {COV_CONFIRMED_TIMEOUT_MAX_MS}"
                    )));
                }
                timing.timeout = bacnet_time::Duration::from_millis(ms);
            }
            "confirmed_retries" => timing.retries = policy_dict::field(COV, &key, &value)?,
            _ => return Err(policy_dict::unexpected_key(COV, &key)),
        }
    }
    for (key, value, max) in [
        ("outbox_entries", outbox.entries, COV_OUTBOX_MAX_ENTRIES),
        ("outbox_octets", outbox.octets, COV_OUTBOX_MAX_OCTETS),
    ] {
        if !(1..=max).contains(&value) {
            return Err(PyValueError::new_err(format!(
                "cov['{key}'] must be 1 to {max}"
            )));
        }
    }
    let config = ReportConfig::new(policy.unwrap_or_default())
        .with_outbox(outbox)
        .with_confirmed(timing);
    Ok(Some(config))
}

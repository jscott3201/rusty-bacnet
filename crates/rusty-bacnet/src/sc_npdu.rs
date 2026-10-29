//! Python configuration and owned snapshots of the existing SC ingress policy.

use bacnet_transport::sc::{ScControlRateLimit, ScNpduAdmissionPolicy, ScNpduDropCounts};
use pyo3::{PyResult, exceptions::PyValueError};
use std::collections::HashMap;

/// The SC NPDU admission policy from the `sc_npdu_per_origin_limit` and
/// `sc_control_rate_limit` keywords, each accepted only for SC and checked
/// with the native rules before any credential lookup or I/O.
pub(crate) fn policy(
    transport: &str,
    limit: Option<usize>,
    control_rate_limit: Option<(u32, u32)>,
) -> PyResult<ScNpduAdmissionPolicy> {
    for (name, given) in [
        ("sc_npdu_per_origin_limit", limit.is_some()),
        ("sc_control_rate_limit", control_rate_limit.is_some()),
    ] {
        if given && transport != "sc" {
            return Err(PyValueError::new_err(format!(
                "{name} applies only to transport='sc'"
            )));
        }
    }
    let mut policy = ScNpduAdmissionPolicy::default();
    if let Some(limit) = limit {
        policy.per_origin_limit = limit;
    }
    if let Some((burst, per_second)) = control_rate_limit {
        policy.control_rate = ScControlRateLimit::new(burst, per_second);
    }
    policy
        .validate()
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(policy)
}

pub(crate) fn snapshot(counts: ScNpduDropCounts) -> HashMap<&'static str, u64> {
    HashMap::from([
        ("control_rate_drops", counts.control_rate_drops),
        ("fairness_drops", counts.fairness_drops),
        ("full_drops", counts.full_drops),
        ("closed_drops", counts.closed_drops),
    ])
}

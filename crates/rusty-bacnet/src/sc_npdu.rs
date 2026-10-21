//! Python configuration and owned snapshots of the existing SC ingress policy.

use bacnet_transport::sc::{ScNpduAdmissionPolicy, ScNpduDropCounts};
use pyo3::{exceptions::PyValueError, PyResult};
use std::collections::HashMap;

pub(crate) fn policy(transport: &str, limit: Option<usize>) -> PyResult<ScNpduAdmissionPolicy> {
    if limit.is_some() && transport != "sc" {
        return Err(PyValueError::new_err(
            "sc_npdu_per_origin_limit applies only to transport='sc'",
        ));
    }
    let mut policy = ScNpduAdmissionPolicy::default();
    if let Some(limit) = limit {
        policy.per_origin_limit = limit;
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

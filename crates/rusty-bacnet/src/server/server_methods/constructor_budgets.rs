//! Validation and budget assembly for the `BACnetServer` constructor: the
//! registered Network Port range, the mutation and DCC policies, and the
//! request-admission and per-service budgets. Each check raises ValueError.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::super::server;

/// Reject a registered Network Port instance outside 1..255, or one given for
/// a transport other than B/IP.
pub(super) fn registered_network_port(port: Option<u32>, transport: &str) -> PyResult<()> {
    if port.is_some_and(|instance| !(1..=255).contains(&instance)) {
        return Err(PyValueError::new_err(
            "registered_network_port must be 1..255",
        ));
    }
    if port.is_some() && transport != "bip" {
        return Err(PyValueError::new_err(
            "registered_network_port requires B/IP",
        ));
    }
    Ok(())
}

/// Parse the `mutation_policy` keyword.
pub(super) fn mutation_policy(name: &str) -> PyResult<bacnet_server::mutation::MutationPolicy> {
    match name {
        "permissive" => Ok(bacnet_server::mutation::MutationPolicy::Permissive),
        "deny_all" => Ok(bacnet_server::mutation::MutationPolicy::DenyAll),
        _ => Err(PyValueError::new_err(
            "mutation_policy must be 'permissive' or 'deny_all'",
        )),
    }
}

/// The DCC policy with its source restriction and disable rate limit.
pub(super) struct DccConfiguration {
    pub policy: server::DccPolicy,
    pub source_restriction: Option<server::DccSourceRestriction>,
    pub disable_rate_limit: Option<server::DccDisableRateLimit>,
}

/// Parse and validate the `dcc_*` keywords. The source restriction is
/// checked against the parsed policy, and the policy against the password.
pub(super) fn dcc_configuration(
    policy: &str,
    password: &Option<String>,
    source_restriction: Option<Vec<(Option<u16>, Vec<u8>)>>,
    disable_rate_limit: Option<(u32, u64)>,
) -> PyResult<DccConfiguration> {
    let policy = match policy {
        "deny_all" => server::DccPolicy::DenyAll,
        "require_password" => server::DccPolicy::RequirePassword,
        "legacy_permissive" => server::DccPolicy::LegacyPermissive,
        _ => {
            return Err(PyValueError::new_err(
                "dcc_policy must be 'deny_all', 'require_password', or 'legacy_permissive'",
            ))
        }
    };
    policy
        .validate(password)
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    let source_restriction = source_restriction
        .map(|entries| {
            let restriction = server::DccSourceRestriction::new(
                entries
                    .into_iter()
                    .map(|(network, address)| match network {
                        None => server::DccSource::Direct(address),
                        Some(network) => server::DccSource::Routed { network, address },
                    })
                    .collect(),
            )?;
            restriction.validate_policy(policy)?;
            Ok::<_, bacnet_types::error::Error>(restriction)
        })
        .transpose()
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    let disable_rate_limit = disable_rate_limit
        .map(|(capacity, refill_interval_ms)| {
            let limit = server::DccDisableRateLimit {
                capacity,
                refill_interval_ms,
            };
            limit.validate()?;
            Ok::<_, bacnet_types::error::Error>(limit)
        })
        .transpose()
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(DccConfiguration {
        policy,
        source_restriction,
        disable_rate_limit,
    })
}

/// The constructor's request-admission and per-service budget keywords, as
/// the Python caller gave them.
pub(super) struct BudgetKeywords {
    pub max_confirmed_in_flight: usize,
    pub max_unconfirmed_in_flight: usize,
    pub max_confirmed_in_flight_per_peer: usize,
    pub max_unconfirmed_in_flight_per_peer: usize,
    pub confirmed_recovery_reserve: usize,
    pub max_recovery_in_flight_per_peer: usize,
    pub rpm_max_result_elements: usize,
    pub rpm_max_service_ack_bytes: usize,
    pub alarm_summary_max_objects: usize,
    pub alarm_summary_max_service_ack_bytes: usize,
    pub enrollment_summary_max_objects: usize,
    pub enrollment_summary_max_service_ack_bytes: usize,
    pub atomic_read_file_max_requested_stream_octets: usize,
    pub atomic_read_file_max_requested_records: usize,
    pub atomic_read_file_max_service_ack_bytes: usize,
    pub atomic_write_file_max_stream_payload_octets: usize,
    pub atomic_write_file_max_records: usize,
    pub atomic_write_file_max_record_payload_bytes: usize,
    pub read_range_max_returned_items: usize,
    pub read_range_max_service_ack_bytes: usize,
    pub event_information_max_objects: usize,
    pub event_information_max_returned_summaries: usize,
    pub event_information_max_service_ack_bytes: usize,
}

/// The validated policy and budgets the server starts with.
pub(super) struct Budgets {
    pub request_admission_policy: server::RequestAdmissionPolicy,
    pub read_property_multiple_budget: server::ReadPropertyMultipleBudget,
    pub get_alarm_summary_budget: server::GetAlarmSummaryBudget,
    pub get_enrollment_summary_budget: server::GetEnrollmentSummaryBudget,
    pub atomic_read_file_budget: server::AtomicReadFileBudget,
    pub atomic_write_file_budget: server::AtomicWriteFileBudget,
    pub read_range_budget: server::ReadRangeBudget,
    pub get_event_information_budget: server::GetEventInformationBudget,
}

/// Build and validate the budgets. The first one the Rust validation refuses
/// raises ValueError with its message.
pub(super) fn budgets(keywords: BudgetKeywords) -> PyResult<Budgets> {
    let invalid = |error: bacnet_types::error::Error| PyValueError::new_err(error.to_string());
    let request_admission_policy = server::RequestAdmissionPolicy {
        max_confirmed_in_flight: keywords.max_confirmed_in_flight,
        max_unconfirmed_in_flight: keywords.max_unconfirmed_in_flight,
        max_confirmed_in_flight_per_peer: keywords.max_confirmed_in_flight_per_peer,
        max_unconfirmed_in_flight_per_peer: keywords.max_unconfirmed_in_flight_per_peer,
        confirmed_recovery_reserve: keywords.confirmed_recovery_reserve,
        max_recovery_in_flight_per_peer: keywords.max_recovery_in_flight_per_peer,
    };
    request_admission_policy.validate().map_err(invalid)?;
    let read_property_multiple_budget = server::ReadPropertyMultipleBudget {
        max_result_elements: keywords.rpm_max_result_elements,
        max_service_ack_bytes: keywords.rpm_max_service_ack_bytes,
    };
    read_property_multiple_budget.validate().map_err(invalid)?;
    let get_alarm_summary_budget = server::GetAlarmSummaryBudget {
        max_objects: keywords.alarm_summary_max_objects,
        max_service_ack_bytes: keywords.alarm_summary_max_service_ack_bytes,
    };
    get_alarm_summary_budget.validate().map_err(invalid)?;
    let get_enrollment_summary_budget = server::GetEnrollmentSummaryBudget {
        max_objects: keywords.enrollment_summary_max_objects,
        max_service_ack_bytes: keywords.enrollment_summary_max_service_ack_bytes,
    };
    get_enrollment_summary_budget.validate().map_err(invalid)?;
    let atomic_read_file_budget = server::AtomicReadFileBudget {
        max_requested_stream_octets: keywords.atomic_read_file_max_requested_stream_octets,
        max_requested_records: keywords.atomic_read_file_max_requested_records,
        max_service_ack_bytes: keywords.atomic_read_file_max_service_ack_bytes,
    };
    atomic_read_file_budget.validate().map_err(invalid)?;
    let read_range_budget = server::ReadRangeBudget {
        max_returned_items: keywords.read_range_max_returned_items,
        max_service_ack_bytes: keywords.read_range_max_service_ack_bytes,
    };
    let atomic_write_file_budget = server::AtomicWriteFileBudget {
        max_stream_payload_octets: keywords.atomic_write_file_max_stream_payload_octets,
        max_records: keywords.atomic_write_file_max_records,
        max_record_payload_bytes: keywords.atomic_write_file_max_record_payload_bytes,
    };
    atomic_write_file_budget.validate().map_err(invalid)?;
    read_range_budget.validate().map_err(invalid)?;
    let get_event_information_budget = server::GetEventInformationBudget {
        max_objects: keywords.event_information_max_objects,
        max_returned_summaries: keywords.event_information_max_returned_summaries,
        max_service_ack_bytes: keywords.event_information_max_service_ack_bytes,
    };
    get_event_information_budget.validate().map_err(invalid)?;
    Ok(Budgets {
        request_admission_policy,
        read_property_multiple_budget,
        get_alarm_summary_budget,
        get_enrollment_summary_budget,
        atomic_read_file_budget,
        atomic_write_file_budget,
        read_range_budget,
        get_event_information_budget,
    })
}

#[cfg(test)]
#[path = "constructor_budgets_tests.rs"]
mod tests;

/// Validate the full-server segmentation declaration and its receive deadline.
pub(super) fn segmentation(
    segmentation_supported: crate::types::PySegmentation,
    apdu_segment_timeout_ms: u64,
) -> PyResult<bacnet_types::enums::Segmentation> {
    let segmentation_supported = segmentation_supported.inner;
    if segmentation_supported.to_raw() > bacnet_types::enums::Segmentation::NONE.to_raw() {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid segmentation_supported",
        ));
    }
    if apdu_segment_timeout_ms == 0
        && segmentation_supported != bacnet_types::enums::Segmentation::NONE
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "apdu_segment_timeout_ms must be positive when segmentation is supported",
        ));
    }
    let representable = apdu_segment_timeout_ms
        .checked_mul(4)
        .and_then(|ms| std::time::Instant::now().checked_add(std::time::Duration::from_millis(ms)))
        .and_then(|deadline| deadline.checked_add(std::time::Duration::from_nanos(1)));
    if representable.is_none() {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "receive segment timeout is not representable",
        ));
    }
    Ok(segmentation_supported)
}

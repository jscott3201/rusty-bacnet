use super::*;

/// Validate a request password against the configured password.
///
/// Uses constant-time comparison to prevent timing side-channel attacks.
fn validate_password(
    configured: &Option<String>,
    request_pw: &Option<String>,
) -> Result<(), Error> {
    if let Some(ref expected) = configured {
        match request_pw {
            Some(ref pw) if constant_time_eq(pw.as_bytes(), expected.as_bytes()) => Ok(()),
            _ => Err(Error::Protocol {
                class: ErrorClass::SECURITY.to_raw() as u32,
                code: ErrorCode::PASSWORD_FAILURE.to_raw() as u32,
            }),
        }
    } else {
        Ok(())
    }
}

/// Constant-time byte-slice comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let len = a.len().max(b.len());
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..len {
        let x = if i < a.len() { a[i] } else { 0 };
        let y = if i < b.len() { b[i] } else { 0 };
        diff |= x ^ y;
    }
    diff == 0
}

pub(crate) struct DccFailure {
    pub error: Error,
    pub outcome: crate::server::dcc_outcomes::DccOutcome,
    pub metadata: crate::server::dcc_outcomes::DccMetadata,
}

/// Decode a DeviceCommunicationControl request and check its password, mode
/// and the local policy, returning the state to commit and the duration in
/// minutes. Nothing is stored here: `dcc_timer::replace` commits the result.
pub(crate) fn validate_dcc(
    service_data: &[u8],
    dcc_password: &Option<String>,
    policy: crate::server::DccPolicy,
) -> Result<(crate::server::DccState, Option<u16>), DccFailure> {
    use crate::server::dcc_outcomes::{DccMetadata, DccOutcome};
    let request =
        DeviceCommunicationControlRequest::decode(service_data).map_err(|error| DccFailure {
            error: error.into_request_reject(),
            outcome: DccOutcome::Malformed,
            metadata: DccMetadata::default(),
        })?;
    let metadata = DccMetadata {
        mode: Some(request.enable_disable.to_raw()),
        duration: request.time_duration,
    };
    let failure = |error, outcome| DccFailure {
        error,
        outcome,
        metadata,
    };
    validate_password(dcc_password, &request.password)
        .map_err(|e| failure(e, DccOutcome::PasswordFailure))?;
    let state = if request.enable_disable == EnableDisable::ENABLE {
        crate::server::DccState::Enable
    } else if request.enable_disable == EnableDisable::DISABLE {
        // ASHRAE 135-2020 Clause 16.1: reject deprecated DISABLE after
        // password validation, without changing state or the caller's timer.
        // No DccPolicy admits it, so DccState has no DISABLE.
        return Err(failure(
            Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            },
            DccOutcome::DeprecatedDenied,
        ));
    } else if request.enable_disable == EnableDisable::DISABLE_INITIATION {
        crate::server::DccState::DisableInitiation
    } else {
        return Err(failure(
            Error::Encoding("unknown EnableDisable value".into()),
            DccOutcome::Malformed,
        ));
    };
    if policy == crate::server::DccPolicy::DenyAll {
        return Err(failure(
            Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            },
            DccOutcome::PolicyDenied,
        ));
    }
    Ok((state, request.time_duration))
}

/// Handle a ReinitializeDevice request: decode it, check the password, and return the
/// requested state. A state Clause 16.4 does not define is refused, after the password
/// check, with SERVICES / SERVICE_REQUEST_DENIED.
pub fn handle_reinitialize_device(
    service_data: &[u8],
    reinit_password: &Option<String>,
) -> Result<bacnet_types::enums::ReinitializedState, Error> {
    use bacnet_types::enums::ReinitializedState;

    let request =
        ReinitializeDeviceRequest::decode(service_data).map_err(Error::into_request_reject)?;
    validate_password(reinit_password, &request.password)?;
    // ACTIVATE_CHANGES (7) is the highest state Clause 16.4 defines.
    if request.reinitialized_state.to_raw() > ReinitializedState::ACTIVATE_CHANGES.to_raw() {
        return Err(Error::Protocol {
            class: ErrorClass::SERVICES.to_raw() as u32,
            code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
        });
    }
    Ok(request.reinitialized_state)
}

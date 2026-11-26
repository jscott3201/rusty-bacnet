use super::*;

/// Validate a request password against the configured password.
///
/// Uses constant-time comparison to prevent timing side-channel attacks.
fn validate_password(
    configured: &Option<String>,
    request_pw: &Option<String>,
) -> Result<(), Error> {
    if let Some(expected) = configured {
        match request_pw {
            Some(pw) if constant_time_eq(pw.as_bytes(), expected.as_bytes()) => Ok(()),
            _ => Err(Error::protocol(
                ErrorClass::SECURITY,
                ErrorCode::PASSWORD_FAILURE,
            )),
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

/// Where a decoded DeviceCommunicationControl request's enable-disable
/// member starts: after the optional time-duration `[0]`, header and
/// contents as the request carried them.
fn enable_disable_at(service_data: &[u8]) -> usize {
    match bacnet_encoding::tags::decode_tag(service_data, 0) {
        Ok((tag, contents)) if tag.is_context(0) => contents + tag.length as usize,
        _ => 0,
    }
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
            Error::protocol(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED),
            DccOutcome::DeprecatedDenied,
        ));
    } else if request.enable_disable == EnableDisable::DISABLE_INITIATION {
        crate::server::DccState::DisableInitiation
    } else {
        // A mode the service doesn't define: the peer's data, refused as
        // out of range (#1833). It goes out as the Error PDU any refusal of
        // a running service does, not a Reject.
        return Err(failure(
            bacnet_types::error::DecodeError::new(
                bacnet_types::error::DecodingKind::OutOfRange,
                enable_disable_at(service_data),
                "unknown EnableDisable value",
            )
            .into(),
            DccOutcome::Malformed,
        ));
    };
    if policy == crate::server::DccPolicy::DenyAll {
        return Err(failure(
            Error::protocol(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED),
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
        return Err(Error::protocol(
            ErrorClass::SERVICES,
            ErrorCode::SERVICE_REQUEST_DENIED,
        ));
    }
    Ok(request.reinitialized_state)
}

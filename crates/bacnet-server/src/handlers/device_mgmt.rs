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

/// Handle a DeviceCommunicationControl request.
///
/// Updates the communication state and returns the requested state plus
/// optional duration (minutes) for auto-revert.
pub fn handle_device_communication_control(
    service_data: &[u8],
    comm_state: &AtomicU8,
    dcc_password: &Option<String>,
) -> Result<(EnableDisable, Option<u16>), Error> {
    let request = DeviceCommunicationControlRequest::decode(service_data)?;
    validate_password(dcc_password, &request.password)?;
    let new_state = if request.enable_disable == EnableDisable::ENABLE {
        0u8
    } else if request.enable_disable == EnableDisable::DISABLE {
        1u8
    } else if request.enable_disable == EnableDisable::DISABLE_INITIATION {
        2u8
    } else {
        return Err(Error::Encoding("unknown EnableDisable value".into()));
    };
    comm_state.store(new_state, Ordering::Release);
    tracing::debug!(
        "DeviceCommunicationControl: state set to {:?} ({}), duration={:?} min",
        request.enable_disable,
        new_state,
        request.time_duration
    );
    Ok((request.enable_disable, request.time_duration))
}

/// Handle a ReinitializeDevice request.
///
/// Returns the requested state. The caller decides what action to take.
pub fn handle_reinitialize_device(
    service_data: &[u8],
    reinit_password: &Option<String>,
) -> Result<(), Error> {
    let request = ReinitializeDeviceRequest::decode(service_data)?;
    validate_password(reinit_password, &request.password)?;
    Ok(())
}

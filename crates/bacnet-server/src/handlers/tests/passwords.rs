use super::*;

// -----------------------------------------------------------------------
// Password validation tests (DCC: Clause 16.1)
// -----------------------------------------------------------------------

#[test]
fn dcc_disable_rejection_preserves_state_and_password_precedence() {
    for initial in [0, 1, 2] {
        for duration in [None, Some(0), Some(5)] {
            for (configured, supplied, class, code) in [
                (
                    None,
                    None,
                    ErrorClass::SERVICES,
                    ErrorCode::SERVICE_REQUEST_DENIED,
                ),
                (
                    None,
                    Some("anything"),
                    ErrorClass::SERVICES,
                    ErrorCode::SERVICE_REQUEST_DENIED,
                ),
                (
                    Some("secret"),
                    Some("secret"),
                    ErrorClass::SERVICES,
                    ErrorCode::SERVICE_REQUEST_DENIED,
                ),
                (
                    Some("secret"),
                    None,
                    ErrorClass::SECURITY,
                    ErrorCode::PASSWORD_FAILURE,
                ),
                (
                    Some("secret"),
                    Some("wrong"),
                    ErrorClass::SECURITY,
                    ErrorCode::PASSWORD_FAILURE,
                ),
            ] {
                let comm_state = AtomicU8::new(initial);
                let request = DeviceCommunicationControlRequest {
                    time_duration: duration,
                    enable_disable: EnableDisable::DISABLE,
                    password: supplied.map(str::to_owned),
                };
                let mut buf = BytesMut::new();
                request.encode(&mut buf).unwrap();
                let err = handle_device_communication_control(
                    &buf,
                    &comm_state,
                    &configured.map(str::to_owned),
                )
                .unwrap_err();
                assert!(
                    matches!(err, Error::Protocol { class: actual_class, code: actual_code }
                    if actual_class == class.to_raw() as u32 && actual_code == code.to_raw() as u32),
                    "unexpected error: {err:?}"
                );
                assert_eq!(comm_state.load(Ordering::Acquire), initial);
            }
        }
    }
}

#[test]
fn dcc_correct_password_accepted() {
    let comm_state = AtomicU8::new(0);
    let pw = Some("secret".to_string());

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: Some("secret".to_string()),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, _) = handle_device_communication_control(&buf, &comm_state, &pw).unwrap();
    assert_eq!(state, EnableDisable::DISABLE_INITIATION);
    assert_eq!(comm_state.load(Ordering::Acquire), 2);
}

#[test]
fn dcc_wrong_password_rejected() {
    let comm_state = AtomicU8::new(0);
    let pw = Some("secret".to_string());

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: Some("wrong".to_string()),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let err = handle_device_communication_control(&buf, &comm_state, &pw).unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
            assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
        }
        other => panic!("expected Protocol error, got: {other:?}"),
    }
    // State unchanged
    assert_eq!(comm_state.load(Ordering::Acquire), 0);
}

#[test]
fn dcc_missing_password_when_required() {
    let comm_state = AtomicU8::new(0);
    let pw = Some("secret".to_string());

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let err = handle_device_communication_control(&buf, &comm_state, &pw).unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
            assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
        }
        other => panic!("expected Protocol error, got: {other:?}"),
    }
    assert_eq!(comm_state.load(Ordering::Acquire), 0);
}

#[test]
fn dcc_no_password_configured_accepts_any() {
    let comm_state = AtomicU8::new(0);

    let request = bacnet_services::device_mgmt::DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::DISABLE_INITIATION,
        password: Some("anything".to_string()),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let (state, _) = handle_device_communication_control(&buf, &comm_state, &None).unwrap();
    assert_eq!(state, EnableDisable::DISABLE_INITIATION);
    assert_eq!(comm_state.load(Ordering::Acquire), 2);
}

#[test]
fn reinit_correct_password_accepted() {
    let pw = Some("reinit-pw".to_string());

    let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
        reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
        password: Some("reinit-pw".to_string()),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    handle_reinitialize_device(&buf, &pw).unwrap();
}

#[test]
fn reinit_wrong_password_rejected() {
    let pw = Some("reinit-pw".to_string());

    let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
        reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
        password: Some("wrong".to_string()),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let err = handle_reinitialize_device(&buf, &pw).unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
            assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
        }
        other => panic!("expected Protocol error, got: {other:?}"),
    }
}

#[test]
fn reinit_missing_password_when_required() {
    let pw = Some("reinit-pw".to_string());

    let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
        reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
        password: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    let err = handle_reinitialize_device(&buf, &pw).unwrap_err();
    match err {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::SECURITY.to_raw() as u32);
            assert_eq!(code, ErrorCode::PASSWORD_FAILURE.to_raw() as u32);
        }
        other => panic!("expected Protocol error, got: {other:?}"),
    }
}

#[test]
fn reinit_no_password_configured_accepts_any() {
    let request = bacnet_services::device_mgmt::ReinitializeDeviceRequest {
        reinitialized_state: bacnet_types::enums::ReinitializedState::WARMSTART,
        password: Some("anything".to_string()),
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();

    handle_reinitialize_device(&buf, &None).unwrap();
}

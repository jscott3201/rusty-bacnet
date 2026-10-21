use super::*;

// -----------------------------------------------------------------------
// Password validation tests (ReinitializeDevice: Clause 16.4)
// -----------------------------------------------------------------------

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
            assert_eq!(class, ErrorClass::SECURITY);
            assert_eq!(code, ErrorCode::PASSWORD_FAILURE);
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
            assert_eq!(class, ErrorClass::SECURITY);
            assert_eq!(code, ErrorCode::PASSWORD_FAILURE);
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

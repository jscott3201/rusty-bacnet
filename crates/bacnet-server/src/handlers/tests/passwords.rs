use super::*;
use bacnet_types::error::RemoteError;

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
        Error::Remote(RemoteError::Error { class, code }) => {
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
        Error::Remote(RemoteError::Error { class, code }) => {
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

// -----------------------------------------------------------------------
// An empty request password is no password (#2094)
// -----------------------------------------------------------------------

/// A ReinitializeDevice (warmstart) and a DCC (enable) request carrying an
/// empty password, which our encoders refuse to build.
fn empty_password_requests() -> ([u8; 4], [u8; 4]) {
    // [0] warmstart, then [1] an empty UTF-8 CharacterString.
    let reinit = [0x09, 0x01, 0x19, 0x00];
    // [1] enable, then [2] an empty UTF-8 CharacterString.
    let dcc = [0x19, 0x00, 0x29, 0x00];
    (reinit, dcc)
}

#[test]
fn an_empty_password_fails_where_one_is_configured() {
    let (reinit, dcc) = empty_password_requests();
    let configured = Some("pw".to_string());
    let password_failure = |error: &Error| {
        matches!(
            error,
            Error::Remote(RemoteError::Error { class, code })
                if *class == ErrorClass::SECURITY && *code == ErrorCode::PASSWORD_FAILURE
        )
    };
    let error = handle_reinitialize_device(&reinit, &configured).unwrap_err();
    assert!(password_failure(&error), "{error:?}");
    let Err(failure) = validate_dcc(
        &dcc,
        &configured,
        crate::server::DccPolicy::LegacyPermissive,
    ) else {
        panic!("an empty DCC password passed a configured one");
    };
    assert!(password_failure(&failure.error), "{:?}", failure.error);
    assert!(matches!(
        failure.outcome,
        crate::server::dcc_outcomes::DccOutcome::PasswordFailure
    ));
}

#[test]
fn an_empty_password_is_accepted_where_none_is_configured() {
    let (reinit, dcc) = empty_password_requests();
    assert_eq!(
        handle_reinitialize_device(&reinit, &None).unwrap(),
        bacnet_types::enums::ReinitializedState::WARMSTART
    );
    let Ok((state, duration)) =
        validate_dcc(&dcc, &None, crate::server::DccPolicy::LegacyPermissive)
    else {
        panic!("an empty DCC password was refused without a configured one");
    };
    assert_eq!(state, crate::server::DccState::Enable);
    assert_eq!(duration, None);
}

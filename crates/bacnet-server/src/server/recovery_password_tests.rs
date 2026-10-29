//! Recovery capacity and password authorization remain independent.
use super::*;

#[tokio::test]
async fn recovery_classification_is_not_password_authorization() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "recovery",
        ServerConfig {
            dcc_password: Some("required".into()),
            dcc_policy: DccPolicy::RequirePassword,
            ..Default::default()
        },
    )
    .await;
    server.comm_state.set_for_test(DccState::DisableInitiation);
    for (id, password) in [(1, None), (2, Some("wrong"))] {
        dispatch(&server, enable(id, password), source(id), None).await;
        observed(&mut started).await;
        assert_eq!(server.comm_state.get(), DccState::DisableInitiation);
        assert!(
            matches!(held_sends(&server).frames.lock().unwrap().last(), Some(Apdu::Error(e)) if e.error_code == ErrorCode::PASSWORD_FAILURE)
        );
    }
    assert_eq!(server.request_admission_counters().recovery_active, 2);
    dispatch(&server, enable(3, Some("required")), source(3), None).await;
    observed(&mut started).await;
    assert_eq!(server.comm_state.get(), DccState::Enable);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn recovery_noneligible_requests_cannot_borrow_and_password_capacity_precedes_handler() {
    let (mut server, _tx, mut started) = fixture_with_config(
        "recovery",
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 2,
                confirmed_recovery_reserve: 1,
                ..Default::default()
            },
            dcc_password: Some("required".into()),
            ..Default::default()
        },
    )
    .await;
    dispatch(&server, request(0), source(0), None).await;
    observed(&mut started).await;
    for (id, service, data) in [
        (
            1,
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            &[0x19, 1][..],
        ),
        (
            2,
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            &[0x19, 2][..],
        ),
        (
            3,
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            &[0x19][..],
        ),
        (
            4,
            ConfirmedServiceChoice::REINITIALIZE_DEVICE,
            &[0x09, 0][..],
        ),
    ] {
        let Apdu::ConfirmedRequest(mut req) = request(id) else {
            unreachable!()
        };
        req.service_choice = service;
        req.service_request = Bytes::copy_from_slice(data);
        dispatch(&server, Apdu::ConfirmedRequest(req), source(id), None).await;
        observed(&mut started).await;
        assert!(
            matches!(held_sends(&server).frames.lock().unwrap().last(), Some(Apdu::Abort(a)) if a.invoke_id == id)
        );
    }
    assert_eq!(
        server.request_admission_counters().recovery_admitted_total,
        0
    );
    dispatch(&server, enable(5, None), source(5), None).await;
    observed(&mut started).await;
    assert!(
        matches!(held_sends(&server).frames.lock().unwrap().last(), Some(Apdu::Error(e)) if e.error_code == ErrorCode::PASSWORD_FAILURE)
    );
    dispatch(&server, enable(6, Some("wrong")), source(6), None).await;
    observed(&mut started).await;
    assert!(
        matches!(held_sends(&server).frames.lock().unwrap().last(), Some(Apdu::Abort(a)) if a.invoke_id == 6)
    );
    assert_eq!(
        server
            .request_admission_counters()
            .recovery_overloaded_total,
        1
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn recovery_accepts_a_twenty_character_multi_octet_password() {
    // 20 characters, 80 UTF-8 octets: the limit counts characters (#2060).
    let password = "😀".repeat(20);
    let (mut server, _tx, mut started) = fixture_with_config(
        "recovery",
        ServerConfig {
            dcc_password: Some(password.clone()),
            dcc_policy: DccPolicy::RequirePassword,
            ..Default::default()
        },
    )
    .await;
    server.comm_state.set_for_test(DccState::DisableInitiation);
    // One character more is refused before the password check.
    let too_long = format!("{password}😀");
    dispatch(&server, enable(1, Some(&too_long)), source(1), None).await;
    observed(&mut started).await;
    assert_eq!(server.comm_state.get(), DccState::DisableInitiation);
    // Valid encoding, size out of the service's range: a Reject, as
    // ReinitializeDevice answers (#2085).
    assert!(matches!(
        held_sends(&server).frames.lock().unwrap().last(),
        Some(Apdu::Reject(r)) if r.invoke_id == 1
            && r.reject_reason == bacnet_types::enums::RejectReason::PARAMETER_OUT_OF_RANGE
    ));
    dispatch(&server, enable(2, Some(&password)), source(2), None).await;
    observed(&mut started).await;
    assert!(matches!(
        held_sends(&server).frames.lock().unwrap().last(),
        Some(Apdu::SimpleAck(a)) if a.invoke_id == 2
    ));
    assert_eq!(server.comm_state.get(), DccState::Enable);
    // The recovery preflight admitted it: it does not bound UTF-8 at 20 octets.
    assert_eq!(
        server.request_admission_counters().recovery_admitted_total,
        1
    );
    server.stop().await.unwrap();
}
